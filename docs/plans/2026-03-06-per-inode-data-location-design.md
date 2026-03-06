# Per-Inode DataLocation + VfsCore Multi-DataServer Routing

**Date:** 2026-03-06
**Status:** Approved
**TODO:** T-27

## Problem

`MetadataServer` holds a single global `data_location: DataLocation` and returns it for every `open()` call regardless of inode. This means:

1. Metadata does not record which DataServer stores a given file's data.
2. `VfsCore` has a `handle_cache` for handle-to-address mapping but never uses it — `read/write` always go to the single `self.data`.
3. There is no path to supporting multiple DataServers without redesigning these layers.

## Decision

**Approach B: Independent data_location mapping table.** Store `inode → data_location` as a separate key-prefix in RocksDB rather than embedding it in `InodeValue`. This keeps `InodeValue` at its fixed 57-byte format and avoids impacting delta fold, cache, and compaction paths.

## Design

### 1. Storage Layer: data_location Mapping Table

**Key encoding:** `[b'L'][inode: u64 BE]` — 9 bytes total, same structure as inode keys but with prefix `'L'`.

**Value:** Raw UTF-8 bytes of the DataServer address string. No length prefix needed (one key = one value).

**Lifecycle:**
- Written on `create` / `symlink` (inside the same `AtomicWriteBatch` as the inode, ensuring atomicity).
- Read on `open`.
- Deleted on `unlink` / `rename` when nlink reaches 0 (inside the same batch as inode deletion).
- Not written for `mkdir` (directories have no data storage location).

**RocksDB placement:** Reuse the existing inodes CF. Key prefix `'L'` does not collide with `'I'` (inodes), `'D'` (dir entries), or `'X'` (deltas).

**Impact on InodeValue, delta, cache, compaction: None.**

### 2. Server Layer: MetadataServer Changes

**Constructor:** Rename `data_location` field to `default_data_location`. Semantics change from "the only DataServer" to "default DataServer for new files".

**`create` / `symlink`:** Add a `PutDataLocation` operation to the `AtomicWriteBatch`, writing `[b'L'][new_inode] → default_data_location.address`.

**`open`:** Read `[b'L'][inode]` from MetadataStore. If found, return that address in `OpenResponse`. If not found (legacy data or directories), fall back to `default_data_location`.

**`unlink` (nlink → 0):** Add `DeleteDataLocation` to the batch alongside `DeleteInode`.

**`rename` (overwrite target, nlink → 0):** Same as unlink — delete the overwritten file's data_location.

**`link`:** No change. Hard links share the same inode; data_location is unchanged.

**`BatchOp` enum:** Add two variants:
- `PutDataLocation { key: Vec<u8>, value: Vec<u8> }`
- `DeleteDataLocation { key: Vec<u8> }`

These operate on the inodes CF (same as `PutInode`/`DeleteInode`), just with different key prefixes.

### 3. Client Layer: VfsCore Multi-DataServer Routing

**New fields:**

```rust
VfsCore {
    metadata: Arc<dyn MetadataOps>,
    default_data: Arc<dyn DataOps>,
    data_servers: Mutex<HashMap<String, Arc<dyn DataOps>>>,
    handle_map: Mutex<HashMap<u64, String>>,
}
```

**`new()` signature:** Accepts `default_data: Arc<dyn DataOps>` plus an optional `HashMap<String, Arc<dyn DataOps>>` for additional DataServers. Single-binary mode passes an empty map.

**`open()`:** Calls `metadata.open()`, stores `handle → data_location.address` in `handle_map`.

**`read()` / `write()` / `flush()` / `fsync()`:** Look up inode in `handle_map` to get address, then find the corresponding `Arc<dyn DataOps>` in `data_servers`. Fall back to `default_data` if address not found.

**`release()`:** Remove the handle's entry from `handle_map` after calling `metadata.release()`.

**EmbeddedClient:** Constructor passes empty `data_servers` map. Behavior is identical to current single-DataServer mode.

**FUSE layer impact: None.** It interacts only with `VfsOps`.

### 4. Testing Strategy

**Unit tests (in existing test modules):**
- `storage/src/encoding.rs`: roundtrip test for `encode_data_location_key`.
- `server` tests: verify `create` → `open` returns correct DataLocation; verify `unlink` (nlink=0) clears the mapping.

**Integration tests (`demo/tests/integration_test.rs`):**
- Existing tests pass unchanged (they don't inspect DataLocation values).

**VfsCore routing test (`client/` module):**
- Construct two RawDisk-backed DataServers with different tempdirs.
- Register both in `data_servers` map.
- Create files with different DataLocations, verify read/write routes to the correct DataServer.

**Not in scope:**
- fsck changes (future enhancement).
- FUSE E2E test changes (behavior unchanged).
- RPC/network tests (T-28 scope).
