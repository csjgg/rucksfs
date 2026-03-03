//! RocksDB-backed implementations of [`MetadataStore`] and [`DirectoryIndex`].
//!
//! This module is only compiled when the `rocksdb` cargo feature is enabled.
//! It provides persistent storage using three RocksDB Column Families:
//!
//! - **inodes**: inode metadata (key = encoded inode key, value = serialized `InodeValue`)
//! - **dir_entries**: directory entries (key = encoded dir entry key, value = child inode as u64 BE)
//! - **system**: system-level KV pairs (e.g. next inode counter)

use rocksdb::{
    ColumnFamilyDescriptor, Options, Transaction, TransactionDB, TransactionDBOptions,
    TransactionOptions, WriteBatchWithTransaction, WriteOptions,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use rucksfs_core::{DirEntry, FileAttr, FsError, FsResult, Inode};

use crate::encoding::{
    decode_delta_key, delta_prefix, dir_entry_prefix, encode_delta_key, encode_dir_entry_key,
    extract_child_name,
};
use crate::{AtomicWriteBatch, BatchOp, DeltaStore, DirectoryIndex, MetadataStore, StorageBundle};

// ... existing CF constants and code below ...

/// Column family name for inode metadata.
const CF_INODES: &str = "inodes";
/// Column family name for directory entries.
const CF_DIR_ENTRIES: &str = "dir_entries";
/// Column family name for system-level data (e.g. allocator state).
const CF_SYSTEM: &str = "system";
/// Column family name for delta entries (append-only incremental inode updates).
const CF_DELTA_ENTRIES: &str = "delta_entries";

/// All column family names used by the storage layer.
const ALL_CFS: &[&str] = &[CF_INODES, CF_DIR_ENTRIES, CF_SYSTEM, CF_DELTA_ENTRIES];

/// Open (or create) a RocksDB database with the required column families.
///
/// This is a shared helper so that both `RocksMetadataStore` and
/// `RocksDirectoryIndex` can be created from the same `Arc<DB>`.
pub fn open_rocks_db(path: impl AsRef<Path>) -> FsResult<Arc<TransactionDB>> {
    let mut db_opts = Options::default();
    db_opts.create_if_missing(true);
    db_opts.create_missing_column_families(true);

    let txn_db_opts = TransactionDBOptions::default();

    let cf_descriptors: Vec<ColumnFamilyDescriptor> = ALL_CFS
        .iter()
        .map(|name| ColumnFamilyDescriptor::new(*name, Options::default()))
        .collect();

    let db = TransactionDB::open_cf_descriptors(&db_opts, &txn_db_opts, path.as_ref(), cf_descriptors)
        .map_err(|e| FsError::Io(format!("RocksDB open failed: {}", e)))?;

    Ok(Arc::new(db))
}

// ===========================================================================
// RocksMetadataStore
// ===========================================================================

/// Persistent metadata store backed by RocksDB.
///
/// Keys and values are stored in the `inodes` and `system` column families.
/// The generic KV interface ([`MetadataStore`]) maps directly to RocksDB
/// get/put/delete operations on the `inodes` CF.
pub struct RocksMetadataStore {
    db: Arc<TransactionDB>,
}

impl RocksMetadataStore {
    /// Create a new store from a shared DB handle.
    pub fn new(db: Arc<TransactionDB>) -> Self {
        Self { db }
    }

    /// Convenience: open a new DB at `path` and return the store.
    pub fn open(path: impl AsRef<Path>) -> FsResult<Self> {
        let db = open_rocks_db(path)?;
        Ok(Self::new(db))
    }

    /// Get a reference to the underlying DB (useful for sharing with
    /// `RocksDirectoryIndex`).
    pub fn db(&self) -> &Arc<TransactionDB> {
        &self.db
    }
}

impl MetadataStore for RocksMetadataStore {
    fn get(&self, key: &[u8]) -> FsResult<Option<Vec<u8>>> {
        let cf = self
            .db
            .cf_handle(CF_INODES)
            .ok_or_else(|| FsError::Io("CF 'inodes' not found".into()))?;
        self.db
            .get_cf(&cf, key)
            .map_err(|e| FsError::Io(format!("RocksDB get: {}", e)))
    }

    fn put(&self, key: &[u8], value: &[u8]) -> FsResult<()> {
        let cf = self
            .db
            .cf_handle(CF_INODES)
            .ok_or_else(|| FsError::Io("CF 'inodes' not found".into()))?;
        self.db
            .put_cf(&cf, key, value)
            .map_err(|e| FsError::Io(format!("RocksDB put: {}", e)))
    }

    fn delete(&self, key: &[u8]) -> FsResult<()> {
        let cf = self
            .db
            .cf_handle(CF_INODES)
            .ok_or_else(|| FsError::Io("CF 'inodes' not found".into()))?;
        self.db
            .delete_cf(&cf, key)
            .map_err(|e| FsError::Io(format!("RocksDB delete: {}", e)))
    }

    fn scan_prefix(&self, prefix: &[u8]) -> FsResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let cf = self
            .db
            .cf_handle(CF_INODES)
            .ok_or_else(|| FsError::Io("CF 'inodes' not found".into()))?;

        let iter = self.db.prefix_iterator_cf(&cf, prefix);
        let mut result = Vec::new();
        for item in iter {
            let (k, v) = item.map_err(|e| FsError::Io(format!("RocksDB iterator: {}", e)))?;
            if !k.starts_with(prefix) {
                break;
            }
            result.push((k.to_vec(), v.to_vec()));
        }
        Ok(result)
    }
}

