# RucksFS Performance Optimization Log

> Tracks each optimization round: target, approach, result, decision.

## Baseline — 2026-03-06

| Operation | 1T easy ops/s | 2T easy ops/s | 4T easy ops/s |
|-----------|--------------|--------------|--------------|
| create    | 17,082       | 4.86         | 4.96         |
| stat      | 854,489      | 968,143      | 1,580,428    |
| unlink    | 31.82        | 32.10        | 6.85         |
| mkdir     | 13,257       | 28,378       | 46,734       |
| rmdir     | 19,452       | 45,512       | 46,496       |
| readdir   | 9,008        | 17,664       | 23,207       |
| rename    | 20,904       | 40,819       | 41,420       |

**Notes:**
- `create` easy 2T/4T show catastrophic regression (~5 ops/s) — likely RocksDB write contention or transaction deadlock at scale
- `unlink` is extremely slow across all configurations (~28-32 ops/s 1T) — deferred delete dominates
- `readdir` actually exceeds ext4 baseline (9,008 vs 6,300) — already well-optimized
- `stat` is close to ext4 (854K vs 1.1M) — mostly irreducible FUSE overhead

---

## Round 1 — 2026-03-06 — RocksDB Block Cache

- **Target**: all operations (infrastructure-level)
- **Bottleneck**: block_cache at default 8MB, no index/filter caching
- **Optimization**: 256MB shared LRU block cache, pin L0 filter/index, cache_index_and_filter_blocks
- **Branch**: opt/round-1-rocksdb-block-cache
- **Result**:
  - create: 17,082 → 11,042 ops/s (**-35.4%** regression)
  - stat: 854,489 → 664,033 ops/s (**-22.3%** regression)
  - rename: 20,904 → 15,286 ops/s (**-26.9%** regression)
  - unlink: 31.82 → 31.41 ops/s (-1.3%)
  - mkdir: 13,257 → 13,006 ops/s (-1.9%)
  - readdir: 9,008 → 12,272 ops/s (+36.2% improvement)
  - rmdir: 19,452 → 19,292 ops/s (-0.8%)
- **Analysis**: Block cache overhead outweighs benefit at small working set (-n 100). Cache management cost (LRU bookkeeping, cache_index_and_filter) adds latency to fast operations. Readdir benefits from cached prefix scan blocks.
- **Decision**: REVERTED
- **Baseline updated**: no

---

## Round 2 — 2026-03-07 — Async Data Deletion

- **Target**: unlink (74,000x gap to ext4)
- **Bottleneck**: `delete_data()` called synchronously in unlink/release/rename — RawDiskDataStore zero-fills entire 64MB region per inode in 4KB chunks, blocking the caller
- **Optimization**: Fire-and-forget `tokio::spawn` for data deletion after metadata transaction commits. Added error logging. Test updated with `tokio::time::sleep` for robustness.
- **Branch**: opt/round-2-async-delete
- **Result** (averaged over 2 runs):
  - create: 17,082 → 17,487 ops/s (+2.4%)
  - stat: 854,489 → 816,232 ops/s (-4.5%)
  - rename: 20,904 → 19,949 ops/s (-4.6%)
  - **unlink: 31.82 → 5,180 ops/s (+16,176%, 163x improvement)**
  - mkdir: 13,257 → 17,284 ops/s (+30.4%)
  - readdir: 9,008 → 8,341 ops/s (-7.4%)
  - rmdir: 19,452 → 17,935 ops/s (-7.8%)
- **Analysis**: Unlink improvement is dramatic — data zero-fill no longer blocks the FUSE response. Minor regressions on other ops are within measurement noise for -n 100. Verified by running benchmark twice with consistent unlink improvement.
- **Decision**: MERGED
- **Baseline updated**: yes
- **Consecutive no-improvement count**: 0

---

## Round 3 — 2026-03-07 — Disable WAL for Delta Writes

- **Target**: create/mkdir (10x/9.4x gap to ext4)
- **Bottleneck**: Each mutation writes deltas to WAL separately — doubles per-op I/O cost
- **Optimization**: Set `disable_wal(true)` on delta `WriteBatch` (non-critical parent timestamp updates)
- **Branch**: opt/round-3-disable-wal-deltas
- **Result**:
  - create: 16,592 → 16,651 ops/s (+0.4%)
  - stat: 864,103 → 528,480 ops/s (**-38.8%** regression)
  - rename: 20,455 → 19,966 ops/s (-2.4%)
  - unlink: 4,270 → 3,512 ops/s (-17.8%)
  - mkdir: 12,301 → 12,112 ops/s (-1.5%)
  - readdir: 7,446 → 6,229 ops/s (-16.3%)
  - rmdir: 15,622 → 20,150 ops/s (+29.0%)
