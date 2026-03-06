# Per-Inode DataLocation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Store per-inode data location in RocksDB and enable VfsCore to route read/write to the correct DataServer based on the address returned by `open()`.

**Architecture:** Independent `[b'L'][inode]` key-prefix mapping table in the inodes CF. MetadataServer writes it on create/symlink, reads on open, deletes on unlink/rename. VfsCore maintains `address → Arc<dyn DataOps>` map and routes I/O accordingly.

**Tech Stack:** Rust, RocksDB, async-trait, tokio

**Design doc:** `docs/plans/2026-03-06-per-inode-data-location-design.md`

---

### Task 1: Add data_location key encoding helpers

**Files:**
- Modify: `storage/src/encoding.rs`

**Step 1: Add encoding functions after the delta key section (~line 221)**

```rust
// ---------------------------------------------------------------------------
// Data location key encoding helpers
// ---------------------------------------------------------------------------

/// Prefix byte for data location keys.
const DATA_LOCATION_KEY_PREFIX: u8 = b'L';

/// Encode a data location key: `[b'L'][inode: u64 BE]`.
pub fn encode_data_location_key(inode: Inode) -> Vec<u8> {
    let mut key = Vec::with_capacity(9);
    key.push(DATA_LOCATION_KEY_PREFIX);
    key.extend_from_slice(&inode.to_be_bytes());
    key
}
```

**Step 2: Add unit test in the existing `tests` module**

```rust
// -- data location key tests -----------------------------------------------

#[test]
fn data_location_key_encoding() {
    let key = encode_data_location_key(42);
    assert_eq!(key.len(), 9);
    assert_eq!(key[0], b'L');
    let inode = u64::from_be_bytes(key[1..9].try_into().unwrap());
    assert_eq!(inode, 42);
}

#[test]
fn data_location_key_no_collision() {
    let loc_key = encode_data_location_key(1);
    let inode_key = encode_inode_key(1);
    let dir_key = encode_dir_entry_key(1, "x");
    let delta_key = encode_delta_key(1, 0);
    // All prefixes are distinct
    assert_ne!(loc_key[0], inode_key[0]);
    assert_ne!(loc_key[0], dir_key[0]);
    assert_ne!(loc_key[0], delta_key[0]);
}
```

**Step 3: Run tests**

Run: `cargo test -p rucksfs-storage -- encoding`
Expected: All encoding tests pass including the two new ones.

**Step 4: Commit**

```
feat(storage): add data_location key encoding helpers
```

---

### Task 2: Add BatchOp variants for data location

**Files:**
- Modify: `storage/src/lib.rs` (~line 87, `BatchOp` enum)
- Modify: `storage/src/rocks.rs` (~line 558, `RocksWriteBatch::push`)

**Step 1: Add two new BatchOp variants in `storage/src/lib.rs`**

After `PutSystem` variant:

```rust
/// Put a data location entry: CF:inodes (key prefix 'L')
PutDataLocation { key: Vec<u8>, value: Vec<u8> },
/// Delete a data location entry: CF:inodes (key prefix 'L')
DeleteDataLocation { key: Vec<u8> },
```

**Step 2: Handle new variants in `RocksWriteBatch::push` in `storage/src/rocks.rs`**

Add after the `BatchOp::PutSystem` arm (before the closing `}`):

```rust
BatchOp::PutDataLocation { key, value } => {
    let cf = self.db.cf_handle(CF_INODES)
        .expect("CF 'inodes' must exist — database is corrupt or misconfigured");
    self.txn.put_cf(&cf, &key, &value)
        .expect("transaction put_cf(inodes/data_location) failed unexpectedly");
}
BatchOp::DeleteDataLocation { key } => {
    let cf = self.db.cf_handle(CF_INODES)
        .expect("CF 'inodes' must exist — database is corrupt or misconfigured");
    self.txn.delete_cf(&cf, &key)
        .expect("transaction delete_cf(inodes/data_location) failed unexpectedly");
}
```

**Step 3: Run tests**

Run: `cargo test -p rucksfs-storage`
Expected: All existing tests pass (no behavioral change).

**Step 4: Commit**

```
feat(storage): add PutDataLocation/DeleteDataLocation batch operations
```

---

### Task 3: MetadataServer — rename field, write on create/symlink

**Files:**
- Modify: `server/src/lib.rs`

**Step 1: Rename `data_location` → `default_data_location`**

In `MetadataServer` struct definition (~line 68), `new()` (~line 92), `with_config()` (~line 134), and `open()` (~line 954). This is a rename of the field and constructor parameter. Four locations in total.

**Step 2: Add batch helper for data location**

Add after `batch_delete_dir_entry` (~line 293):

