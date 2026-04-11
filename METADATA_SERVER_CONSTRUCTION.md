# RucksFS MetadataServer & Full Stack Construction Guide

## Executive Summary
This document provides exact constructor signatures, import paths, and line numbers for building RucksFS systems directly in code, bypassing FUSE.

---

## 1. MetadataServer Constructor Signatures

### File: `/data/workspace/rucksfs/server/src/lib.rs`

#### Constructor 1: `MetadataServer::new()` (RECOMMENDED - Default Config)
**Location:** Lines 100-135

```rust
pub fn new(
    metadata: Arc<M>,
    index: Arc<I>,
    delta_store: Arc<DS>,
    default_data_location: DataLocation,
    storage_bundle: Arc<dyn StorageBundle>,
) -> Self
```

**Generic Parameters:**
- `M: MetadataStore` - Metadata store implementation
- `I: DirectoryIndex` - Directory index implementation  
- `DS: DeltaStore` - Delta store implementation

**Parameters:**
- `metadata: Arc<M>` - Metadata store (e.g., `Arc<RocksMetadataStore>`)
- `index: Arc<I>` - Directory index (e.g., `Arc<RocksDirectoryIndex>`)
- `delta_store: Arc<DS>` - Delta store for incremental updates (e.g., `Arc<RocksDeltaStore>`)
- `default_data_location: DataLocation` - Default DataServer identifier for new files
- `storage_bundle: Arc<dyn StorageBundle>` - Atomic write batch provider (e.g., `Arc<RocksStorageBundle>`)

**Returns:** `MetadataServer<M, I, DS>` instance

**Side Effects:**
- Calls `InodeAllocator::load()` to recover allocator state
- Creates `InodeFoldedCache` with `DEFAULT_CACHE_CAPACITY` (10,000 inodes)
- Creates `DeltaCompactionWorker` with default `CompactionConfig`
- Calls `init_root()` to ensure root directory (inode 1) exists

---

#### Constructor 2: `MetadataServer::with_config()` (Custom Config)
**Location:** Lines 140-177

```rust
pub fn with_config(
    metadata: Arc<M>,
    index: Arc<I>,
    delta_store: Arc<DS>,
    default_data_location: DataLocation,
    cache_capacity: usize,
    compaction_config: CompactionConfig,
    storage_bundle: Arc<dyn StorageBundle>,
) -> Self
```

**Additional Parameters:**
- `cache_capacity: usize` - LRU cache size for folded inodes (default: 10,000)
- `compaction_config: CompactionConfig` - Background compaction tuning (from `crate::compaction`)

**Returns:** `MetadataServer<M, I, DS>` instance with custom cache and compaction config

---

## 2. Storage Backends Setup

### File: `/data/workspace/rucksfs/storage/src/rocks.rs`

#### RocksDB Database Opening
**Location:** Lines 92-108

```rust
pub fn open_rocks_db(path: impl AsRef<Path>) -> FsResult<Arc<TransactionDB>>
```

**Parameters:**
- `path: impl AsRef<Path>` - Path to RocksDB directory

**Returns:** `Arc<TransactionDB>` — shared handle wrapping a RocksDB with 4 column families:
- `CF_INODES` ("inodes") - Inode metadata and related data
- `CF_DIR_ENTRIES` ("dir_entries") - Directory entries
- `CF_SYSTEM` ("system") - System-level KV pairs (allocator state)
- `CF_DELTA_ENTRIES` ("delta_entries") - Append-only delta operations

**Error:** `FsError::Io` on RocksDB open failure

---

#### RocksMetadataStore Constructor
**Location:** Lines 125-127

```rust
pub fn new(db: Arc<TransactionDB>) -> Self
```

**Parameters:**
- `db: Arc<TransactionDB>` - Shared RocksDB handle from `open_rocks_db()`

**Returns:** `RocksMetadataStore` instance

**Implements:** `MetadataStore` trait

---

#### RocksDirectoryIndex Constructor
**Location:** Lines 210-212

```rust
pub fn new(db: Arc<TransactionDB>) -> Self
```

**Parameters:**
- `db: Arc<TransactionDB>` - Shared RocksDB handle from `open_rocks_db()`

