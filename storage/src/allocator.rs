//! Atomic inode allocator with optional persistence.
//!
//! Inodes 0 and 1 are reserved (0 = invalid, 1 = root directory).
//! Allocation starts from inode 2.

use std::sync::atomic::{AtomicU64, Ordering};

use rucksfs_core::{FsError, FsResult, Inode};

use crate::MetadataStore;

/// Well-known key used to persist the next-inode counter.
const NEXT_INODE_KEY: &[u8] = b"next_inode";

/// Root directory inode (always 1).
pub const ROOT_INODE: Inode = 1;

/// First allocatable inode (0 and 1 are reserved).
const FIRST_ALLOC_INODE: u64 = 2;

/// Thread-safe inode allocator backed by an `AtomicU64`.
pub struct InodeAllocator {
    next: AtomicU64,
}

impl InodeAllocator {
    /// Create a fresh allocator starting at inode 2.
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(FIRST_ALLOC_INODE),
        }
    }

    /// Allocate the next unique inode ID.
    ///
    /// This is lock-free and safe to call from multiple threads concurrently.
    pub fn alloc(&self) -> Inode {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// Return the current counter value (next inode that *would* be allocated).
    pub fn current(&self) -> u64 {
        self.next.load(Ordering::Relaxed)
    }

    /// Persist the current counter value into a [`MetadataStore`].
    pub fn persist(&self, store: &dyn MetadataStore) -> FsResult<()> {
        let val = self.next.load(Ordering::Relaxed);
        store.put(NEXT_INODE_KEY, &val.to_be_bytes())
    }

    /// Restore the allocator state from a [`MetadataStore`].
    ///
    /// If no persisted value exists, the allocator starts from [`FIRST_ALLOC_INODE`].
    pub fn load(store: &dyn MetadataStore) -> FsResult<Self> {
        match store.get(NEXT_INODE_KEY)? {
            Some(bytes) => {
                if bytes.len() != 8 {
                    return Err(FsError::InvalidInput(format!(
                        "InodeAllocator: expected 8 bytes, got {}",
                        bytes.len()
                    )));
                }
                let val = u64::from_be_bytes(bytes.try_into().unwrap());
                Ok(Self {
                    next: AtomicU64::new(val),
                })
            }
            None => Ok(Self::new()),
        }
    }
}

impl Default for InodeAllocator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rocks::{open_rocks_db, RocksMetadataStore};
    use std::sync::Arc;

    #[test]
    fn initial_value() {
        let alloc = InodeAllocator::new();
        assert_eq!(alloc.current(), FIRST_ALLOC_INODE);
    }

    #[test]
    fn sequential_alloc() {
        let alloc = InodeAllocator::new();
        assert_eq!(alloc.alloc(), 2);
        assert_eq!(alloc.alloc(), 3);
        assert_eq!(alloc.alloc(), 4);
    }

    #[test]
    fn concurrent_alloc_unique() {
        use std::collections::HashSet;
        use std::thread;

        let alloc = Arc::new(InodeAllocator::new());
        let n = 1000;
        let mut handles = vec![];

        for _ in 0..n {
            let a = Arc::clone(&alloc);
            handles.push(thread::spawn(move || a.alloc()));
        }

        let inodes: HashSet<Inode> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(inodes.len(), n, "all allocated inodes should be unique");
    }

    #[test]
    fn persist_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let db = open_rocks_db(tmp.path().join("meta.db")).unwrap();
        let store = RocksMetadataStore::new(Arc::clone(&db));

        let alloc = InodeAllocator::new();
        alloc.alloc(); // 2
        alloc.alloc(); // 3
        alloc.alloc(); // 4 → next = 5
        alloc.persist(&store).unwrap();

        let restored = InodeAllocator::load(&store).unwrap();
        assert_eq!(restored.current(), 5);
        assert_eq!(restored.alloc(), 5);
    }

    #[test]
    fn load_without_persisted_value() {
        let tmp = tempfile::tempdir().unwrap();
        let db = open_rocks_db(tmp.path().join("meta.db")).unwrap();
        let store = RocksMetadataStore::new(db);

        let restored = InodeAllocator::load(&store).unwrap();
        assert_eq!(restored.current(), FIRST_ALLOC_INODE);
    }

    #[test]
    fn load_corrupted_value() {
        let tmp = tempfile::tempdir().unwrap();
        let db = open_rocks_db(tmp.path().join("meta.db")).unwrap();
        let store = RocksMetadataStore::new(db);

        store.put(NEXT_INODE_KEY, &[1, 2, 3]).unwrap(); // wrong length
        let result = InodeAllocator::load(&store);
        assert!(result.is_err());
    }
}
