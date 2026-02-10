//! RocksDB-backed implementations of [`MetadataStore`] and [`DirectoryIndex`].
//!
//! This module is only compiled when the `rocksdb` cargo feature is enabled.
//! It provides persistent storage using three RocksDB Column Families:
//!
//! - **inodes**: inode metadata (key = encoded inode key, value = serialized `InodeValue`)
//! - **dir_entries**: directory entries (key = encoded dir entry key, value = child inode as u64 BE)
//! - **system**: system-level KV pairs (e.g. next inode counter)

use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use std::path::Path;
use std::sync::Arc;

use rucksfs_core::{DirEntry, FileAttr, FsError, FsResult, Inode};

use crate::encoding::{
    dir_entry_prefix, encode_dir_entry_key, encode_inode_key, extract_child_name, InodeValue,
};
use crate::{DirectoryIndex, MetadataStore};

/// Column family name for inode metadata.
const CF_INODES: &str = "inodes";
/// Column family name for directory entries.
const CF_DIR_ENTRIES: &str = "dir_entries";
/// Column family name for system-level data (e.g. allocator state).
const CF_SYSTEM: &str = "system";

/// All column family names used by the storage layer.
const ALL_CFS: &[&str] = &[CF_INODES, CF_DIR_ENTRIES, CF_SYSTEM];

/// Open (or create) a RocksDB database with the required column families.
///
/// This is a shared helper so that both `RocksMetadataStore` and
/// `RocksDirectoryIndex` can be created from the same `Arc<DB>`.
pub fn open_rocks_db(path: impl AsRef<Path>) -> FsResult<Arc<DB>> {
    let mut db_opts = Options::default();
    db_opts.create_if_missing(true);
    db_opts.create_missing_column_families(true);

    let cf_descriptors: Vec<ColumnFamilyDescriptor> = ALL_CFS
        .iter()
        .map(|name| ColumnFamilyDescriptor::new(*name, Options::default()))
        .collect();

    let db = DB::open_cf_descriptors(&db_opts, path.as_ref(), cf_descriptors)
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
    db: Arc<DB>,
}

impl RocksMetadataStore {
    /// Create a new store from a shared DB handle.
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Convenience: open a new DB at `path` and return the store.
    pub fn open(path: impl AsRef<Path>) -> FsResult<Self> {
        let db = open_rocks_db(path)?;
        Ok(Self::new(db))
    }

    /// Get a reference to the underlying DB (useful for sharing with
    /// `RocksDirectoryIndex`).
    pub fn db(&self) -> &Arc<DB> {
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
    db: Arc<DB>,
}

/// Serialized size of a directory entry value: inode(8) + mode(4) = 12 bytes.
const DIR_VALUE_SIZE: usize = 12;

impl RocksDirectoryIndex {
    /// Create a new directory index from a shared DB handle.
    pub fn new(db: Arc<DB>) -> Self {
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
// Tests
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temporary RocksDB and return the shared DB handle.
    fn temp_db() -> (tempfile::TempDir, Arc<DB>) {
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
        use crate::encoding::InodeValue;

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
}
