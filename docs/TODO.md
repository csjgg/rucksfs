# RucksFS — TODO

> Structured task list for AI agents and developers.
> Priority: P0 (critical) > P1 (important) > P2 (nice-to-have).
> Status: ✅ done | ⬜ open | 🔧 in-progress.

---

## Context

RucksFS is a **single-binary FUSE filesystem** backed by RocksDB.
The demo binary is the final deliverable (not a simplified preview).
Memory backends are for **tests only**; production uses RocksDB + RawDisk (`--persist`).

---

## Open Tasks

### P1 — Correctness

| ID | Status | Task | Details | Affected Files |
|----|--------|------|---------|----------------|
| T-01 | ⬜ | MemoryWriteBatch::commit is not truly atomic | Each op acquires/releases RwLock separately; two batches may interleave. Readers can observe intermediate state. | `storage/src/memory.rs` |
| T-02 | ⬜ | RocksDB path: batch + insert_child double-write | create/mkdir batch already writes dir_entry, then `insert_child()` writes again. Idempotent but wastes I/O. Use `index.is_persistent()` guard. | `server/src/lib.rs` |

### P2 — Performance

| ID | Status | Task | Details | Affected Files |
|----|--------|------|---------|----------------|
| T-03 | ⬜ | LRU cache get() is O(n) | `inner.order.retain()` scans entire VecDeque on every get/put. Replace with `lru` crate or `linked-hash-map` for O(1). | `server/src/cache.rs` |

### P2 — Future Features

| ID | Status | Task | Details |
|----|--------|------|---------|
| T-04 | ⬜ | Chunk/Slice data model | File → 64MB Chunks. `open` returns full data map. `report_write` computes Chunk ranges. |
| T-05 | ⬜ | Deferred GC | `unlink` records PendingDelete. Background GcWorker cleans up Chunk metadata and data. |
| T-06 | ⬜ | fsck / consistency checker | Detect orphan inodes, verify nlink counts, fix next_inode counter. |