**Returns:** `RocksDirectoryIndex` instance

**Implements:** `DirectoryIndex` trait

**Special:** `shares_batch_storage()` returns `true` (mutations in transactions are immediately visible)

---

#### RocksDeltaStore Constructor
**Location:** Lines 350-363

```rust
pub fn new(db: Arc<TransactionDB>) -> Self
```

**Parameters:**
- `db: Arc<TransactionDB>` - Shared RocksDB handle from `open_rocks_db()`

**Returns:** `RocksDeltaStore` instance

**Implements:** `DeltaStore` trait

**Side Effects:**
- Automatically calls `recover_seqs()` on startup to recover per-inode sequence counters from existing deltas
- Logs warning (non-fatal) if recovery fails

---

#### RocksStorageBundle Constructor
**Location:** Lines 547-549

```rust
pub fn new(db: Arc<TransactionDB>) -> Self
```

**Parameters:**
- `db: Arc<TransactionDB>` - Shared RocksDB handle from `open_rocks_db()`

**Returns:** `RocksStorageBundle` instance

**Implements:** `StorageBundle` trait for atomic cross-column-family writes

---

### File: `/data/workspace/rucksfs/storage/src/rawdisk.rs`

#### RawDiskDataStore Constructor
**Location:** Lines 36-54

```rust
pub fn open(path: &std::path::Path, max_file_size: u64) -> FsResult<Self>
```

**Parameters:**
- `path: &std::path::Path` - Path to raw disk backing file (created if missing)
- `max_file_size: u64` - Maximum bytes per inode (recommended: 67_108_864 = 64 MiB)

**Returns:** `RawDiskDataStore` instance

**Implements:** `DataStore` trait (async)

**Error:** `FsError::InvalidInput` if `max_file_size == 0`

**Semantics:**
- Each inode allocated fixed `[inode * max_file_size .. (inode+1) * max_file_size)` region
- Unwritten regions read as zeros (sparse semantics)
- Thread-safe via `pread`/`pwrite` atomicity (no mutex)

---

### File: `/data/workspace/rucksfs/dataserver/src/lib.rs`

#### DataServer Constructor
**Location:** Lines 21-23

```rust
pub fn new(store: D) -> Self
where
    D: DataStore
```

**Parameters:**
- `store: D` - Any `DataStore` implementation (e.g., `RawDiskDataStore`)

**Returns:** `DataServer<D>` instance

**Implements:** `DataOps` trait (async)

---

## 3. Complete Stack Setup Example

### File: `/data/workspace/rucksfs/demo/src/main.rs` (Lines 59-84)

```rust
fn build_client(data_dir: &Path, max_file_size: u64) -> EmbeddedClient {
    // 1. Open RocksDB
    let db_path = data_dir.join("metadata.db");
    let db = open_rocks_db(&db_path).expect("failed to open RocksDB");
    
    // 2. Create metadata store, index, and delta store (all share same DB)
    let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
    let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
    let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
    
    // 3. Open data store
    let data_path = data_dir.join("data.raw");
    let data_store = RawDiskDataStore::open(&data_path, max_file_size)
        .expect("failed to open RawDisk data store");
    
    // 4. Create DataServer
    let data_server: Arc<dyn DataOps> = Arc::new(DataServer::new(data_store));
    
    // 5. Create storage bundle for atomic writes
    let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));
    
    // 6. Create MetadataServer
    let metadata_server: Arc<dyn MetadataOps> = Arc::new(MetadataServer::new(
        metadata,
        index,
        delta_store,
        DataLocation {
            server_id: "default".to_string(),
        },
        storage_bundle,
    ));
    
    // 7. Create EmbeddedClient (in-process client)
    EmbeddedClient::new(metadata_server, data_server)
}
```

---

## 4. EmbeddedClient Setup

### File: `/data/workspace/rucksfs/client/src/embedded.rs`

#### EmbeddedClient Constructor
**Location:** Lines 24-28

```rust
pub fn new(metadata: Arc<dyn MetadataOps>, data: Arc<dyn DataOps>) -> Self
```