```rust
/// Add a "put data location" operation to the batch.
fn batch_put_data_location(
    batch: &mut dyn AtomicWriteBatch,
    inode: Inode,
    address: &str,
) {
    let key = encode_data_location_key(inode);
    batch.push(BatchOp::PutDataLocation {
        key,
        value: address.as_bytes().to_vec(),
    });
}

/// Add a "delete data location" operation to the batch.
fn batch_delete_data_location(
    batch: &mut dyn AtomicWriteBatch,
    inode: Inode,
) {
    batch.push(BatchOp::DeleteDataLocation {
        key: encode_data_location_key(inode),
    });
}
```

**Step 3: Add `encode_data_location_key` to imports**

In the `use rucksfs_storage::encoding` import (~line 21), add `encode_data_location_key`.

**Step 4: Write data_location in `create` transaction**

In `create()` (~line 527, after `batch_put_dir_entry`), add:

```rust
Self::batch_put_data_location(
    batch.as_mut(),
    new_inode,
    &self.default_data_location.address,
);
```

**Step 5: Write data_location in `symlink` transaction**

In `symlink()` (~line 1106, after `batch_put_dir_entry`), add the same call.

**Step 6: Run tests**

Run: `cargo test -p rucksfs-server`
Expected: All existing server tests pass.

**Step 7: Commit**

```
feat(server): write per-inode data_location on create/symlink
```

---

### Task 4: MetadataServer — read on open, delete on unlink/rename

**Files:**
- Modify: `server/src/lib.rs`

**Step 1: Change `open()` to read data_location from store**

Replace the current `open()` implementation (~line 954-968) with:

```rust
async fn open(&self, inode: Inode, _flags: u32) -> FsResult<OpenResponse> {
    let iv = self.load_inode(inode)?;
    if Self::is_dir(iv.mode) {
        return Err(FsError::IsADirectory);
    }
    // Increment open handle count.
    {
        let mut handles = self.open_handles.lock().expect("open_handles poisoned");
        *handles.entry(inode).or_insert(0) += 1;
    }
    // Read per-inode data location; fall back to default if not found.
    let loc_key = encode_data_location_key(inode);
    let address = match self.metadata.get(&loc_key)? {
        Some(bytes) => String::from_utf8(bytes)
            .unwrap_or_else(|_| self.default_data_location.address.clone()),
        None => self.default_data_location.address.clone(),
    };
    Ok(OpenResponse {
        handle: inode,
        data_location: DataLocation { address },
    })
}
```

**Step 2: Delete data_location in `unlink` when nlink → 0**

In `unlink()`, inside the `if child_iv.nlink == 0` branch (~line 648-649), after `batch_delete_inode`, add:

```rust
Self::batch_delete_data_location(batch.as_mut(), child_inode);
```

**Step 3: Delete data_location in `rename` when overwriting a file (nlink → 0)**

In `rename()`, inside the `if let Some((dst_inode, _)) = dst_inode_opt` branch (~line 853-856), after `batch_delete_inode`, add:

```rust
Self::batch_delete_data_location(batch.as_mut(), dst_inode);
```

**Step 4: Run tests**

Run: `cargo test -p rucksfs-server`
Expected: All tests pass.

**Step 5: Commit**

```
feat(server): read data_location on open, delete on unlink/rename
```

---

### Task 5: Add server-level tests for data_location lifecycle

**Files:**
- Modify: `server/src/lib.rs` (tests module at the bottom, or `demo/tests/integration_test.rs`)

**Step 1: Add test — create then open returns correct DataLocation**

In `demo/tests/integration_test.rs`, add a new test. This requires accessing MetadataOps directly (not via EmbeddedClient which hides OpenResponse). Add a helper that returns both the MetadataServer and DataServer separately:

```rust
fn new_metadata_and_data() -> (
    tempfile::TempDir,
    Arc<dyn MetadataOps>,
    Arc<dyn DataOps>,
) {
    let tmp = tempfile::tempdir().unwrap();
    let db = open_rocks_db(tmp.path().join("meta.db")).unwrap();
    let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
    let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
    let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
    let data_store = RawDiskDataStore::open(
        &tmp.path().join("data.raw"),
        64 * 1024 * 1024,
    ).unwrap();

    let data_server: Arc<dyn DataOps> = Arc::new(DataServer::new(data_store));
    let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));
    let metadata_server: Arc<dyn MetadataOps> = Arc::new(MetadataServer::new(
        metadata,
        index,
        delta_store,
        Arc::clone(&data_server),
        DataLocation {
            address: "test-data-server:9001".to_string(),
        },
        storage_bundle,
    ));

    (tmp, metadata_server, data_server)
}

#[tokio::test]
async fn open_returns_per_inode_data_location() {
    let (_tmp, meta, _data) = new_metadata_and_data();
    let file = meta.create(ROOT, "testfile", 0o644, 0, 0).await.unwrap();
    let resp = meta.open(file.inode, 0).await.unwrap();
    assert_eq!(resp.data_location.address, "test-data-server:9001");
}
```

