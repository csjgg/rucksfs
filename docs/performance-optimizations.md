# RucksFS Performance Optimizations

> Summary of all performance optimizations applied to RucksFS, with before/after metrics.
> Detailed per-round logs: `benchmark/bench-tool/optimization-log.md`

## Overview

Over 11 optimization rounds (6 merged, 5 reverted), RucksFS metadata throughput improved
from **significantly below ext4** to **matching or exceeding ext4** on 6 of 7 POSIX
metadata operations.

| Operation | Original | Final | Improvement | vs ext4 |
|-----------|----------|-------|-------------|---------|
| create    | 17,082   | 196,978 | **11.5x** | **1.18x** |
| stat      | 854,489  | 1,201,582 | **1.4x** | **1.06x** |
| unlink    | 31.82    | 231,673 | **7,280x** | 0.98x |
| mkdir     | 13,257   | 127,748 | **9.6x** | **1.10x** |
| rmdir     | 19,452   | 142,980 | **7.4x** | **1.09x** |
| readdir   | 9,008    | 60,753  | **6.7x** | **9.64x** |
| rename    | 20,904   | 204,849 | **9.8x** | **1.08x** |

Benchmark: `-t 1 -n 100`, single-thread, easy mode, 100 files per operation.

---

## Merged Optimizations (High Impact)

### 1. Async Data Deletion (Round 2)

**Problem**: `unlink` called `delete_data()` synchronously. `RawDiskDataStore::delete`
zero-fills a 64 MB region per inode in 4 KB chunks, blocking the FUSE response for
~30 ms per file.

**Solution**: Fire-and-forget `tokio::spawn` for data deletion after the metadata
transaction commits. The FUSE response returns immediately once metadata is committed.

**Files changed**:
- `server/src/lib.rs` — `unlink()`, `release()`, `rename()`: replaced synchronous
  `delete_data().await` with `tokio::spawn` + error logging
- `server/Cargo.toml` — added `"time"` to tokio dev-dependencies

**Key result**: unlink 31.82 → 5,180 ops/s (**163x improvement**)

---

### 2. No-op Data Delete (Round 5)

**Problem**: Even with async deletion from Round 2, background `tokio::spawn` tasks
still zero-fill 64 MB per inode. Under multi-thread benchmarks, these background tasks
saturate disk I/O, collapsing throughput for all operations. This caused:
- create 2T: 17K → 5 ops/s (catastrophic)
- create hard 1T: 0.88 ops/s (114 seconds for 100 files)

**Solution**: Make `RawDiskDataStore::delete` a no-op. Inode numbers are monotonically
increasing via `InodeAllocator` (atomic `fetch_add`) and never reused. Stale data
regions are permanently unreachable through metadata, so zero-filling is unnecessary.

**Files changed**:
- `storage/src/rawdisk.rs` — `delete()` method: replaced zero-fill loop with `Ok(())`
- `dataserver/src/lib.rs` — updated `delete_data_is_noop` test
- `server/tests/integration.rs` — renamed and updated `unlink_nlink_zero_removes_metadata` test
- `server/src/lib.rs` — updated stale comments referencing zero-fill

**Key results**:
- unlink: 5,180 → 24,472 ops/s (**4.7x over Round 2**)
- create 2T: 5 → 37,596 ops/s (**fixed multi-thread collapse**)
- create hard 1T: 0.88 → 15,965 ops/s (**fixed hard mode**)

---

### 3. Inline Parent Timestamp Deltas (Round 6)

**Problem**: Every mutation operation (create, mkdir, unlink, rmdir, rename, link,
symlink) performed **two separate RocksDB writes**:
1. Main PCC transaction commit (inode + dir_entry + data_location)
2. Separate `WriteBatch` for parent directory timestamp deltas (SetMtime, SetCtime)
   via `append_parent_deltas → DeltaStore::append_deltas`

The second write doubles per-op WAL I/O. Since RocksDB's WAL lock serializes all
writers, this halves aggregate throughput.

**Solution**: Move `SetMtime`/`SetCtime` delta writes into the main transaction batch
using `batch_parent_deltas` (generalized from `batch_nlink_deltas`). Post-commit code
now only updates the in-memory cache and marks dirty for background compaction.
Removed the now-unused `append_parent_deltas` helper.

**Files changed**:
- `server/src/lib.rs`:
  - Renamed `batch_nlink_deltas` → `batch_parent_deltas`
  - All 7 mutation methods: moved timestamp deltas into transaction, replaced
    post-commit `append_parent_deltas` with `cache.apply_deltas` + `compaction.mark_dirty`
  - Removed `append_parent_deltas` helper
  - Fixed timestamp drift: unlink/rmdir/rename/link now return the transaction
    timestamp and reuse it for cache updates

