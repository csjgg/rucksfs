//! In-memory implementations of storage traits for testing and development.
//!
//! These implementations are fully functional and thread-safe (`Send + Sync`),
//! but data is not persisted across process restarts.

use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use async_trait::async_trait;
use rucksfs_core::{DirEntry, FileAttr, FsError, FsResult, Inode};

use crate::{DataStore, DirectoryIndex, MetadataStore};

// ===========================================================================
// MemoryMetadataStore
// ===========================================================================

/// Thread-safe, sorted in-memory KV store backed by `BTreeMap`.
pub struct MemoryMetadataStore {
    data: RwLock<BTreeMap<Vec<u8>, Vec<u8>>>,
}

impl MemoryMetadataStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            data: RwLock::new(BTreeMap::new()),
        }
    }
}

impl Default for MemoryMetadataStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataStore for MemoryMetadataStore {
    fn get(&self, key: &[u8]) -> FsResult<Option<Vec<u8>>> {
        let guard = self.data.read().map_err(|e| {
            FsError::Io(format!("MemoryMetadataStore read lock poisoned: {}", e))
        })?;
        Ok(guard.get(key).cloned())
    }

    fn put(&self, key: &[u8], value: &[u8]) -> FsResult<()> {
        let mut guard = self.data.write().map_err(|e| {
            FsError::Io(format!("MemoryMetadataStore write lock poisoned: {}", e))
        })?;
        guard.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    fn delete(&self, key: &[u8]) -> FsResult<()> {
        let mut guard = self.data.write().map_err(|e| {
            FsError::Io(format!("MemoryMetadataStore write lock poisoned: {}", e))
        })?;
        guard.remove(key);
        Ok(())
    }

    fn scan_prefix(&self, prefix: &[u8]) -> FsResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let guard = self.data.read().map_err(|e| {
            FsError::Io(format!("MemoryMetadataStore read lock poisoned: {}", e))
        })?;

        // BTreeMap iteration is already sorted by key.
        let result: Vec<(Vec<u8>, Vec<u8>)> = guard
            .range(prefix.to_vec()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Ok(result)
    }
}

// ===========================================================================
// MemoryDirectoryIndex
// ===========================================================================

/// Child entry stored inside a directory.
#[derive(Clone, Debug)]
struct ChildEntry {
    inode: Inode,
    kind: u32,
}

/// Thread-safe in-memory directory index.
///
/// Each directory inode maps to a set of named children.
pub struct MemoryDirectoryIndex {
    /// parent_inode → { child_name → ChildEntry }
    dirs: RwLock<HashMap<Inode, HashMap<String, ChildEntry>>>,
}

impl MemoryDirectoryIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self {
            dirs: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MemoryDirectoryIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectoryIndex for MemoryDirectoryIndex {
    fn resolve_path(&self, parent: Inode, name: &str) -> FsResult<Option<Inode>> {
        let guard = self.dirs.read().map_err(|e| {
            FsError::Io(format!("MemoryDirectoryIndex read lock poisoned: {}", e))
        })?;
        Ok(guard
            .get(&parent)
            .and_then(|children| children.get(name))
            .map(|entry| entry.inode))
    }

    fn list_dir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        let guard = self.dirs.read().map_err(|e| {
            FsError::Io(format!("MemoryDirectoryIndex read lock poisoned: {}", e))
        })?;
        let entries = match guard.get(&inode) {
            Some(children) => children
                .iter()
                .map(|(name, entry)| DirEntry {
                    name: name.clone(),
                    inode: entry.inode,
                    kind: entry.kind,
                })
                .collect(),
            None => Vec::new(),
        };
        Ok(entries)
    }

    fn insert_child(
        &self,
        parent: Inode,
        name: &str,
        inode: Inode,
        attr: FileAttr,
    ) -> FsResult<()> {
        let mut guard = self.dirs.write().map_err(|e| {
            FsError::Io(format!("MemoryDirectoryIndex write lock poisoned: {}", e))
        })?;
        let children = guard.entry(parent).or_default();
        children.insert(
            name.to_string(),
            ChildEntry {
                inode,
                kind: attr.mode,
            },
        );
        Ok(())
    }