**Step 2: Add test — unlink clears data_location, open falls back to default**

```rust
#[tokio::test]
async fn unlink_clears_data_location() {
    let (_tmp, meta, _data) = new_metadata_and_data();

    // Create and then hard-link so unlink won't delete the inode
    // (we need the inode to survive to test open fallback).
    let file = meta.create(ROOT, "f1", 0o644, 0, 0).await.unwrap();
    meta.link(ROOT, "f1_link", file.inode).await.unwrap();

    // Unlink the original — nlink goes from 2 to 1, data_location NOT deleted.
    meta.unlink(ROOT, "f1").await.unwrap();
    let resp = meta.open(file.inode, 0).await.unwrap();
    assert_eq!(resp.data_location.address, "test-data-server:9001");

    // Unlink the link — nlink goes from 1 to 0, data_location IS deleted.
    // But inode is also deleted, so open should fail.
    meta.unlink(ROOT, "f1_link").await.unwrap();
    let err = meta.open(file.inode, 0).await;
    assert!(err.is_err());
}
```

**Step 3: Run tests**

Run: `cargo test -p rucksfs --test integration_test`
Expected: New tests pass. Existing tests also pass.

**Step 4: Commit**

```
test(server): add data_location lifecycle tests for open/unlink
```

---

### Task 6: VfsCore — multi-DataServer routing

**Files:**
- Modify: `client/src/vfs_core.rs`
- Modify: `client/src/embedded.rs`

**Step 1: Restructure VfsCore fields**

Replace the current struct and `new()` in `client/src/vfs_core.rs`:

```rust
pub struct VfsCore {
    metadata: Arc<dyn MetadataOps>,
    default_data: Arc<dyn DataOps>,
    /// Registry of DataServer addresses to their DataOps implementations.
    /// Used for routing read/write to the correct DataServer.
    data_servers: Mutex<HashMap<String, Arc<dyn DataOps>>>,
    /// Maps open file handles (inode) to their DataServer address.
    handle_map: Mutex<HashMap<u64, String>>,
}

impl VfsCore {
    pub fn new(metadata: Arc<dyn MetadataOps>, data: Arc<dyn DataOps>) -> Self {
        Self {
            metadata,
            default_data: data,
            data_servers: Mutex::new(HashMap::new()),
            handle_map: Mutex::new(HashMap::new()),
        }
    }

    /// Create a VfsCore with additional DataServer registrations.
    pub fn with_data_servers(
        metadata: Arc<dyn MetadataOps>,
        default_data: Arc<dyn DataOps>,
        data_servers: HashMap<String, Arc<dyn DataOps>>,
    ) -> Self {
        Self {
            metadata,
            default_data,
            data_servers: Mutex::new(data_servers),
            handle_map: Mutex::new(HashMap::new()),
        }
    }

    /// Look up the DataOps for a given inode based on its open handle mapping.
    /// Falls back to default_data if the inode has no mapping or the address
    /// is not in data_servers.
    fn resolve_data(&self, inode: u64) -> Arc<dyn DataOps> {
        let handle_map = self.handle_map.lock().expect("handle_map poisoned");
        if let Some(address) = handle_map.get(&inode) {
            let servers = self.data_servers.lock().expect("data_servers poisoned");
            if let Some(ds) = servers.get(address) {
                return Arc::clone(ds);
            }
        }
        Arc::clone(&self.default_data)
    }
}
```

**Step 2: Update VfsOps implementation**

Change `open()`:
```rust
async fn open(&self, inode: Inode, flags: u32) -> FsResult<u64> {
    let resp = self.metadata.open(inode, flags).await?;
    {
        let mut map = self.handle_map.lock().expect("handle_map poisoned");
        map.insert(resp.handle, resp.data_location.address);
    }
    Ok(resp.handle)
}
```

Change `read()`:
```rust
async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
    self.resolve_data(inode).read_data(inode, offset, size).await
}
```

Change `write()`:
```rust
async fn write(&self, inode: Inode, offset: u64, data: &[u8], _flags: u32) -> FsResult<u32> {
    let ds = self.resolve_data(inode);
    let written = ds.write_data(inode, offset, data).await?;
    let new_end = offset + written as u64;
    let ts = now_secs();
    self.metadata.report_write(inode, new_end, ts).await?;
    Ok(written)
}
```

Change `flush()`:
```rust
async fn flush(&self, inode: Inode) -> FsResult<()> {
    self.resolve_data(inode).flush(inode).await
}
```

Change `fsync()`:
```rust
async fn fsync(&self, inode: Inode, _datasync: bool) -> FsResult<()> {
    self.resolve_data(inode).flush(inode).await
}
```