// ===========================================================================
// RocksDirectoryIndex
// ===========================================================================

/// Persistent directory index backed by the `dir_entries` column family.
///
/// Each directory entry is stored as:
/// - key: `encode_dir_entry_key(parent, name)`
/// - value: child inode as u64 big-endian + mode as u32 big-endian (12 bytes)
pub struct RocksDirectoryIndex {
    db: Arc<TransactionDB>,
}

/// Serialized size of a directory entry value: inode(8) + mode(4) = 12 bytes.
const DIR_VALUE_SIZE: usize = 12;

impl RocksDirectoryIndex {
    /// Create a new directory index from a shared DB handle.
    pub fn new(db: Arc<TransactionDB>) -> Self {
        Self { db }
    }

    /// Convenience: open a new DB at `path` and return the index.
    pub fn open(path: impl AsRef<Path>) -> FsResult<Self> {
        let db = open_rocks_db(path)?;
        Ok(Self::new(db))
    }

    /// Encode a directory entry value: `[inode: u64 BE][mode: u32 BE]`.
    fn encode_dir_value(inode: Inode, mode: u32) -> Vec<u8> {
        let mut buf = Vec::with_capacity(DIR_VALUE_SIZE);
        buf.extend_from_slice(&inode.to_be_bytes());
        buf.extend_from_slice(&mode.to_be_bytes());
        buf
    }

    /// Decode a directory entry value.
    fn decode_dir_value(data: &[u8]) -> FsResult<(Inode, u32)> {
        if data.len() < DIR_VALUE_SIZE {
            return Err(FsError::InvalidInput(format!(
                "dir entry value too short: {} bytes",
                data.len()
            )));
        }
        let inode = u64::from_be_bytes(data[0..8].try_into().unwrap());
        let mode = u32::from_be_bytes(data[8..12].try_into().unwrap());
        Ok((inode, mode))
    }
}

