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

### P0 — Design-vs-Code Divergence

| ID | Status | Task | Details | Affected Files |
|----|--------|------|---------|----------------|
| T-10 | ⬜ | RawDiskDataStore: `Mutex<File>` → `pread`/`pwrite` | Current code serializes all I/O through a global `Mutex<File>` + seek. Design §5.2 specifies `FileExt::read_at/write_at` which allow lock-free concurrent I/O. **Performance-critical.** | `storage/src/rawdisk.rs` |
| T-11 | ⬜ | FUSE create/mkdir: respect caller `uid`/`gid` and `umask` | Hardcoded `uid: 0, gid: 0`. The `mode` is used but `umask` is ignored. | `client/src/fuse.rs`, `server/src/lib.rs` |
| T-12 | ✅ | rmdir/rename: fix TOCTOU on empty-check | Added `AtomicWriteBatch::is_dir_empty()` using `txn.prefix_iterator_cf()` for transactional reads. Both `rmdir` and `rename` now check emptiness inside the PCC transaction. | `storage/src/lib.rs`, `storage/src/rocks.rs`, `server/src/lib.rs` |

### P1 — Unimplemented Features (from design.md)

| ID | Status | Task | Details | Affected Files |
|----|--------|------|---------|----------------|
| T-20 | ⬜ | POSIX permission model | Design §8.1: `check_permission(uid/gid/mode)` for every op. Currently all ops run as uid=0 with no checks. | `server/src/lib.rs`, `core/src/lib.rs` |
| T-21 | ⬜ | `open()` flags validation | Design §6.3.1: check O_RDONLY/O_WRONLY/O_RDWR vs file permissions. Currently `_flags` is ignored. | `server/src/lib.rs` |
| T-22 | ⬜ | Deferred unlink (open handle tracking) | Design §6.2.4: unlinked files with open handles defer deletion until last close. No handle tracking exists. | `server/src/lib.rs`, `client/src/fuse.rs` |
| T-23 | ⬜ | RocksDB per-CF tuning | Design §5.4: bloom filter, prefix extractor, compression per CF. Currently all default options. | `storage/src/rocks.rs` |
| T-24 | ⬜ | FUSE mount options | Design §10.3: `allow_other`, `default_permissions`, `noatime`, `max_read`/`max_write`. Only `FSName` + `AutoUnmount` set. | `client/src/fuse.rs` |
| T-25 | ⬜ | gRPC transport layer (Mode A) | Design §2.3/§2.5A: full gRPC client/server with protobuf + TLS + Bearer Token. `rpc` crate has no implementation. | `rpc/` |
| T-26 | ⬜ | RawDiskDataStore crash recovery | Design §9.3: recovery mechanisms for the data file. No crash recovery logic exists. | `storage/src/rawdisk.rs` |