Change `release()`:
```rust
async fn release(&self, inode: Inode) -> FsResult<()> {
    self.metadata.release(inode).await?;
    let mut map = self.handle_map.lock().expect("handle_map poisoned");
    map.remove(&inode);
    Ok(())
}
```

All other methods (lookup, getattr, readdir, create, mkdir, unlink, rmdir, rename, setattr, statfs, link, symlink, readlink) remain unchanged — they delegate to `self.metadata`.

**Step 3: Run tests**

Run: `cargo test --workspace`
Expected: All tests pass. EmbeddedClient uses `VfsCore::new()` which sets empty data_servers, so behavior is identical.

**Step 4: Commit**

```
feat(client): implement multi-DataServer routing in VfsCore
```

---

### Task 7: Update demo binary for renamed field

**Files:**
- Modify: `demo/src/main.rs` (~line 78)
- Modify: `demo/tests/integration_test.rs` (~line 34)

**Step 1: Update `build_client()` in demo/src/main.rs**

The `DataLocation { address: "embedded".to_string() }` passed to `MetadataServer::new()` doesn't need to change — the parameter was renamed but the call site just passes a positional argument. Verify this compiles.

**Step 2: Update `new_client()` and `new_metadata_and_data()` in integration test**

Same — positional arguments, should compile as-is. Verify.

**Step 3: Run full test suite**

Run: `cargo test --workspace`
Expected: All ~192+ tests pass.

**Step 4: Commit (only if changes were needed)**

```
chore(demo): update for renamed default_data_location field
```

---

### Task 8: VfsCore routing integration test

**Files:**
- Modify: `demo/tests/integration_test.rs`

**Step 1: Add multi-DataServer routing test**

```rust
/// Test that VfsCore routes read/write to the correct DataServer
/// based on the data_location returned by open().
#[tokio::test]
async fn vfscore_routes_to_correct_dataserver() {
    use rucksfs_client::VfsCore;

    let tmp = tempfile::tempdir().unwrap();
    let db = open_rocks_db(tmp.path().join("meta.db")).unwrap();
    let metadata_store = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
    let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
    let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
    let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));

    // Create two separate DataServers with different backing files
    let ds_a = Arc::new(DataServer::new(
        RawDiskDataStore::open(&tmp.path().join("data_a.raw"), 64 * 1024 * 1024).unwrap(),
    )) as Arc<dyn DataOps>;
    let ds_b = Arc::new(DataServer::new(
        RawDiskDataStore::open(&tmp.path().join("data_b.raw"), 64 * 1024 * 1024).unwrap(),
    )) as Arc<dyn DataOps>;

    let server_address = "ds-a:9001";
    let metadata_server: Arc<dyn MetadataOps> = Arc::new(MetadataServer::new(
        metadata_store,
        index,
        delta_store,
        Arc::clone(&ds_a),
        DataLocation { address: server_address.to_string() },
        storage_bundle,
    ));

    // Register ds_a under the address that MetadataServer will return
    let mut servers = std::collections::HashMap::new();
    servers.insert(server_address.to_string(), Arc::clone(&ds_a));
    servers.insert("ds-b:9002".to_string(), Arc::clone(&ds_b));

    let vfs = VfsCore::with_data_servers(
        metadata_server,
        Arc::clone(&ds_a),
        servers,
    );

    // Create a file — MetadataServer assigns default_data_location "ds-a:9001"
    let file = vfs.create(ROOT, "routed_file", 0o644, 0, 0).await.unwrap();
    let handle = vfs.open(file.inode, 0).await.unwrap();

    // Write through VfsCore — should route to ds_a
    vfs.write(file.inode, 0, b"routed data", 0).await.unwrap();

    // Read back through VfsCore
    let data = vfs.read(file.inode, 0, 11).await.unwrap();
    assert_eq!(&data[..11], b"routed data");

    // Verify ds_b did NOT receive the data
    let ds_b_data = ds_b.read_data(file.inode, 0, 11).await.unwrap();
    assert_eq!(ds_b_data, vec![0u8; 11]);

    // Release
    vfs.release(file.inode).await.unwrap();
    let _ = handle; // suppress unused warning
}
```

**Step 2: Run tests**

Run: `cargo test -p rucksfs --test integration_test -- vfscore_routes`
Expected: PASS

**Step 3: Run full workspace test suite**

Run: `cargo test --workspace`
Expected: All tests pass.

**Step 4: Commit**

```
test(client): add VfsCore multi-DataServer routing integration test
```

---

### Task 9: Final verification and TODO update

**Step 1: Run full test suite**

Run: `cargo test --workspace`
Expected: All tests pass.

**Step 2: Run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings.

**Step 3: Update TODO.md — mark T-27 as done**

Change `| T-27 | 🔧 |` to `| T-27 | ✅ |` and add affected files summary.

**Step 4: Commit**

```
docs(todo): mark T-27 per-inode DataLocation as done
```