    fn remove_child(&self, parent: Inode, name: &str) -> FsResult<()> {
        let mut guard = self.dirs.write().map_err(|e| {
            FsError::Io(format!("MemoryDirectoryIndex write lock poisoned: {}", e))
        })?;
        if let Some(children) = guard.get_mut(&parent) {
            children.remove(name);
        }
        Ok(())
    }
}

// ===========================================================================
// MemoryDataStore
// ===========================================================================

/// Thread-safe in-memory file data store.
///
/// Each inode has its own `Vec<u8>` buffer.  Reads from un-written regions
/// return zero bytes (sparse-file semantics).
pub struct MemoryDataStore {
    files: RwLock<HashMap<Inode, Vec<u8>>>,
}

impl MemoryDataStore {
    /// Create an empty data store.
    pub fn new() -> Self {
        Self {
            files: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MemoryDataStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DataStore for MemoryDataStore {
    async fn read_at(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        let guard = self.files.read().map_err(|e| {
            FsError::Io(format!("MemoryDataStore read lock poisoned: {}", e))
        })?;

        let offset = offset as usize;
        let size = size as usize;

        match guard.get(&inode) {
            Some(buf) => {
                if offset >= buf.len() {
                    // Past end of file → return zeros
                    Ok(vec![0u8; size])
                } else {
                    let end = (offset + size).min(buf.len());
                    let mut result = buf[offset..end].to_vec();
                    // Pad with zeros if read extends past stored data
                    if result.len() < size {
                        result.resize(size, 0);
                    }
                    Ok(result)
                }
            }
            // Inode has no data yet → return zeros
            None => Ok(vec![0u8; size]),
        }
    }

    async fn write_at(&self, inode: Inode, offset: u64, data: &[u8]) -> FsResult<u32> {
        let mut guard = self.files.write().map_err(|e| {
            FsError::Io(format!("MemoryDataStore write lock poisoned: {}", e))
        })?;

        let offset = offset as usize;
        let buf = guard.entry(inode).or_default();

        let required_len = offset + data.len();
        if buf.len() < required_len {
            buf.resize(required_len, 0);
        }
        buf[offset..offset + data.len()].copy_from_slice(data);

        Ok(data.len() as u32)
    }

    async fn truncate(&self, inode: Inode, size: u64) -> FsResult<()> {
        let mut guard = self.files.write().map_err(|e| {
            FsError::Io(format!("MemoryDataStore write lock poisoned: {}", e))
        })?;

        let size = size as usize;
        let buf = guard.entry(inode).or_default();

        if size < buf.len() {
            buf.truncate(size);
        } else {
            buf.resize(size, 0);
        }

        Ok(())
    }

    async fn flush(&self, _inode: Inode) -> FsResult<()> {
        // In-memory store: nothing to flush.
        Ok(())
    }
}

// ===========================================================================
// Tests
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // MemoryMetadataStore tests
    // -----------------------------------------------------------------------
    mod metadata_store {
        use super::*;

        #[test]
        fn basic_put_get() {
            let store = MemoryMetadataStore::new();
            store.put(b"k1", b"v1").unwrap();
            assert_eq!(store.get(b"k1").unwrap(), Some(b"v1".to_vec()));
        }

        #[test]
        fn get_missing_key() {
            let store = MemoryMetadataStore::new();
            assert_eq!(store.get(b"nope").unwrap(), None);
        }

        #[test]
        fn put_overwrite() {
            let store = MemoryMetadataStore::new();
            store.put(b"k", b"v1").unwrap();
            store.put(b"k", b"v2").unwrap();
            assert_eq!(store.get(b"k").unwrap(), Some(b"v2".to_vec()));
        }

        #[test]
        fn delete_existing() {
            let store = MemoryMetadataStore::new();
            store.put(b"k", b"v").unwrap();
            store.delete(b"k").unwrap();
            assert_eq!(store.get(b"k").unwrap(), None);
        }

        #[test]
        fn delete_idempotent() {
            let store = MemoryMetadataStore::new();
            // Delete non-existent key should succeed
            store.delete(b"ghost").unwrap();
        }

        #[test]
        fn scan_prefix_basic() {
            let store = MemoryMetadataStore::new();
            store.put(b"dir/a", b"1").unwrap();
            store.put(b"dir/b", b"2").unwrap();
            store.put(b"dir/c", b"3").unwrap();
            store.put(b"other/x", b"4").unwrap();

            let result = store.scan_prefix(b"dir/").unwrap();
            assert_eq!(result.len(), 3);
            // Should be sorted by key
            assert_eq!(result[0].0, b"dir/a");
            assert_eq!(result[1].0, b"dir/b");
            assert_eq!(result[2].0, b"dir/c");
        }

        #[test]
        fn scan_prefix_empty() {
            let store = MemoryMetadataStore::new();
            store.put(b"other", b"v").unwrap();
            let result = store.scan_prefix(b"nope").unwrap();
            assert!(result.is_empty());
        }

        #[test]
        fn concurrent_read_write() {
            use std::sync::Arc;
            use std::thread;

            let store = Arc::new(MemoryMetadataStore::new());
            let mut handles = vec![];

            // Writers
            for i in 0..10u8 {
                let s = Arc::clone(&store);
                handles.push(thread::spawn(move || {
                    s.put(&[i], &[i]).unwrap();
                }));
            }

            for h in handles {
                h.join().unwrap();
            }

            // Verify
            for i in 0..10u8 {
                assert_eq!(store.get(&[i]).unwrap(), Some(vec![i]));
            }
        }
    }

    // -----------------------------------------------------------------------
    // MemoryDirectoryIndex tests
    // -----------------------------------------------------------------------
    mod directory_index {
        use super::*;

        fn make_attr(inode: Inode, mode: u32) -> FileAttr {
            FileAttr {
                inode,
                mode,
                ..Default::default()
            }
        }

        #[test]
        fn insert_and_resolve() {
            let idx = MemoryDirectoryIndex::new();
            idx.insert_child(1, "hello.txt", 10, make_attr(10, 0o100644))
                .unwrap();
            assert_eq!(idx.resolve_path(1, "hello.txt").unwrap(), Some(10));
        }

        #[test]
        fn resolve_missing() {
            let idx = MemoryDirectoryIndex::new();
            assert_eq!(idx.resolve_path(1, "nope").unwrap(), None);
        }

        #[test]
        fn list_dir_entries() {
            let idx = MemoryDirectoryIndex::new();
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
            let idx = MemoryDirectoryIndex::new();
            let entries = idx.list_dir(999).unwrap();
            assert!(entries.is_empty());
        }

        #[test]
        fn remove_child_and_resolve() {
            let idx = MemoryDirectoryIndex::new();
            idx.insert_child(1, "f", 10, make_attr(10, 0o100644))
                .unwrap();
            idx.remove_child(1, "f").unwrap();
            assert_eq!(idx.resolve_path(1, "f").unwrap(), None);
        }

        #[test]
        fn remove_nonexistent_is_ok() {
            let idx = MemoryDirectoryIndex::new();
            // Should not error
            idx.remove_child(1, "ghost").unwrap();
        }

        #[test]
        fn overwrite_child() {
            let idx = MemoryDirectoryIndex::new();
            idx.insert_child(1, "f", 10, make_attr(10, 0o100644))
                .unwrap();
            idx.insert_child(1, "f", 20, make_attr(20, 0o100644))
                .unwrap();
            assert_eq!(idx.resolve_path(1, "f").unwrap(), Some(20));
        }

        #[test]
        fn concurrent_insert_remove() {
            use std::sync::Arc;
            use std::thread;

            let idx = Arc::new(MemoryDirectoryIndex::new());
            let mut handles = vec![];

            for i in 0..20u64 {
                let idx_c = Arc::clone(&idx);
                handles.push(thread::spawn(move || {
                    let name = format!("f{}", i);
                    idx_c
                        .insert_child(1, &name, i + 100, make_attr(i + 100, 0o100644))
                        .unwrap();
                }));
            }

            for h in handles {
                h.join().unwrap();
            }

            let entries = idx.list_dir(1).unwrap();
            assert_eq!(entries.len(), 20);
        }
    }

    // -----------------------------------------------------------------------
    // MemoryDataStore tests
    // -----------------------------------------------------------------------
    mod data_store {
        use super::*;

        fn rt() -> tokio::runtime::Runtime {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        }

        #[test]
        fn write_then_read() {
            rt().block_on(async {
                let ds = MemoryDataStore::new();
                let written = ds.write_at(1, 0, b"hello").await.unwrap();
                assert_eq!(written, 5);
                let data = ds.read_at(1, 0, 5).await.unwrap();
                assert_eq!(data, b"hello");
            });
        }

        #[test]
        fn read_unwritten_returns_zeros() {
            rt().block_on(async {
                let ds = MemoryDataStore::new();
                let data = ds.read_at(1, 0, 10).await.unwrap();
                assert_eq!(data, vec![0u8; 10]);
            });
        }

        #[test]
        fn sparse_read_past_end() {
            rt().block_on(async {
                let ds = MemoryDataStore::new();
                ds.write_at(1, 0, b"abc").await.unwrap();
                // Read starting at offset 1, requesting 5 bytes
                let data = ds.read_at(1, 1, 5).await.unwrap();
                // Should be [b, c, 0, 0, 0]
                assert_eq!(data, vec![b'b', b'c', 0, 0, 0]);
            });
        }

        #[test]
        fn write_at_offset() {
            rt().block_on(async {
                let ds = MemoryDataStore::new();
                ds.write_at(1, 5, b"world").await.unwrap();
                let data = ds.read_at(1, 0, 10).await.unwrap();
                assert_eq!(&data[..5], &[0, 0, 0, 0, 0]);
                assert_eq!(&data[5..10], b"world");
            });
        }

        #[test]
        fn truncate_shrink() {
            rt().block_on(async {
                let ds = MemoryDataStore::new();
                ds.write_at(1, 0, b"hello world").await.unwrap();
                ds.truncate(1, 5).await.unwrap();
                let data = ds.read_at(1, 0, 11).await.unwrap();
                assert_eq!(&data[..5], b"hello");
                assert_eq!(&data[5..], &[0; 6]);
            });
        }

        #[test]
        fn truncate_expand() {
            rt().block_on(async {
                let ds = MemoryDataStore::new();
                ds.write_at(1, 0, b"hi").await.unwrap();
                ds.truncate(1, 10).await.unwrap();
                let data = ds.read_at(1, 0, 10).await.unwrap();
                assert_eq!(&data[..2], b"hi");
                assert_eq!(&data[2..], &[0; 8]);
            });
        }

        #[test]
        fn cross_inode_isolation() {
            rt().block_on(async {
                let ds = MemoryDataStore::new();
                ds.write_at(1, 0, b"inode1").await.unwrap();
                ds.write_at(2, 0, b"inode2").await.unwrap();
                assert_eq!(ds.read_at(1, 0, 6).await.unwrap(), b"inode1");
                assert_eq!(ds.read_at(2, 0, 6).await.unwrap(), b"inode2");
            });
        }

        #[test]
        fn flush_is_noop() {
            rt().block_on(async {
                let ds = MemoryDataStore::new();
                ds.flush(1).await.unwrap();
                ds.flush(u64::MAX).await.unwrap();
            });
        }
    }
}