impl DirectoryIndex for RocksDirectoryIndex {
    fn resolve_path(&self, parent: Inode, name: &str) -> FsResult<Option<Inode>> {
        let cf = self
            .db
            .cf_handle(CF_DIR_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'dir_entries' not found".into()))?;
        let key = encode_dir_entry_key(parent, name);
        match self
            .db
            .get_cf(&cf, &key)
            .map_err(|e| FsError::Io(format!("RocksDB get: {}", e)))?
        {
            Some(val) => {
                let (inode, _mode) = Self::decode_dir_value(&val)?;
                Ok(Some(inode))
            }
            None => Ok(None),
        }
    }

    fn list_dir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        let cf = self
            .db
            .cf_handle(CF_DIR_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'dir_entries' not found".into()))?;
        let prefix = dir_entry_prefix(inode);
        let iter = self.db.prefix_iterator_cf(&cf, &prefix);

        let mut entries = Vec::new();
        for item in iter {
            let (k, v) = item.map_err(|e| FsError::Io(format!("RocksDB iterator: {}", e)))?;
            if !k.starts_with(&prefix) {
                break;
            }
            let name = extract_child_name(&k)?.to_string();
            let (child_inode, kind) = Self::decode_dir_value(&v)?;
            entries.push(DirEntry {
                name,
                inode: child_inode,
                kind,
            });
        }
        Ok(entries)
    }

    fn insert_child(
        &self,
        parent: Inode,
        name: &str,
        inode: Inode,
        attr: FileAttr,
    ) -> FsResult<()> {
        let cf = self
            .db
            .cf_handle(CF_DIR_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'dir_entries' not found".into()))?;
        let key = encode_dir_entry_key(parent, name);
        let value = Self::encode_dir_value(inode, attr.mode);
        self.db
            .put_cf(&cf, &key, &value)
            .map_err(|e| FsError::Io(format!("RocksDB put: {}", e)))
    }

    fn remove_child(&self, parent: Inode, name: &str) -> FsResult<()> {
        let cf = self
            .db
            .cf_handle(CF_DIR_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'dir_entries' not found".into()))?;
        let key = encode_dir_entry_key(parent, name);
        self.db
            .delete_cf(&cf, &key)
            .map_err(|e| FsError::Io(format!("RocksDB delete: {}", e)))
    }
}

// ===========================================================================
// RocksDeltaStore
// ===========================================================================

/// Persistent delta store backed by the `delta_entries` column family.
///
/// Each delta entry is stored as:
/// - key: `encode_delta_key(inode, seq)` — 17 bytes
/// - value: serialized `DeltaOp` bytes (produced by the server layer)
///
/// Per-inode sequence numbers are tracked in memory using `AtomicU64`
/// counters and recovered from the CF on startup.
pub struct RocksDeltaStore {
    db: Arc<TransactionDB>,
    /// Per-inode next sequence number.  Lazily populated on first access
    /// or recovered from disk on `recover_seq`.
    seqs: RwLock<HashMap<Inode, AtomicU64>>,
}

impl RocksDeltaStore {
    /// Create a new delta store from a shared DB handle.
    pub fn new(db: Arc<TransactionDB>) -> Self {
        Self {
            db,
            seqs: RwLock::new(HashMap::new()),
        }
    }

    /// Recover per-inode sequence counters by scanning the `delta_entries` CF.
    ///
    /// Should be called once at startup before serving requests.
    pub fn recover_seqs(&self) -> FsResult<()> {
        let cf = self
            .db
            .cf_handle(CF_DELTA_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'delta_entries' not found".into()))?;

        let iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::End);
        let mut max_seqs: HashMap<Inode, u64> = HashMap::new();

        // Iterate from end to collect max seq per inode efficiently.
        for item in iter {
            let (k, _) = item.map_err(|e| FsError::Io(format!("RocksDB iterator: {}", e)))?;
            if k.len() < 17 {
                continue;
            }
            if let Ok((inode, seq)) = decode_delta_key(&k) {
                max_seqs
                    .entry(inode)
                    .and_modify(|s| {
                        if seq > *s {
                            *s = seq;
                        }
                    })
                    .or_insert(seq);
            }
        }

        let mut guard = self.seqs.write().map_err(|e| {
            FsError::Io(format!("RocksDeltaStore seqs lock poisoned: {}", e))
        })?;
        for (inode, max_seq) in max_seqs {
            guard.insert(inode, AtomicU64::new(max_seq + 1));
        }
        Ok(())
    }

    /// Allocate the next sequence number for `inode`.
    fn next_seq(&self, inode: Inode) -> u64 {
        // Fast path: counter already exists.
        {
            let guard = self.seqs.read().expect("seqs read lock poisoned");
            if let Some(counter) = guard.get(&inode) {
                return counter.fetch_add(1, Ordering::Relaxed);
            }
        }
        // Slow path: create the counter.
        let mut guard = self.seqs.write().expect("seqs write lock poisoned");
        let counter = guard.entry(inode).or_insert_with(|| AtomicU64::new(0));
        counter.fetch_add(1, Ordering::Relaxed)
    }
}

impl DeltaStore for RocksDeltaStore {
    fn append_deltas(&self, inode: Inode, values: &[Vec<u8>]) -> FsResult<Vec<u64>> {
        let cf = self
            .db
            .cf_handle(CF_DELTA_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'delta_entries' not found".into()))?;

        let mut batch = WriteBatchWithTransaction::<true>::default();
        let mut assigned = Vec::with_capacity(values.len());

        for v in values {
            let seq = self.next_seq(inode);
            let key = encode_delta_key(inode, seq);
            batch.put_cf(&cf, &key, v);
            assigned.push(seq);
        }

        self.db
            .write(batch)
            .map_err(|e| FsError::Io(format!("RocksDB write batch: {}", e)))?;

        Ok(assigned)
    }

    fn scan_deltas(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>> {
        let cf = self
            .db
            .cf_handle(CF_DELTA_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'delta_entries' not found".into()))?;

        let prefix = delta_prefix(inode);
        let iter = self.db.prefix_iterator_cf(&cf, &prefix);

        let mut result = Vec::new();
        for item in iter {
            let (k, v) = item.map_err(|e| FsError::Io(format!("RocksDB iterator: {}", e)))?;
            if !k.starts_with(&prefix) {
                break;
            }
            result.push(v.to_vec());
        }
        Ok(result)
    }

    fn scan_delta_keys(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>> {
        let cf = self
            .db
            .cf_handle(CF_DELTA_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'delta_entries' not found".into()))?;

        let prefix = delta_prefix(inode);
        let iter = self.db.prefix_iterator_cf(&cf, &prefix);

        let mut result = Vec::new();
        for item in iter {
            let (k, _) = item.map_err(|e| FsError::Io(format!("RocksDB iterator: {}", e)))?;
            if !k.starts_with(&prefix) {
                break;
            }
            result.push(k.to_vec());
        }
        Ok(result)
    }

    fn clear_deltas(&self, inode: Inode) -> FsResult<()> {
        let cf = self
            .db
            .cf_handle(CF_DELTA_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'delta_entries' not found".into()))?;

        // Scan and delete all delta entries for the given inode.
        let prefix = delta_prefix(inode);
        let iter = self.db.prefix_iterator_cf(&cf, &prefix);

        let mut batch = WriteBatchWithTransaction::<true>::default();
        for item in iter {
            let (k, _) = item.map_err(|e| FsError::Io(format!("RocksDB iterator: {}", e)))?;
            if !k.starts_with(&prefix) {
                break;
            }
            batch.delete_cf(&cf, &k);
        }

        self.db
            .write(batch)
            .map_err(|e| FsError::Io(format!("RocksDB write batch: {}", e)))?;

        // Reset in-memory sequence counter.
        if let Ok(mut guard) = self.seqs.write() {
            if let Some(counter) = guard.get_mut(&inode) {
                counter.store(0, Ordering::Relaxed);
            }
        }

        Ok(())
    }
}

// ===========================================================================
// RocksStorageBundle — atomic cross-CF write batch
// ===========================================================================

/// A bundle of RocksDB-backed stores that supports atomic cross-CF writes.
///
/// All three stores (metadata, directory index, delta) share the same
/// underlying `Arc<DB>`, so a single `WriteBatch` can span all column
/// families atomically.
pub struct RocksStorageBundle {
    db: Arc<TransactionDB>,
}

impl RocksStorageBundle {
    /// Create a new bundle from a shared DB handle.
    pub fn new(db: Arc<TransactionDB>) -> Self {
        Self { db }
    }
}

/// Atomic write batch backed by a RocksDB PCC `Transaction`.
///
/// Each operation is applied to the transaction immediately via
/// `txn.put_cf()` / `txn.delete_cf()`.  `commit()` calls
/// `txn.commit()` which is atomic.  `get_for_update_*` methods
/// acquire pessimistic row locks inside the transaction.
struct RocksWriteBatch<'db> {
    txn: Transaction<'db, TransactionDB>,
    db: Arc<TransactionDB>,
}

/// Helper to map RocksDB errors from a transaction operation.
///
/// `Status::Busy` and `Status::TimedOut` (deadlock / lock-wait timeout)
/// are mapped to `FsError::TransactionConflict`; everything else becomes
/// `FsError::Io`.
fn map_txn_err(e: rocksdb::Error) -> FsError {
    let msg = e.to_string();
    if msg.contains("Busy") || msg.contains("TimedOut") || msg.contains("deadlock") {
        FsError::TransactionConflict
    } else {
        FsError::Io(format!("RocksDB transaction: {}", e))
    }
}

impl<'db> AtomicWriteBatch for RocksWriteBatch<'db> {
    fn push(&mut self, op: BatchOp) {
        match op {
            BatchOp::PutInode { key, value } => {
                let cf = self.db.cf_handle(CF_INODES)
                    .expect("CF 'inodes' must exist — database is corrupt or misconfigured");
                self.txn.put_cf(&cf, &key, &value)
                    .expect("transaction put_cf(inodes) failed unexpectedly");
            }
            BatchOp::DeleteInode { key } => {
                let cf = self.db.cf_handle(CF_INODES)
                    .expect("CF 'inodes' must exist — database is corrupt or misconfigured");
                self.txn.delete_cf(&cf, &key)
                    .expect("transaction delete_cf(inodes) failed unexpectedly");
            }
            BatchOp::PutDirEntry { key, value } => {
                let cf = self.db.cf_handle(CF_DIR_ENTRIES)
                    .expect("CF 'dir_entries' must exist — database is corrupt or misconfigured");
                self.txn.put_cf(&cf, &key, &value)
                    .expect("transaction put_cf(dir_entries) failed unexpectedly");
            }
            BatchOp::DeleteDirEntry { key } => {
                let cf = self.db.cf_handle(CF_DIR_ENTRIES)
                    .expect("CF 'dir_entries' must exist — database is corrupt or misconfigured");
                self.txn.delete_cf(&cf, &key)
                    .expect("transaction delete_cf(dir_entries) failed unexpectedly");
            }
            BatchOp::PutDelta { key, value } => {
                let cf = self.db.cf_handle(CF_DELTA_ENTRIES)
                    .expect("CF 'delta_entries' must exist — database is corrupt or misconfigured");
                self.txn.put_cf(&cf, &key, &value)
                    .expect("transaction put_cf(delta_entries) failed unexpectedly");
            }
            BatchOp::DeleteDelta { key } => {
                let cf = self.db.cf_handle(CF_DELTA_ENTRIES)
                    .expect("CF 'delta_entries' must exist — database is corrupt or misconfigured");
                self.txn.delete_cf(&cf, &key)
                    .expect("transaction delete_cf(delta_entries) failed unexpectedly");
            }
            BatchOp::PutSystem { key, value } => {
                let cf = self.db.cf_handle(CF_SYSTEM)
                    .expect("CF 'system' must exist — database is corrupt or misconfigured");
                self.txn.put_cf(&cf, &key, &value)
                    .expect("transaction put_cf(system) failed unexpectedly");
            }
        }
    }

    fn commit(self: Box<Self>) -> FsResult<()> {
        self.txn.commit().map_err(map_txn_err)
    }

    fn get_for_update_inode(&self, key: &[u8]) -> FsResult<Option<Vec<u8>>> {
        let cf = self.db.cf_handle(CF_INODES)
            .ok_or_else(|| FsError::Io("CF 'inodes' not found".into()))?;
        self.txn
            .get_for_update_cf(&cf, key, true)
            .map_err(map_txn_err)
    }

    fn get_for_update_dir_entry(&self, key: &[u8]) -> FsResult<Option<Vec<u8>>> {
        let cf = self.db.cf_handle(CF_DIR_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'dir_entries' not found".into()))?;
        self.txn
            .get_for_update_cf(&cf, key, true)
            .map_err(map_txn_err)
    }

    fn is_dir_empty(&self, parent: Inode) -> FsResult<bool> {
        let cf = self.db.cf_handle(CF_DIR_ENTRIES)
            .ok_or_else(|| FsError::Io("CF 'dir_entries' not found".into()))?;
        let prefix = dir_entry_prefix(parent);
        // Use transaction-level iterator so the read participates in
        // the transaction's snapshot, avoiding TOCTOU races.
        let iter = self.txn.prefix_iterator_cf(&cf, &prefix);
        for item in iter {
            let (k, _) = item.map_err(|e| FsError::Io(format!("RocksDB txn iterator: {}", e)))?;
            if k.starts_with(&prefix) {
                return Ok(false);
            }
            break;
        }
        Ok(true)
    }
}

impl StorageBundle for RocksStorageBundle {
    fn begin_write(&self) -> Box<dyn AtomicWriteBatch + '_> {
        let txn_opts = TransactionOptions::default();
        let write_opts = WriteOptions::default();
        let txn = self.db.transaction_opt(&write_opts, &txn_opts);
        Box::new(RocksWriteBatch {
            txn,
            db: Arc::clone(&self.db),
        })
    }
}

// ===========================================================================
// Tests
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temporary RocksDB and return the shared DB handle.
    fn temp_db() -> (tempfile::TempDir, Arc<TransactionDB>) {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let db = open_rocks_db(tmp.path()).expect("open RocksDB");
        (tmp, db)
    }

    // -----------------------------------------------------------------------
    // RocksMetadataStore tests
    // -----------------------------------------------------------------------
    mod metadata_store {
        use super::*;
        use crate::encoding::InodeValue;

        fn new_store() -> (tempfile::TempDir, RocksMetadataStore) {
            let (tmp, db) = temp_db();
            (tmp, RocksMetadataStore::new(db))
        }

        #[test]
        fn basic_put_get() {
            let (_tmp, store) = new_store();
            store.put(b"k1", b"v1").unwrap();
            assert_eq!(store.get(b"k1").unwrap(), Some(b"v1".to_vec()));
        }

        #[test]
        fn get_missing_key() {
            let (_tmp, store) = new_store();
            assert_eq!(store.get(b"nope").unwrap(), None);
        }

        #[test]
        fn put_overwrite() {
            let (_tmp, store) = new_store();
            store.put(b"k", b"v1").unwrap();
            store.put(b"k", b"v2").unwrap();
            assert_eq!(store.get(b"k").unwrap(), Some(b"v2".to_vec()));
        }

        #[test]
        fn delete_existing() {
            let (_tmp, store) = new_store();
            store.put(b"k", b"v").unwrap();
            store.delete(b"k").unwrap();
            assert_eq!(store.get(b"k").unwrap(), None);
        }

        #[test]
        fn delete_idempotent() {
            let (_tmp, store) = new_store();
            // Delete non-existent key should succeed
            store.delete(b"ghost").unwrap();
        }

        #[test]
        fn scan_prefix_basic() {
            let (_tmp, store) = new_store();
            // Use inode keys to ensure prefix scan works with real encoding
            let k1 = crate::encoding::encode_inode_key(1);
            let k2 = crate::encoding::encode_inode_key(2);
            let k3 = crate::encoding::encode_inode_key(3);

            let attr1 = FileAttr {
                inode: 1,
                size: 100,
                mode: 0o100644,
                nlink: 1,
                ..Default::default()
            };
            let attr2 = FileAttr {
                inode: 2,
                size: 200,
                mode: 0o040755,
                nlink: 2,
                ..Default::default()
            };
            let attr3 = FileAttr {
                inode: 3,
                size: 300,
                mode: 0o100644,
                nlink: 1,
                ..Default::default()
            };

            store
                .put(&k1, &InodeValue::from_attr(&attr1).serialize())
                .unwrap();
            store
                .put(&k2, &InodeValue::from_attr(&attr2).serialize())
                .unwrap();
            store
                .put(&k3, &InodeValue::from_attr(&attr3).serialize())
                .unwrap();

            // Scan with prefix 'I' should return all inode keys
            let result = store.scan_prefix(&[b'I']).unwrap();
            assert_eq!(result.len(), 3);
        }

        #[test]
        fn scan_prefix_empty() {
            let (_tmp, store) = new_store();
            store.put(b"other", b"v").unwrap();
            let result = store.scan_prefix(b"nope").unwrap();
            assert!(result.is_empty());
        }

        #[test]
        fn inode_roundtrip() {
            let (_tmp, store) = new_store();
            let attr = FileAttr {
                inode: 42,
                size: 1024,
                mode: 0o100644,
                nlink: 1,
                uid: 1000,
                gid: 1000,
                atime: 1_700_000_000,
                mtime: 1_700_000_001,
                ctime: 1_700_000_002,
            };
            let key = crate::encoding::encode_inode_key(42);
            let val = InodeValue::from_attr(&attr).serialize();
            store.put(&key, &val).unwrap();

            let loaded = store.get(&key).unwrap().unwrap();
            let restored = InodeValue::deserialize(&loaded).unwrap().to_attr();
            assert_eq!(restored, attr);
        }

        #[test]
        fn data_persists_across_reopen() {
            let tmp = tempfile::tempdir().expect("create temp dir");

            // Write some data
            {
                let db = open_rocks_db(tmp.path()).unwrap();
                let store = RocksMetadataStore::new(db);
                store.put(b"persistent_key", b"persistent_value").unwrap();
            }

            // Re-open and verify
            {
                let db = open_rocks_db(tmp.path()).unwrap();
                let store = RocksMetadataStore::new(db);
                assert_eq!(
                    store.get(b"persistent_key").unwrap(),
                    Some(b"persistent_value".to_vec())
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // RocksDirectoryIndex tests
    // -----------------------------------------------------------------------
    mod directory_index {
        use super::*;

        fn new_index() -> (tempfile::TempDir, RocksDirectoryIndex) {
            let (tmp, db) = temp_db();
            (tmp, RocksDirectoryIndex::new(db))
        }

        fn make_attr(inode: Inode, mode: u32) -> FileAttr {
            FileAttr {
                inode,
                mode,
                ..Default::default()
            }
        }

        #[test]
        fn insert_and_resolve() {
            let (_tmp, idx) = new_index();
            idx.insert_child(1, "hello.txt", 10, make_attr(10, 0o100644))
                .unwrap();
            assert_eq!(idx.resolve_path(1, "hello.txt").unwrap(), Some(10));
        }

        #[test]
        fn resolve_missing() {
            let (_tmp, idx) = new_index();
            assert_eq!(idx.resolve_path(1, "nope").unwrap(), None);
        }

        #[test]
        fn list_dir_entries() {
            let (_tmp, idx) = new_index();
            idx.insert_child(1, "a", 10, make_attr(10, 0o100644))
                .unwrap();
            idx.insert_child(1, "b", 11, make_attr(11, 0o040755))
                .unwrap();

            let entries = idx.list_dir(1).unwrap();
            assert_eq!(entries.len(), 2);
            let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
            assert!(names.contains(&"a"));
            assert!(names.contains(&"b"));
        }

        #[test]
        fn list_empty_dir() {
            let (_tmp, idx) = new_index();
            let entries = idx.list_dir(999).unwrap();
            assert!(entries.is_empty());
        }

        #[test]
        fn remove_child_and_resolve() {
            let (_tmp, idx) = new_index();
            idx.insert_child(1, "f", 10, make_attr(10, 0o100644))
                .unwrap();
            idx.remove_child(1, "f").unwrap();
            assert_eq!(idx.resolve_path(1, "f").unwrap(), None);
        }

        #[test]
        fn remove_nonexistent_is_ok() {
            let (_tmp, idx) = new_index();
            idx.remove_child(1, "ghost").unwrap();
        }

        #[test]
        fn overwrite_child() {
            let (_tmp, idx) = new_index();
            idx.insert_child(1, "f", 10, make_attr(10, 0o100644))
                .unwrap();
            idx.insert_child(1, "f", 20, make_attr(20, 0o100644))
                .unwrap();
            assert_eq!(idx.resolve_path(1, "f").unwrap(), Some(20));
        }

        #[test]
        fn dir_isolation_between_parents() {
            let (_tmp, idx) = new_index();
            idx.insert_child(1, "shared_name", 10, make_attr(10, 0o100644))
                .unwrap();
            idx.insert_child(2, "shared_name", 20, make_attr(20, 0o100644))
                .unwrap();

            assert_eq!(idx.resolve_path(1, "shared_name").unwrap(), Some(10));
            assert_eq!(idx.resolve_path(2, "shared_name").unwrap(), Some(20));

            let entries1 = idx.list_dir(1).unwrap();
            assert_eq!(entries1.len(), 1);
            let entries2 = idx.list_dir(2).unwrap();
            assert_eq!(entries2.len(), 1);
        }

        #[test]
        fn preserves_mode_in_dir_entry() {
            let (_tmp, idx) = new_index();
            idx.insert_child(1, "dir", 10, make_attr(10, 0o040755))
                .unwrap();
            idx.insert_child(1, "file", 11, make_attr(11, 0o100644))
                .unwrap();

            let entries = idx.list_dir(1).unwrap();
            let dir_entry = entries.iter().find(|e| e.name == "dir").unwrap();
            let file_entry = entries.iter().find(|e| e.name == "file").unwrap();
            assert_eq!(dir_entry.kind, 0o040755);
            assert_eq!(file_entry.kind, 0o100644);
        }

        #[test]
        fn data_persists_across_reopen() {
            let tmp = tempfile::tempdir().expect("create temp dir");

            // Write
            {
                let db = open_rocks_db(tmp.path()).unwrap();
                let idx = RocksDirectoryIndex::new(db);
                idx.insert_child(1, "persistent", 42, make_attr(42, 0o100644))
                    .unwrap();
            }

            // Re-open and verify
            {
                let db = open_rocks_db(tmp.path()).unwrap();
                let idx = RocksDirectoryIndex::new(db);
                assert_eq!(idx.resolve_path(1, "persistent").unwrap(), Some(42));
                let entries = idx.list_dir(1).unwrap();
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "persistent");
            }
        }

        #[test]
        fn encode_decode_dir_value_roundtrip() {
            let encoded = RocksDirectoryIndex::encode_dir_value(42, 0o040755);
            assert_eq!(encoded.len(), DIR_VALUE_SIZE);
            let (inode, mode) = RocksDirectoryIndex::decode_dir_value(&encoded).unwrap();
            assert_eq!(inode, 42);
            assert_eq!(mode, 0o040755);
        }

        #[test]
        fn decode_dir_value_too_short() {
            let result = RocksDirectoryIndex::decode_dir_value(&[0u8; 4]);
            assert!(result.is_err());
        }
    }

    // -----------------------------------------------------------------------
    // Integration: shared DB between metadata store and directory index
    // -----------------------------------------------------------------------
    mod integration {
        use super::*;
        use crate::encoding::{encode_inode_key, InodeValue};

        #[test]
        fn shared_db_metadata_and_dir_index() {
            let (_tmp, db) = temp_db();
            let meta = RocksMetadataStore::new(Arc::clone(&db));
            let dir_idx = RocksDirectoryIndex::new(db);

            // Store inode metadata
            let attr = FileAttr {
                inode: 10,
                size: 512,
                mode: 0o100644,
                nlink: 1,
                uid: 1000,
                gid: 1000,
                atime: 100,
                mtime: 200,
                ctime: 300,
            };
            let key = encode_inode_key(10);
            meta.put(&key, &InodeValue::from_attr(&attr).serialize())
                .unwrap();

            // Store directory entry
            dir_idx
                .insert_child(1, "myfile.txt", 10, attr.clone())
                .unwrap();

            // Verify both work
            let loaded = meta.get(&key).unwrap().unwrap();
            let restored = InodeValue::deserialize(&loaded).unwrap().to_attr();
            assert_eq!(restored, attr);

            assert_eq!(
                dir_idx.resolve_path(1, "myfile.txt").unwrap(),
                Some(10)
            );
        }
    }

    // -----------------------------------------------------------------------
    // RocksDeltaStore tests
    // -----------------------------------------------------------------------
    mod delta_store {
        use super::*;
        use crate::encoding::encode_inode_key;

        fn new_delta_store() -> (tempfile::TempDir, RocksDeltaStore) {
            let (tmp, db) = temp_db();
            (tmp, RocksDeltaStore::new(db))
        }

        #[test]
        fn append_and_scan_single() {
            let (_tmp, store) = new_delta_store();
            let val = vec![1u8, 0, 0, 0, 1]; // IncrementNlink(1)
            let seqs = store.append_deltas(42, &[val.clone()]).unwrap();
            assert_eq!(seqs, vec![0]);

            let scanned = store.scan_deltas(42).unwrap();
            assert_eq!(scanned.len(), 1);
            assert_eq!(scanned[0], val);
        }

        #[test]
        fn append_and_scan_multiple() {
            let (_tmp, store) = new_delta_store();
            let v1 = vec![1u8, 0, 0, 0, 1]; // IncrementNlink(1)
            let v2 = vec![2u8, 0, 0, 0, 0, 0, 0, 0x07, 0xD0]; // SetMtime(2000)
            let seqs = store.append_deltas(42, &[v1.clone(), v2.clone()]).unwrap();
            assert_eq!(seqs, vec![0, 1]);

            let scanned = store.scan_deltas(42).unwrap();
            assert_eq!(scanned.len(), 2);
            assert_eq!(scanned[0], v1);
            assert_eq!(scanned[1], v2);
        }

        #[test]
        fn scan_returns_empty_for_unknown_inode() {
            let (_tmp, store) = new_delta_store();
            let scanned = store.scan_deltas(999).unwrap();
            assert!(scanned.is_empty());
        }

        #[test]
        fn clear_deltas_removes_all() {
            let (_tmp, store) = new_delta_store();
            let v1 = vec![1u8, 0, 0, 0, 1];
            let v2 = vec![2u8, 0, 0, 0, 0, 0, 0, 0x07, 0xD0];
            store.append_deltas(42, &[v1, v2]).unwrap();
            assert_eq!(store.scan_deltas(42).unwrap().len(), 2);

            store.clear_deltas(42).unwrap();
            assert!(store.scan_deltas(42).unwrap().is_empty());
        }

        #[test]
        fn clear_resets_sequence_counter() {
            let (_tmp, store) = new_delta_store();
            let v = vec![1u8, 0, 0, 0, 1];
            store.append_deltas(42, &[v.clone(), v.clone()]).unwrap();
            store.clear_deltas(42).unwrap();
            // After clear, next append should start from 0 again
            let seqs = store.append_deltas(42, &[v]).unwrap();
            assert_eq!(seqs, vec![0]);
        }

        #[test]
        fn inode_isolation() {
            let (_tmp, store) = new_delta_store();
            let v1 = vec![1u8, 0, 0, 0, 1];
            let v2 = vec![2u8, 0, 0, 0, 0, 0, 0, 0x07, 0xD0];
            store.append_deltas(10, &[v1.clone()]).unwrap();
            store.append_deltas(20, &[v2.clone()]).unwrap();

            assert_eq!(store.scan_deltas(10).unwrap(), vec![v1]);
            assert_eq!(store.scan_deltas(20).unwrap(), vec![v2]);

            // Clear inode 10 should not affect inode 20
            store.clear_deltas(10).unwrap();
            assert!(store.scan_deltas(10).unwrap().is_empty());
            assert_eq!(store.scan_deltas(20).unwrap().len(), 1);
        }

        #[test]
        fn sequential_appends_monotonic_seq() {
            let (_tmp, store) = new_delta_store();
            let v = vec![1u8, 0, 0, 0, 1];
            for i in 0..10u64 {
                let seqs = store.append_deltas(42, &[v.clone()]).unwrap();
                assert_eq!(seqs, vec![i]);
            }
            assert_eq!(store.scan_deltas(42).unwrap().len(), 10);
        }

        #[test]
        fn clear_nonexistent_inode_is_ok() {
            let (_tmp, store) = new_delta_store();
            store.clear_deltas(999).unwrap();
        }

        #[test]
        fn scan_preserves_order() {
            let (_tmp, store) = new_delta_store();
            let values: Vec<Vec<u8>> = (0..5u8)
                .map(|i| vec![1u8, 0, 0, 0, i])
                .collect();
            store.append_deltas(42, &values).unwrap();
            let scanned = store.scan_deltas(42).unwrap();
            assert_eq!(scanned, values);
        }

        #[test]
        fn data_persists_across_reopen() {
            let tmp = tempfile::tempdir().expect("create temp dir");
            let v = vec![1u8, 0, 0, 0, 1];

            // Write
            {
                let db = open_rocks_db(tmp.path()).unwrap();
                let store = RocksDeltaStore::new(db);
                store.append_deltas(42, &[v.clone()]).unwrap();
            }

            // Re-open and verify
            {
                let db = open_rocks_db(tmp.path()).unwrap();
                let store = RocksDeltaStore::new(db);
                let scanned = store.scan_deltas(42).unwrap();
                assert_eq!(scanned, vec![v]);
            }
        }

        #[test]
        fn recover_seqs_on_restart() {
            let tmp = tempfile::tempdir().expect("create temp dir");
            let v = vec![1u8, 0, 0, 0, 1];

            // Write 3 deltas
            {
                let db = open_rocks_db(tmp.path()).unwrap();
                let store = RocksDeltaStore::new(db);
                store
                    .append_deltas(42, &[v.clone(), v.clone(), v.clone()])
                    .unwrap();
            }

            // Re-open, recover, then append — should not collide
            {
                let db = open_rocks_db(tmp.path()).unwrap();
                let store = RocksDeltaStore::new(db);
                store.recover_seqs().unwrap();
                let seqs = store.append_deltas(42, &[v]).unwrap();
                // Previous max seq was 2, so next should be 3
                assert_eq!(seqs, vec![3]);
            }
        }

        #[test]
        fn shared_db_with_metadata_and_dir() {
            let (_tmp, db) = temp_db();
            let meta = RocksMetadataStore::new(Arc::clone(&db));
            let dir_idx = RocksDirectoryIndex::new(Arc::clone(&db));
            let delta = RocksDeltaStore::new(db);

            // All three should work on the same DB
            meta.put(&encode_inode_key(42), b"inode_data").unwrap();
            dir_idx
                .insert_child(
                    1,
                    "test.txt",
                    42,
                    FileAttr {
                        inode: 42,
                        mode: 0o100644,
                        ..Default::default()
                    },
                )
                .unwrap();
            let v = vec![1u8, 0, 0, 0, 1];
            delta.append_deltas(42, &[v.clone()]).unwrap();

            assert!(meta.get(&encode_inode_key(42)).unwrap().is_some());
            assert_eq!(dir_idx.resolve_path(1, "test.txt").unwrap(), Some(42));
            assert_eq!(delta.scan_deltas(42).unwrap(), vec![v]);
        }
    }

    // -----------------------------------------------------------------------
    // AtomicWriteBatch::is_dir_empty — transactional emptiness check
    // -----------------------------------------------------------------------
    mod is_dir_empty {
        use super::*;

        #[test]
        fn empty_dir_returns_true() {
            let (_tmp, db) = temp_db();
            let bundle = RocksStorageBundle::new(Arc::clone(&db));
            let batch = bundle.begin_write();

            // Directory 42 has no children at all.
            assert!(batch.is_dir_empty(42).unwrap());
        }

        #[test]
        fn non_empty_dir_returns_false() {
            let (_tmp, db) = temp_db();
            let dir_idx = RocksDirectoryIndex::new(Arc::clone(&db));
            dir_idx
                .insert_child(
                    42,
                    "child.txt",
                    100,
                    FileAttr {
                        inode: 100,
                        mode: 0o100644,
                        ..Default::default()
                    },
                )
                .unwrap();

            let bundle = RocksStorageBundle::new(Arc::clone(&db));
            let batch = bundle.begin_write();

            assert!(!batch.is_dir_empty(42).unwrap());
        }

        #[test]
        fn sees_uncommitted_writes_in_same_txn() {
            let (_tmp, db) = temp_db();
            let bundle = RocksStorageBundle::new(Arc::clone(&db));
            let mut batch = bundle.begin_write();

            // Directory 42 is empty before the transaction write.
            assert!(batch.is_dir_empty(42).unwrap());

            // Write a dir entry inside the same transaction.
            let key = encode_dir_entry_key(42, "new_child");
            let mut value = Vec::with_capacity(12);
            value.extend_from_slice(&100u64.to_be_bytes());
            value.extend_from_slice(&0o100644u32.to_be_bytes());
            batch.push(BatchOp::PutDirEntry { key, value });

            // The transaction-local iterator should see the uncommitted entry.
            assert!(!batch.is_dir_empty(42).unwrap());
        }

        #[test]
        fn does_not_see_other_parent_entries() {
            let (_tmp, db) = temp_db();
            let dir_idx = RocksDirectoryIndex::new(Arc::clone(&db));
            // Insert a child under parent 10, not parent 42.
            dir_idx
                .insert_child(
                    10,
                    "other.txt",
                    200,
                    FileAttr {
                        inode: 200,
                        mode: 0o100644,
                        ..Default::default()
                    },
                )
                .unwrap();

            let bundle = RocksStorageBundle::new(Arc::clone(&db));
            let batch = bundle.begin_write();

            // Parent 42 should still be empty.
            assert!(batch.is_dir_empty(42).unwrap());
            // Parent 10 should not be empty.
            assert!(!batch.is_dir_empty(10).unwrap());
        }
    }
}