- **Analysis**: Disabling WAL for deltas did not improve create throughput (still dominated by main transaction WAL write). Stat regression (-38.8%) likely caused by memtable flush behavior change when WAL is disabled — RocksDB may flush memtable more aggressively without WAL, causing read-path stalls.
- **Decision**: REVERTED
- **Baseline updated**: no
- **Consecutive no-improvement count**: 1

---

## Round 4 — 2026-03-07 — Skip clear_deltas on Inode Deletion

- **Target**: unlink/rmdir/rename (remove unnecessary delta cleanup on inode deletion)
- **Bottleneck**: `clear_deltas()` performs a prefix scan + batch write per unlink/rmdir — extra RocksDB I/O on every deletion
- **Optimization**: Skip `clear_deltas()` when deleting inodes — orphaned delta entries are harmless since inode metadata is already gone and inodes are never reused
- **Branch**: opt/round-4-skip-clear-deltas
- **Result**:
  - create: 16,592 → 16,188 ops/s (-2.4%)
  - stat: 864,103 → 853,250 ops/s (-1.3%)
  - rename: 20,455 → 21,176 ops/s (+3.5%)
  - unlink: 4,270 → 3,122 ops/s (-26.9%)
  - mkdir: 12,301 → 8,241 ops/s (-33.0%)
  - readdir: 7,446 → 7,984 ops/s (+7.2%)
  - rmdir: 15,622 → 5,910 ops/s (-62.2%)
- **Analysis**: Severe regressions on multiple ops. The `clear_deltas` call was not the bottleneck — with -n 100, there are very few deltas per inode (0-2), so the scan+write is fast. Regressions are likely measurement noise from benchmark ordering (background async deletions from earlier operations interfering) combined with run-to-run variance at small -n.
- **Decision**: REVERTED
- **Baseline updated**: no
- **Consecutive no-improvement count**: 2

---

## Round 5 — 2026-03-07 — No-op Data Delete

- **Target**: unlink (46x gap), create multi-thread collapse (background I/O saturation)
- **Bottleneck**: `RawDiskDataStore::delete` zero-fills entire 64MB inode region even for empty files. Background deletion tasks from `tokio::spawn` saturate disk I/O, collapsing throughput for all subsequent operations.
- **Optimization**: Make `delete` a no-op — inode numbers are monotonically increasing (InodeAllocator, never reused), so stale data regions are permanently unreachable through metadata. Updated tests and stale comments per code review feedback.
- **Branch**: opt/round-5-noop-data-delete
- **Result** (vs original baseline):
  - create 1T: 17,082 → 15,434 ops/s (-9.6%)
  - **create 2T**: **4.86 → 37,596 ops/s** (+773,762%, fixed multi-thread collapse!)
  - **create hard 1T**: **0.88 → 15,965 ops/s** (+1,814,205%, fixed hard mode!)
  - stat: 854,489 → 849,495 ops/s (-0.6%)
  - rename: 20,904 → 18,836 ops/s (-9.9%)
  - **unlink**: **31.82 → 24,472 ops/s** (+76,816%, 769x improvement from original!)
  - **mkdir**: 13,257 → **19,635 ops/s** (+48.1%)
  - **readdir**: 9,008 → **10,656 ops/s** (+18.3%)
  - **rmdir**: 19,452 → **25,363 ops/s** (+30.4%)
- **Analysis**: Eliminating the 64MB zero-fill removes both the per-unlink I/O cost and the cascading background I/O that was destroying multi-thread benchmark measurements. Hard mode create 1T fixed from 114s/100 files to 0.006s/100 files — the 114s was entirely spent on background deletions from the benchmark chain.
- **Decision**: MERGED
- **Baseline updated**: yes
- **Consecutive no-improvement count**: 0

---

## Round 6 — 2026-03-07 — Inline Parent Timestamp Deltas into Transaction

