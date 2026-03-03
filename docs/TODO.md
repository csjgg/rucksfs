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
| T-02 | ✅ | RocksDB path: batch + insert_child double-write | Added `DirectoryIndex::shares_batch_storage()` trait method. When `true` (RocksDB backend), post-commit `insert_child`/`remove_child` calls are skipped since the batch already wrote to the same CF. Guards added in create/mkdir/unlink/rmdir/rename. | `storage/src/lib.rs`, `storage/src/rocks.rs`, `server/src/lib.rs` |

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
| T-11 | ✅ | FUSE create/mkdir: respect caller `uid`/`gid` and `umask` | Added `uid`/`gid` params to `MetadataOps`/`VfsOps` traits. FUSE layer extracts `req.uid()`/`req.gid()` and applies `umask`. Server `InodeValue` now uses caller-supplied values. | `core/src/lib.rs`, `client/src/fuse.rs`, `client/src/vfs_core.rs`, `client/src/embedded.rs`, `server/src/lib.rs` |
| T-12 | ✅ | rmdir/rename: fix TOCTOU on empty-check | Added `AtomicWriteBatch::is_dir_empty()` using `txn.prefix_iterator_cf()` for transactional reads. Both `rmdir` and `rename` now check emptiness inside the PCC transaction. | `storage/src/lib.rs`, `storage/src/rocks.rs`, `server/src/lib.rs` |

### P1 — Unimplemented Features (from design.md)

| ID | Status | Task | Details | Affected Files |
|----|--------|------|---------|----------------|
| T-20 | ✅ | POSIX permission model | Covered by FUSE `default_permissions` mount option (T-24). Kernel VFS performs permission checks before requests reach the daemon. Server-side `check_permission()` deferred to future gRPC mode. | `client/src/fuse.rs` |
| T-21 | ✅ | `open()` flags validation | Covered by FUSE `default_permissions` mount option (T-24). Kernel VFS checks open flags against inode permissions before forwarding to daemon. | `client/src/fuse.rs` |
| T-22 | ⬜ | Deferred unlink (open handle tracking) | Design §6.2.4: unlinked files with open handles defer deletion until last close. No handle tracking exists. | `server/src/lib.rs`, `client/src/fuse.rs` |
| T-23 | ⬜ | RocksDB per-CF tuning | Design §5.4: bloom filter, prefix extractor, compression per CF. Currently all default options. | `storage/src/rocks.rs` |
| T-24 | ✅ | FUSE mount options | Added `DefaultPermissions` + `AllowOther` to `mount_fuse()`. Kernel now enforces POSIX permission checks. Requires `user_allow_other` in `/etc/fuse.conf`. | `client/src/fuse.rs` |
| T-25 | ⬜ | gRPC transport layer (Mode A) | Design §2.3/§2.5A: full gRPC client/server with protobuf + TLS + Bearer Token. `rpc` crate has no implementation. | `rpc/` |
| T-26 | ⬜ | RawDiskDataStore crash recovery | Design §9.3: recovery mechanisms for the data file. No crash recovery logic exists. | `storage/src/rawdisk.rs` |
