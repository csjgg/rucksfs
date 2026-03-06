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
| T-03 | ✅ | LRU cache get() is O(n) | Replaced hand-rolled HashMap+VecDeque with `lru` crate for O(1) get/put/evict. | `server/src/cache.rs`, `server/Cargo.toml` |

### P2 — Future Features

| ID | Status | Task | Details |
|----|--------|------|---------|
| T-04 | ⬜ | Chunk/Slice data model | File → 64MB Chunks. `open` returns full data map. `report_write` computes Chunk ranges. |
| T-05 | ⬜ | Deferred GC | `unlink` records PendingDelete. Background GcWorker cleans up Chunk metadata and data. |
| T-06 | ✅ | fsck / consistency checker | Scans RocksDB for orphan inodes, nlink mismatches, and next_inode counter inconsistencies. Available via `--fsck` CLI flag. | `server/src/fsck.rs`, `demo/src/main.rs` |

### P0 — Design-vs-Code Divergence

| ID | Status | Task | Details | Affected Files |
|----|--------|------|---------|----------------|
| T-10 | ✅ | RawDiskDataStore: `Mutex<File>` → `pread`/`pwrite` | Replaced `Mutex<File>` with plain `File` using `FileExt::read_at/write_at` (pread/pwrite). Added concurrent I/O test. Commit `3b9f866`. | `storage/src/rawdisk.rs` |
| T-11 | ✅ | FUSE create/mkdir: respect caller `uid`/`gid` and `umask` | Added `uid`/`gid` params to `MetadataOps`/`VfsOps` traits. FUSE layer extracts `req.uid()`/`req.gid()` and applies `umask`. Server `InodeValue` now uses caller-supplied values. | `core/src/lib.rs`, `client/src/fuse.rs`, `client/src/vfs_core.rs`, `client/src/embedded.rs`, `server/src/lib.rs` |
| T-12 | ✅ | rmdir/rename: fix TOCTOU on empty-check | Added `AtomicWriteBatch::is_dir_empty()` using `txn.prefix_iterator_cf()` for transactional reads. Both `rmdir` and `rename` now check emptiness inside the PCC transaction. | `storage/src/lib.rs`, `storage/src/rocks.rs`, `server/src/lib.rs` |

### P1 — Unimplemented Features (from design.md)

| ID | Status | Task | Details | Affected Files |
|----|--------|------|---------|----------------|
| T-20 | ✅ | POSIX permission model | Covered by FUSE `default_permissions` mount option (T-24). Kernel VFS performs permission checks before requests reach the daemon. Server-side `check_permission()` deferred to future gRPC mode. | `client/src/fuse.rs` |
| T-21 | ✅ | `open()` flags validation | Covered by FUSE `default_permissions` mount option (T-24). Kernel VFS checks open flags against inode permissions before forwarding to daemon. | `client/src/fuse.rs` |
| T-22 | ✅ | Deferred unlink (open handle tracking) | Added `release()` to MetadataOps/VfsOps. MetadataServer tracks open handles per inode. Unlink defers data deletion when handles > 0. Release triggers cleanup when last handle is closed. | `core/src/lib.rs`, `server/src/lib.rs`, `client/src/fuse.rs`, `client/src/vfs_core.rs`, `client/src/embedded.rs` |
| T-23 | ✅ | RocksDB per-CF tuning | Added per-CF bloom filters (BlockBasedOptions), prefix extractors (8-byte inode prefix for dir_entries/delta_entries), and LZ4 compression. | `storage/src/rocks.rs` |
| T-24 | ✅ | FUSE mount options | Added `DefaultPermissions` + `AllowOther` to `mount_fuse()`. Kernel now enforces POSIX permission checks. Requires `user_allow_other` in `/etc/fuse.conf`. | `client/src/fuse.rs` |
| T-25 | ⬜ | gRPC transport layer (Mode A) | Design §2.3/§2.5A: full gRPC client/server with protobuf + TLS + Bearer Token. `rpc` crate has no implementation. | `rpc/` |
| T-26 | ⬜ | RawDiskDataStore crash recovery | Design §9.3: recovery mechanisms for the data file. No crash recovery logic exists. | `storage/src/rawdisk.rs` |
| T-27 | 🔧 | Per-inode DataLocation + VfsCore multi-DataServer routing | InodeValue 增加 data_location 字段（持久化到 RocksDB），create 时写入，open 从 inode 读取返回。VfsCore 维护 address→DataOps 映射，read/write 根据 handle 选择对应 DataServer。为分布式多存储节点打基础。 | `storage/src/encoding.rs`, `core/src/lib.rs`, `server/src/lib.rs`, `client/src/vfs_core.rs`, `client/src/embedded.rs`, `demo/src/main.rs` |
| T-28 | ⬜ | Full distributed deployment with gRPC DataOps | 在 T-27（per-inode DataLocation）和 T-25（gRPC transport）基础上，实现完整的分布式部署：多客户端 FUSE 节点通过 gRPC 访问独立元数据服务器（RocksDB），根据 DataLocation 路由到不同存储节点的 DataServer。包含：gRPC DataOps client/server 实现、多节点部署脚本、DataServer 注册/心跳机制。 | `rpc/`, `demo/`, deployment scripts |

### P0 — Benchmark

| ID | Status | Task | Details | Affected Files |
|----|--------|------|---------|----------------|
| T-30 | ⬜ | Rust 性能压测工具 (rucksfs-bench) | 当前 shell 脚本 benchmark 瓶颈在 bash for+touch fork/exec（ext4 上也仅 ~1,500 ops/s），无法反映真实文件系统性能。需实现 Rust 原生压测工具，通过 FUSE 挂载点直接 syscall（open/mknod/stat/unlink），支持：(1) 可配置操作类型（create/stat/unlink/mkdir/readdir/mixed）；(2) 可配置并发度（线程数/async task 数）；(3) 可配置规模（文件数、目录数、目录深度）；(4) 精确计时（Instant 微秒级）+ 输出 CSV。与 mdtest（C 实现）对标，用于论文性能对比。POSIX 正确性测试保留现有 shell 脚本。 | 新建 `benchmark/bench-tool/`（独立 binary crate） |