**Parameters:**
- `metadata: Arc<dyn MetadataOps>` - MetadataServer or similar
- `data: Arc<dyn DataOps>` - DataServer or similar

**Returns:** `EmbeddedClient` instance

**Implements:** `VfsOps` trait (full POSIX operations)

**Note:** Uses shared `VfsCore` routing logic without network overhead

---

## 5. Import Paths & Module Hierarchy

### Core Types
```rust
// Metadata operations
use rucksfs_core::{
    DataLocation, FileAttr, FsError, FsResult, Inode, MetadataOps, 
    OpenResponse, ReleaseResponse, RenameResponse, SetAttrRequest, 
    SetAttrResponse, StatFs, UnlinkResponse, DataOps, VfsOps, DirEntry
};

// Storage abstractions
use rucksfs_storage::{
    AtomicWriteBatch, BatchOp, DeltaStore, DirectoryIndex, 
    MetadataStore, StorageBundle
};

// RocksDB implementations
use rucksfs_storage::{
    open_rocks_db, RawDiskDataStore, RocksDeltaStore, 
    RocksDirectoryIndex, RocksMetadataStore, RocksStorageBundle
};

// Server
use rucksfs_server::MetadataServer;

// Data server
use rucksfs_dataserver::DataServer;

// Client
use rucksfs_client::EmbeddedClient;
```

### Compaction Configuration (Optional)
```rust
use rucksfs_server::compaction::CompactionConfig;
```

---

## 6. Key Type Definitions

### DataLocation
**File:** `rucksfs_core`

```rust
pub struct DataLocation {
    pub server_id: String,
}
```

Used to specify which DataServer handles file data (default: "default").

---

### Generic Constraints Summary

```rust
pub struct MetadataServer<M, I, DS>
where
    M: MetadataStore,
    I: DirectoryIndex,
    DS: DeltaStore,
```

**Trait Requirements:**

- **MetadataStore** (trait):
  - `fn get(&self, key: &[u8]) -> FsResult<Option<Vec<u8>>>`
  - `fn put(&self, key: &[u8], value: &[u8]) -> FsResult<()>`
  - `fn delete(&self, key: &[u8]) -> FsResult<()>`
  - `fn scan_prefix(&self, prefix: &[u8]) -> FsResult<Vec<(Vec<u8>, Vec<u8>)>>`

- **DirectoryIndex** (trait):
  - `fn resolve_path(&self, parent: Inode, name: &str) -> FsResult<Option<Inode>>`
  - `fn list_dir(&self, inode: Inode) -> FsResult<Vec<DirEntry>>`
  - `fn insert_child(&self, parent: Inode, name: &str, inode: Inode, attr: FileAttr) -> FsResult<()>`
  - `fn remove_child(&self, parent: Inode, name: &str) -> FsResult<()>`
  - `fn shares_batch_storage(&self) -> bool` (default: false)

- **DeltaStore** (trait):
  - `fn append_deltas(&self, inode: Inode, values: &[Vec<u8>]) -> FsResult<Vec<u64>>`
  - `fn scan_deltas(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>>`
  - `fn scan_delta_keys(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>>`
  - `fn clear_deltas(&self, inode: Inode) -> FsResult<()>`
  - `fn next_seq(&self, inode: Inode) -> u64`
  - `fn scan_deltas_with_keys(&self, inode: Inode) -> FsResult<Vec<(Vec<u8>, Vec<u8>)>>`

- **DataStore** (trait, async):
  - `async fn read_at(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>>`
  - `async fn write_at(&self, inode: Inode, offset: u64, data: &[u8]) -> FsResult<u32>`
  - `async fn truncate(&self, inode: Inode, size: u64) -> FsResult<()>`
  - `async fn flush(&self, inode: Inode) -> FsResult<()>`
  - `async fn delete(&self, inode: Inode) -> FsResult<()>`

- **StorageBundle** (trait):
  - `fn begin_write(&self) -> Box<dyn AtomicWriteBatch + '_>`

---

## 7. Configuration Constants

### File: `/data/workspace/rucksfs/server/src/lib.rs`