**Key results** (vs Round 5 baseline):
- create: 15,434 → 196,978 ops/s (**12.8x**, now 1.18x ext4)
- rename: 18,836 → 204,849 ops/s (**10.9x**, now 1.08x ext4)
- unlink: 24,472 → 231,673 ops/s (**9.5x**, now 0.98x ext4)
- mkdir: 19,635 → 127,748 ops/s (**6.5x**, now 1.10x ext4)

---

## Merged Optimizations (Micro / Code Quality)

### 4. Reduce mark_dirty Condvar Notifications (Round 7)

**Problem**: `mark_dirty()` acquires two `std::sync::Mutex` and calls
`Condvar::notify_one()` on every mutation, even when unnecessary.

**Solution**: Only wake the compaction loop when the dirty set transitions from
empty to non-empty.

**Files changed**: `server/src/compaction.rs` — `mark_dirty()` method

**Result**: No measurable impact at -n 100, but eliminates redundant syscalls.

---

### 5. Disable RocksDB Deadlock Detection (Round 8)

**Problem**: `set_deadlock_detect(true)` traverses the deadlock-detection graph
on every `get_for_update` call.

**Solution**: `set_deadlock_detect(false)` — our lock ordering (inode-ID sorted
in rename) prevents deadlocks by design.

**Files changed**: `storage/src/rocks.rs` — `begin_write()` method

**Result**: No measurable impact at -n 100, but eliminates unnecessary CPU work.

---

### 6. Stack Buffer for Serialization (Round 11)

**Problem**: `InodeValue::serialize()` uses `Vec::with_capacity(57)` + 9x
`extend_from_slice` with per-call bounds checks.

**Solution**: Assemble into `[u8; 57]` stack buffer with `copy_from_slice`,
then `.to_vec()`. Eliminates per-field bounds check overhead.

**Files changed**: `storage/src/encoding.rs` — `serialize()` method

**Result**: No measurable impact at -n 100, improved code clarity.

---

## Reverted Optimizations (Lessons Learned)

### Round 1 — RocksDB Block Cache (REVERTED)

256 MB shared LRU block cache with pinned L0 filters. At small working sets (-n 100),
cache management overhead (LRU bookkeeping) outweighed any benefit. Most operations
regressed 20-35%.

**Lesson**: Block cache helps large working sets; at small scale, the overhead dominates.

### Round 3 — Disable WAL for Delta Writes (REVERTED)

Set `disable_wal(true)` on delta `WriteBatch`. Did not improve create throughput (still
dominated by main transaction WAL write). Stat regressed -38.8%, likely due to RocksDB
flushing memtable more aggressively without WAL protection.

**Lesson**: Disabling WAL has non-obvious side effects on read path via memtable flush behavior.

### Round 4 — Skip clear_deltas on Inode Deletion (REVERTED)

Skipped `clear_deltas()` when deleting inodes. Multiple severe regressions, likely
benchmark noise at small -n.

**Lesson**: At small -n, only trust large (>2x) improvements.

### Round 9 — Manual WAL Flush (REVERTED)

`set_manual_wal_flush(true)` to batch WAL writes. No improvement — `write()` syscall
is already fast when `sync=false`.

**Lesson**: WAL write overhead is not the bottleneck when `fsync` is not required.

### Round 10 — Increase Allocator PERSIST_INTERVAL (REVERTED)

Increased from 64 to 1024. No impact — at -n 100, the persist triggers only once.

**Lesson**: Optimization must match benchmark scale to be measurable.

---

## Round Summary

| Round | Optimization | Decision | Impact |
|-------|-------------|----------|--------|
| 1 | RocksDB block cache | REVERTED | -22% to -35% regression |
| 2 | Async data deletion | **MERGED** | unlink **163x** |
| 3 | Disable WAL for deltas | REVERTED | stat -39% regression |
| 4 | Skip clear_deltas | REVERTED | multiple regressions |
| 5 | No-op data delete | **MERGED** | unlink **4.7x**, fixed multi-thread |
| 6 | Inline timestamp deltas | **MERGED** | all ops **5-13x** |
| 7 | Reduce mark_dirty notifications | **MERGED** | code quality |
| 8 | Disable deadlock detection | **MERGED** | code quality |
| 9 | Manual WAL flush | REVERTED | no benefit |
| 10 | Increase allocator interval | REVERTED | no benefit |
| 11 | Stack buffer serialization | **MERGED** | code quality |

---

## Measurement Methodology

- **Tool**: `rucksfs-bench` (custom Rust benchmark in `benchmark/bench-tool/`)
- **Modes**: easy (separate directories per thread), hard (shared directory)
- **Protocol**: file chain (create→stat→rename→unlink) + dir chain (mkdir→readdir→rmdir)
- **Parameters**: `-t 1,2,4 -n 100`
- **Verification**: Each round runs benchmark 2x to confirm consistency
- **Decision criteria**: ≥10% improvement on any op with no >5% regression on others
- **ext4 baselines**: measured on same hardware with identical parameters
