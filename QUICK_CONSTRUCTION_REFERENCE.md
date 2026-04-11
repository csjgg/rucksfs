# RucksFS Direct Construction Quick Reference

## One-Liner Construction Pattern

```rust
// All 10 steps in order
std::fs::create_dir_all(&data_dir)?;
let db = rucksfs_storage::open_rocks_db(&data_dir.join("metadata.db"))?;
let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
let data_store = RawDiskDataStore::open(&data_dir.join("data.raw"), 67_108_864)?;
let data_server: Arc<dyn DataOps> = Arc::new(DataServer::new(data_store));
let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));
let metadata_server: Arc<dyn MetadataOps> = Arc::new(MetadataServer::new(
    metadata, index, delta_store,
    DataLocation { server_id: "default".to_string() },
    storage_bundle,
));
let client = EmbeddedClient::new(metadata_server, data_server);
```

## Constructor Signatures at a Glance

| What | Constructor | File | Lines |
|------|-------------|------|-------|
| **MetadataServer** | `::new(metadata, index, delta_store, data_location, bundle)` | server/lib.rs | 100-135 |
| **RocksMetadataStore** | `::new(db)` | storage/rocks.rs | 125-127 |
| **RocksDirectoryIndex** | `::new(db)` | storage/rocks.rs | 210-212 |
| **RocksDeltaStore** | `::new(db)` | storage/rocks.rs | 350-363 |
| **RocksStorageBundle** | `::new(db)` | storage/rocks.rs | 547-549 |
| **RawDiskDataStore** | `::open(path, max_size)` | storage/rawdisk.rs | 36-54 |
| **DataServer** | `::new(store)` | dataserver/lib.rs | 21-23 |
| **EmbeddedClient** | `::new(metadata, data)` | client/embedded.rs | 24-28 |
| **open_rocks_db** | `(path)` | storage/rocks.rs | 92-108 |

## Required Imports

```rust
use std::sync::Arc;
use rucksfs_core::{DataLocation, DataOps, MetadataOps};
use rucksfs_storage::{
    open_rocks_db, RawDiskDataStore, RocksDeltaStore,
    RocksDirectoryIndex, RocksMetadataStore, RocksStorageBundle,
};
use rucksfs_server::MetadataServer;
use rucksfs_dataserver::DataServer;
use rucksfs_client::EmbeddedClient;
```

## Parameter Details

### MetadataServer::new()
```
metadata:              Arc<M> where M: MetadataStore
index:                 Arc<I> where I: DirectoryIndex
delta_store:           Arc<DS> where DS: DeltaStore
default_data_location: DataLocation { server_id: String }
storage_bundle:        Arc<dyn StorageBundle>
```

### RawDiskDataStore::open()
```
path:           &Path (created if missing)
max_file_size:  u64 (recommended: 67_108_864 = 64 MiB)
```

### EmbeddedClient::new()
```
metadata: Arc<dyn MetadataOps>  (e.g., Arc<MetadataServer<...>>)
data:     Arc<dyn DataOps>      (e.g., Arc<DataServer<...>>)
```

## Critical Notes

1. **All three RocksDB stores share ONE database** — clone the `Arc<TransactionDB>` for each
2. **RocksStorageBundle MUST use same db** — enables atomic cross-store writes
3. **Arc wrappers required** — MetadataServer needs `Arc<>` for thread safety
4. **DataLocation.server_id** — identifies which DataServer handles file data
5. **max_file_size > 0** — RawDiskDataStore requires non-zero max_file_size
6. **Default config sufficient** — `MetadataServer::new()` creates sensible defaults (10k cache, default compaction)
7. **No network overhead** — EmbeddedClient uses direct trait objects for in-process access

## Commonly Used Constants

```rust
const DEFAULT_CACHE_CAPACITY: usize = 10_000;  // Folded inode LRU size
const MAX_FILE_SIZE: u64 = 67_108_864;         // 64 MiB per inode (RawDisk)
const TXN_MAX_RETRIES: usize = 3;              // Transaction conflict retries
```

## Error Handling

All constructors return `FsResult<T>` = `Result<T, FsError>`

```rust
match client_result {
    Ok(client) => { /* use client */ }
    Err(FsError::Io(msg)) => eprintln!("I/O error: {}", msg),
    Err(e) => eprintln!("Error: {}", e),
}
```

## File Layout on Disk

```
~/.rucksfs/
├── metadata.db/          # RocksDB directory (created by open_rocks_db)
│   ├── CURRENT
│   ├── MANIFEST-*
│   ├── *.sst
│   └── ...
└── data.raw              # Raw disk file (max_size × inode_count bytes)
```

## Memory Layout (RawDiskDataStore)

```
data.raw layout:
Inode 1: [0 .. max_file_size)
Inode 2: [max_file_size .. 2*max_file_size)
Inode 3: [2*max_file_size .. 3*max_file_size)
...
```

Unwritten regions read as zeros (sparse semantics).

## RocksDB Column Families

| CF Name | Purpose | Key Examples |
|---------|---------|--------------|
| `inodes` | Inode metadata + symlink targets + data locations | `I<inode>`, `S<inode>`, `L<inode>` |
| `dir_entries` | Directory entries (parent→child mapping) | `D<parent><name>` |
| `delta_entries` | Append-only inode attribute updates | `X<inode><seq>` |
| `system` | System-level state (allocator counter, etc) | Custom keys |

## Compaction

Background delta compaction runs automatically:
- Scans pending deltas for inodes
- Folds deltas into base inode value
- Deletes delta entries
- Configurable via `CompactionConfig` (see `server/compaction.rs`)

For custom config:
```rust
use rucksfs_server::compaction::CompactionConfig;
let config = CompactionConfig { /* ... */ };
let server = MetadataServer::with_config(
    metadata, index, delta_store, data_location,
    10_000,  // cache capacity
    config,  // custom compaction config
    storage_bundle,
);
```

## POSIX Operations Available

All via `VfsOps` trait on `EmbeddedClient`:

```
lookup, getattr, readdir, create, mkdir, unlink, rmdir, rename,
setattr, open, close, read, write, flush, fsync, link, symlink,
readlink, statfs, release
```

Example:
```rust
let attr = client.lookup(ROOT_INODE, "mydir").await?;
let entries = client.readdir(attr.inode).await?;
```

## Performance Tips

1. **Cache capacity** — Default 10k inodes. Increase for larger working sets.
2. **Compaction frequency** — Tune `CompactionConfig` for workload balance.
3. **max_file_size** — Smaller = more inodes; larger = bigger files per inode.
4. **Lock timeout** — RocksDB default 5s (see `RocksStorageBundle::begin_write`).
5. **Transaction retries** — Auto-retried on conflict (TXN_MAX_RETRIES = 3).