```rust
/// Default cache capacity for folded inode LRU
const DEFAULT_CACHE_CAPACITY: usize = 10_000;

/// Max transaction retries on conflict
const TXN_MAX_RETRIES: usize = 3;
```

### File: `/data/workspace/rucksfs/storage/src/rocks.rs`

```rust
/// RocksDB column families
const CF_INODES: &str = "inodes";
const CF_DIR_ENTRIES: &str = "dir_entries";
const CF_SYSTEM: &str = "system";
const CF_DELTA_ENTRIES: &str = "delta_entries";

/// Directory entry value size: u64 inode + u32 mode = 12 bytes
const DIR_VALUE_SIZE: usize = 12;
```

---

## 8. Practical Construction Checklist

### Step-by-step to build full stack:

1. **Create directory structure** ✓
   ```rust
   std::fs::create_dir_all(data_dir)?;
   ```

2. **Open RocksDB** ✓
   ```rust
   let db = open_rocks_db(&data_dir.join("metadata.db"))?;
   let db_arc = Arc::new(db);
   ```

3. **Create metadata store** ✓
   ```rust
   let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db_arc)));
   ```

4. **Create directory index** ✓
   ```rust
   let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db_arc)));
   ```

5. **Create delta store** ✓
   ```rust
   let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db_arc)));
   ```

6. **Open data store** ✓
   ```rust
   let data_store = RawDiskDataStore::open(
       &data_dir.join("data.raw"), 
       67_108_864  // 64 MiB per inode
   )?;
   ```

7. **Create data server** ✓
   ```rust
   let data_server: Arc<dyn DataOps> = Arc::new(DataServer::new(data_store));
   ```

8. **Create storage bundle** ✓
   ```rust
   let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db_arc)));
   ```

9. **Create metadata server** ✓
   ```rust
   let metadata_server: Arc<dyn MetadataOps> = Arc::new(
       MetadataServer::new(
           metadata,
           index,
           delta_store,
           DataLocation {
               server_id: "default".to_string(),
           },
           storage_bundle,
       )
   );
   ```

10. **Create embedded client** ✓
    ```rust
    let client = EmbeddedClient::new(metadata_server, data_server);
    ```

---

## 9. Error Handling

All constructors and methods return `FsResult<T>` which is an alias for `Result<T, FsError>`.

### FsError variants (from rucksfs_core):
- `NotFound` - Inode or entry not found
- `AlreadyExists` - Entry already exists
- `IsADirectory` - Operation requires file, got directory
- `NotADirectory` - Operation requires directory, got file
- `DirectoryNotEmpty` - Directory not empty
- `PermissionDenied` - Permission denied
- `InvalidInput(String)` - Invalid argument
- `Io(String)` - I/O error
- `TransactionConflict` - Retryable transaction conflict (internal use)

---

## 10. Demo Binary Reference

### Run demo with custom data directory:
```bash
cargo run --bin rucksfs -- --data-dir /tmp/rucksfs_demo
```

### Run interactive REPL:
```bash
cargo run --bin rucksfs -- --interactive --data-dir /tmp/rucksfs
```

### Mount as FUSE (Linux only):
```bash
cargo run --bin rucksfs -- --mount /tmp/mnt --data-dir /tmp/rucksfs
```

---

## 11. File Locations Summary

| Component | File | Key Lines |
|-----------|------|-----------|
| MetadataServer | `/server/src/lib.rs` | 66-177 (struct + constructors) |
| RocksMetadataStore | `/storage/src/rocks.rs` | 119-140 |
| RocksDirectoryIndex | `/storage/src/rocks.rs` | 201-319 |
| RocksDeltaStore | `/storage/src/rocks.rs` | 333-534 |
| RocksStorageBundle | `/storage/src/rocks.rs` | 541-708 |
| RawDiskDataStore | `/storage/src/rawdisk.rs` | 24-150 |
| DataServer | `/dataserver/src/lib.rs` | 15-24 |
| EmbeddedClient | `/client/src/embedded.rs` | 18-29 |
| open_rocks_db | `/storage/src/rocks.rs` | 92-108 |
| CompactionConfig | `/server/src/compaction.rs` | (optional) |