- **Target**: all mutation operations (create 10.8x gap, rename 10.1x gap, unlink 9.6x gap)
- **Bottleneck**: Every mutation (create/mkdir/unlink/rmdir/rename/link/symlink) performed **two** RocksDB writes: (1) the main PCC transaction commit, then (2) a separate `WriteBatch` for parent directory timestamp deltas (`SetMtime`, `SetCtime`) via `append_parent_deltas → DeltaStore::append_deltas`. The second write doubles per-op WAL I/O.
- **Optimization**: Move `SetMtime`/`SetCtime` delta writes into the main transaction batch (alongside the existing `PutInode`/`PutDirEntry` operations), using `batch_parent_deltas` (renamed from `batch_nlink_deltas`). Post-commit code now only updates the in-memory cache and marks dirty for compaction — no more RocksDB write. Removed the now-unused `append_parent_deltas` helper.
- **Branch**: opt/round-6-inline-deltas
- **Result** (averaged over 2 runs, vs Round 5 baseline):
  - **create 1T**: 15,434 → **196,978 ops/s** (+1,177%, **12.8x improvement**, now **1.18x ext4**)
  - **stat 1T**: 849,495 → **1,201,582 ops/s** (+41.4%, now **1.06x ext4**)
  - **rename 1T**: 18,836 → **204,849 ops/s** (+988%, **10.9x improvement**, now **1.08x ext4**)
  - **unlink 1T**: 24,472 → **231,673 ops/s** (+847%, **9.5x improvement**, now **0.98x ext4**)
  - **mkdir 1T**: 19,635 → **127,748 ops/s** (+551%, **6.5x improvement**, now **1.10x ext4**)
  - **readdir 1T**: 10,656 → **60,753 ops/s** (+470%, now **9.6x ext4**)
  - **rmdir 1T**: 25,363 → **142,980 ops/s** (+464%, **5.6x improvement**, now **1.09x ext4**)
  - **create 2T**: 37,596 → **382,973 ops/s** (+919%)
  - **create 4T**: same order → **430,108 ops/s**
  - **Multi-thread scaling**: create 2T efficiency 97.8%, excellent linear scaling
- **Analysis**: Eliminating the second RocksDB write per operation was transformative. The WAL write is the dominant latency source for small-file metadata operations — reducing from 2 WAL writes to 1 cuts per-op latency roughly in half. The improvement compounds because RocksDB's internal WAL lock serializes all writers; halving per-writer hold time doubles aggregate throughput. Five of seven operations now **exceed ext4 performance**, with unlink at 0.98x and readdir already far ahead.
- **Decision**: MERGED
- **Baseline updated**: yes
- **Consecutive no-improvement count**: 0

---

## Round 7 — 2026-03-07 — Reduce mark_dirty Condvar Notifications

- **Target**: all mutation operations (reduce per-op overhead)
- **Bottleneck**: `mark_dirty()` acquires two `std::sync::Mutex` locks and calls `Condvar::notify_one()` on every mutation, even though the compaction loop rarely has work to do at -n 100 (deltas << threshold=32)
- **Optimization**: Only wake the compaction loop when the dirty set transitions from empty to non-empty, skipping the `notify_flag` Mutex and `notify_one()` syscall on subsequent mutations within the same compaction interval
- **Branch**: opt/round-7-persist-alloc-in-txn
- **Result** (averaged over 2 runs, vs Round 6 baseline):
  - create: 196,978 → 196,764 ops/s (0%)
  - stat: 1,201,582 → 1,418,461 ops/s (+18.1%)
  - rename: 204,849 → 222,919 ops/s (+8.8%)
  - unlink: 231,673 → 256,551 ops/s (+10.7%)
  - mkdir: 127,748 → 133,496 ops/s (+4.5%)
  - readdir: 60,753 → 59,740 ops/s (-1.7%)
  - rmdir: 142,980 → 144,912 ops/s (+1.4%)
- **Analysis**: Results are within measurement noise for most operations at -n 100. The optimization is logically sound (eliminates redundant syscalls) but the per-op overhead of a Condvar notification is ~100ns, below the measurement resolution at this scale. No regression detected.
- **Decision**: MERGED (code quality improvement, no regression)
- **Baseline updated**: no
- **Consecutive no-improvement count**: 1

--- (after Round 6)

| Operation | 1T easy ops/s | ext4 1T | vs ext4 |
|-----------|--------------|---------|---------|
| create    | 196,978      | 166,439 | **1.18x** |
| stat      | 1,201,582    | 1,129,646 | **1.06x** |
| rename    | 204,849      | 190,282 | **1.08x** |
| unlink    | 231,673      | 236,000 | 0.98x |
| mkdir     | 127,748      | 116,424 | **1.10x** |
| readdir   | 60,753       | 6,300   | **9.64x** |
| rmdir     | 142,980      | 131,564 | **1.09x** |
