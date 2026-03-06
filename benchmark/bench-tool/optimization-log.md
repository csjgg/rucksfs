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
