# RucksFS

A standalone, single-process POSIX file system built in Rust, backed by **RocksDB** for metadata and **RawDisk** for file data. RucksFS runs as a single binary — metadata and data engines are embedded in the same process, eliminating network overhead while maintaining a clean separation of concerns through trait-based abstractions.

---

## Highlights

- **Single binary, zero deployment complexity** — one executable, no daemons or external databases
- **Full POSIX semantics** — `mkdir`, `create`, `read`, `write`, `rename`, `unlink`, `rmdir`, `readdir`, `getattr`, `setattr`, `statfs`, `flush`, `fsync`
- **RocksDB-backed metadata** — LSM-tree optimized for write-heavy workloads with structured key encoding
- **Delta-based metadata updates** — append-only deltas with background compaction for high write throughput
- **FUSE mount** — mount as a real Linux filesystem, usable by any program
- **Interactive REPL** — built-in shell for exploring and manipulating the filesystem
- **Pluggable storage traits** — `MetadataStore`, `DataStore`, `DirectoryIndex`, `DeltaStore` abstractions enable alternative backends

---

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                     rucksfs (binary)                 │
│  CLI: auto-demo │ interactive REPL │ FUSE mount      │
├──────────────────────────────────────────────────────┤
│                   rucksfs-client                     │
│  ┌────────────────────────────────────────────────┐  │
│  │ EmbeddedClient (in-process, no network)        │  │
│  │ VfsCore (routes metadata ↔ data operations)    │  │
│  └────────────┬──────────────┬────────────────────┘  │
│        MetadataOps         DataOps                   │
├───────────────┬──────────────┬───────────────────────┤
│ rucksfs-server│              │ rucksfs-dataserver     │
│ MetadataServer│              │ DataServer             │
│ (namespace,   │              │ (file I/O,             │
│  attributes,  │              │  block allocation)     │
│  delta engine)│              │                        │
├───────────────┴──────────────┴───────────────────────┤
│                  rucksfs-storage                     │
│  ┌───────────────────┐  ┌────────────────────────┐   │
│  │ RocksMetadataStore│  │ RawDiskDataStore        │   │
│  │ RocksDirectoryIdx │  │ (pre-allocated file,    │   │
│  │ RocksDeltaStore   │  │  offset-based I/O)      │   │
│  └───────────────────┘  └────────────────────────┘   │
├──────────────────────────────────────────────────────┤
│                   rucksfs-core                       │
│  Traits: MetadataOps, DataOps, VfsOps                │
│  Types:  FileAttr, DirEntry, StatFs, FsError         │
└──────────────────────────────────────────────────────┘
```

### Data Flow

1. **Metadata path** — Client → `VfsCore` → `MetadataOps` → `MetadataServer` → RocksDB (`MetadataStore` + `DirectoryIndex` + `DeltaStore`)
2. **Data path** — Client → `VfsCore` → `DataOps` → `DataServer` → `RawDiskDataStore`
3. **Write flow** — client writes data to DataServer, then calls `MetadataServer::report_write()` to update file size / mtime
4. **Persistence** — all state lives in `~/.rucksfs/` (configurable via `--data-dir`):
   - `metadata.db/` — RocksDB database (inodes, directory entries, deltas)
   - `data.raw` — pre-allocated raw file for block-level data storage

---

## Crate Overview

| Crate | Lines | Description |
|---|---|---|
| **core** | ~165 | Shared types (`FileAttr`, `DirEntry`, `StatFs`, `FsError`) and trait definitions (`MetadataOps`, `DataOps`, `VfsOps`) |
| **storage** | ~2,200 | Storage abstractions and implementations: RocksDB-backed metadata/index/delta stores, RawDisk data store, structured key encoding, block allocator |
| **server** | ~2,600 | `MetadataServer` — namespace engine with LRU cache, delta-based updates, and background compaction |
| **dataserver** | ~160 | `DataServer` — file data I/O engine, implements `DataOps` |
| **client** | ~900 | `VfsCore` (routing), `EmbeddedClient` (in-process), FUSE adapter (`FuseClient`), `mount_fuse` |
| **rpc** | ~800 | gRPC layer (Protocol Buffers): reserved for future distributed mode |
| **demo** | ~1,900 | Single-binary entry point with three modes: auto-demo, interactive REPL, FUSE mount |

**Total: ~8,800 lines of Rust**

---

## Quick Start

```bash
# Clone and build
git clone https://github.com/csjgg/rucksfs.git
cd rucksfs

# Build
cargo build -p rucksfs

# Run the automatic demo (data stored at ~/.rucksfs by default)
cargo run -p rucksfs

# Interactive REPL
cargo run -p rucksfs -- --interactive

# Custom data directory
cargo run -p rucksfs -- --data-dir /tmp/my-rucksfs
```

### FUSE Mount (Linux Only)

```bash
# Install FUSE dev libraries
sudo apt-get install libfuse-dev fuse    # Debian/Ubuntu

# Mount
cargo run -p rucksfs -- --mount /mnt/rucksfs

# Use with standard tools
ls /mnt/rucksfs
echo "hello" > /mnt/rucksfs/test.txt
cat /mnt/rucksfs/test.txt

# Unmount
fusermount -u /mnt/rucksfs
```

---

## Running Tests

```bash
# All workspace tests
cargo test --workspace

# Stress tests (concurrent operations, race conditions)
cargo test -p rucksfs --test stress_test

# Server integration tests
cargo test -p rucksfs-server
```

---

## Key Design Decisions

| Decision | Rationale |
|---|---|
| **RocksDB for metadata** | LSM-tree write amplification is acceptable; sequential write throughput and WriteBatch atomicity outperform B-tree for metadata-heavy workloads (see TableFS, LocoFS) |
| **Structured key encoding** | `<parent_inode, child_name>` prefix layout enables efficient `readdir` via prefix scan; dictionary order matches numeric order |
| **Delta-based updates** | Append-only attribute deltas avoid read-modify-write; background compaction merges deltas into base inodes |
| **RawDisk data store** | Pre-allocated file with block allocator avoids host filesystem overhead for data I/O |
| **Trait-based separation** | `MetadataOps` / `DataOps` / `VfsOps` traits enable in-process embedding today and potential networked backends in the future |

---

## License

MIT
