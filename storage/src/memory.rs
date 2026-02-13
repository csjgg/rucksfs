//! In-memory implementations of storage traits for testing and development.
//!
//! These implementations are fully functional and thread-safe (`Send + Sync`),
//! but data is not persisted across process restarts.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use async_trait::async_trait;
use rucksfs_core::{DirEntry, FileAttr, FsError, FsResult, Inode};

use crate::{DataStore, DeltaStore, DirectoryIndex, MetadataStore};

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
// MemoryDeltaStore
// ===========================================================================

/// Thread-safe, in-memory implementation of [`DeltaStore`] for testing.
///
/// Uses a `BTreeMap<(Inode, u64), Vec<u8>>` to store serialized delta ops,
/// and per-inode `AtomicU64` counters for monotonically increasing sequence
/// numbers.
pub struct MemoryDeltaStore {
    /// (inode, seq) → serialized DeltaOp bytes.
    data: RwLock<BTreeMap<(Inode, u64), Vec<u8>>>,
    /// Per-inode next sequence number counter.
    seqs: RwLock<HashMap<Inode, AtomicU64>>,
}

impl MemoryDeltaStore {
    /// Create an empty delta store.
    pub fn new() -> Self {
        Self {
            data: RwLock::new(BTreeMap::new()),
            seqs: RwLock::new(HashMap::new()),
        }
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
        let counter = guard
            .entry(inode)
            .or_insert_with(|| AtomicU64::new(0));
        counter.fetch_add(1, Ordering::Relaxed)
    }
}

impl Default for MemoryDeltaStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DeltaStore for MemoryDeltaStore {
    fn append_deltas(&self, inode: Inode, values: &[Vec<u8>]) -> FsResult<Vec<u64>> {
        let mut data_guard = self.data.write().map_err(|e| {
            FsError::Io(format!("MemoryDeltaStore write lock poisoned: {}", e))
        })?;
        let mut assigned = Vec::with_capacity(values.len());
        for v in values {
            let seq = self.next_seq(inode);
            data_guard.insert((inode, seq), v.clone());
            assigned.push(seq);
        }
        Ok(assigned)
    }

    fn scan_deltas(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>> {
        let guard = self.data.read().map_err(|e| {
            FsError::Io(format!("MemoryDeltaStore read lock poisoned: {}", e))
        })?;
        // BTreeMap is sorted by (inode, seq), so we just range-scan.
        let start = (inode, 0u64);
        let end = (inode, u64::MAX);
        let result: Vec<Vec<u8>> = guard
            .range(start..=end)
            .map(|(_, v)| v.clone())
            .collect();
        Ok(result)
    }

    fn clear_deltas(&self, inode: Inode) -> FsResult<()> {
        let mut data_guard = self.data.write().map_err(|e| {
            FsError::Io(format!("MemoryDeltaStore write lock poisoned: {}", e))
        })?;
        // Collect keys to remove.
        let keys: Vec<(Inode, u64)> = data_guard
            .range((inode, 0u64)..=(inode, u64::MAX))
            .map(|(k, _)| *k)
            .collect();
        for k in keys {
            data_guard.remove(&k);
        }
        // Reset sequence counter.
        if let Ok(mut seq_guard) = self.seqs.write() {
            if let Some(counter) = seq_guard.get_mut(&inode) {
                counter.store(0, Ordering::Relaxed);
            }
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

    // -----------------------------------------------------------------------
    // MemoryDeltaStore tests
    // -----------------------------------------------------------------------
    mod delta_store {
        use super::*;

        #[test]
        fn append_and_scan_single() {
            let store = MemoryDeltaStore::new();
            let val = vec![1u8, 0, 0, 0, 1]; // IncrementNlink(1)
            let seqs = store.append_deltas(42, &[val.clone()]).unwrap();
            assert_eq!(seqs, vec![0]);

            let scanned = store.scan_deltas(42).unwrap();
            assert_eq!(scanned.len(), 1);
            assert_eq!(scanned[0], val);
        }

        #[test]
        fn append_and_scan_multiple() {
            let store = MemoryDeltaStore::new();
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
            let store = MemoryDeltaStore::new();
            let scanned = store.scan_deltas(999).unwrap();
            assert!(scanned.is_empty());
        }

        #[test]
        fn clear_deltas_removes_all() {
            let store = MemoryDeltaStore::new();
            let v1 = vec![1u8, 0, 0, 0, 1];
            let v2 = vec![2u8, 0, 0, 0, 0, 0, 0, 0x07, 0xD0];
            store.append_deltas(42, &[v1, v2]).unwrap();
            assert_eq!(store.scan_deltas(42).unwrap().len(), 2);

            store.clear_deltas(42).unwrap();
            assert!(store.scan_deltas(42).unwrap().is_empty());
        }

        #[test]
        fn clear_resets_sequence_counter() {
            let store = MemoryDeltaStore::new();
            let v = vec![1u8, 0, 0, 0, 1];
            store.append_deltas(42, &[v.clone(), v.clone()]).unwrap();
            // seq should have been 0, 1
            store.clear_deltas(42).unwrap();
            // After clear, next append should start from 0 again
            let seqs = store.append_deltas(42, &[v]).unwrap();
            assert_eq!(seqs, vec![0]);
        }

        #[test]
        fn inode_isolation() {
            let store = MemoryDeltaStore::new();
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
            let store = MemoryDeltaStore::new();
            let v = vec![1u8, 0, 0, 0, 1];
            for i in 0..10u64 {
                let seqs = store.append_deltas(42, &[v.clone()]).unwrap();
                assert_eq!(seqs, vec![i]);
            }
            assert_eq!(store.scan_deltas(42).unwrap().len(), 10);
        }

        #[test]
        fn concurrent_appends_unique_seqs() {
            use std::sync::Arc;
            use std::thread;

            let store = Arc::new(MemoryDeltaStore::new());
            let mut handles = vec![];

            for _ in 0..20 {
                let s = Arc::clone(&store);
                handles.push(thread::spawn(move || {
                    let v = vec![1u8, 0, 0, 0, 1];
                    s.append_deltas(42, &[v]).unwrap()
                }));
            }

            let mut all_seqs: Vec<u64> = vec![];
            for h in handles {
                let seqs = h.join().unwrap();
                all_seqs.extend(seqs);
            }

            // All sequence numbers should be unique
            all_seqs.sort();
            all_seqs.dedup();
            assert_eq!(all_seqs.len(), 20);

            // scan should return all 20
            assert_eq!(store.scan_deltas(42).unwrap().len(), 20);
        }

        #[test]
        fn clear_nonexistent_inode_is_ok() {
            let store = MemoryDeltaStore::new();
            store.clear_deltas(999).unwrap();
        }

        #[test]
        fn scan_preserves_order() {
            let store = MemoryDeltaStore::new();
            let values: Vec<Vec<u8>> = (0..5u8)
                .map(|i| vec![1u8, 0, 0, 0, i])
                .collect();
            store.append_deltas(42, &values).unwrap();
            let scanned = store.scan_deltas(42).unwrap();
            assert_eq!(scanned, values);
        }
    }
}
