//! LRU-based inode folded-state cache.
//!
//! Keeps the most-recently-accessed inode values in memory so that
//! `getattr` can be served without scanning delta entries from the store.

use std::num::NonZeroUsize;
use std::sync::Mutex;

use lru::LruCache;
use rucksfs_core::Inode;
use rucksfs_storage::encoding::InodeValue;

use crate::delta::DeltaOp;

/// Thread-safe LRU cache for folded inode values.
///
/// All public methods acquire the internal [`Mutex`].  The cache is designed
/// to be accessed from multiple FUSE / RPC handler threads.
pub struct InodeFoldedCache {
    inner: Mutex<LruCache<Inode, InodeValue>>,
}

impl InodeFoldedCache {
    /// Create a new cache with the given maximum capacity.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "cache capacity must be > 0");
        Self {
            inner: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity).expect("capacity must be > 0"),
            )),
        }
    }

    /// Look up a cached folded inode value.
    ///
    /// If found, the entry is promoted to most-recently-used.
    pub fn get(&self, inode: Inode) -> Option<InodeValue> {
        let mut cache = self.inner.lock().expect("cache lock poisoned");
        cache.get(&inode).cloned()
    }

    /// Insert (or overwrite) a folded inode value.
    ///
    /// If the cache is full, the least-recently-used entry is evicted.
    pub fn put(&self, inode: Inode, value: InodeValue) {
        let mut cache = self.inner.lock().expect("cache lock poisoned");
        cache.put(inode, value);
    }

    /// Apply a single delta operation to a cached entry **in place**.
    ///
    /// If the inode is not in the cache this is a no-op (the caller will
    /// do a full fold on the next read miss).
    pub fn apply_delta(&self, inode: Inode, delta: &DeltaOp) {
        let mut cache = self.inner.lock().expect("cache lock poisoned");
        if let Some(val) = cache.get_mut(&inode) {
            crate::delta::fold_deltas(val, &[delta.clone()]);
        }
    }

    /// Apply multiple delta operations to a cached entry **in place**.
    pub fn apply_deltas(&self, inode: Inode, deltas: &[DeltaOp]) {
        let mut cache = self.inner.lock().expect("cache lock poisoned");
        if let Some(val) = cache.get_mut(&inode) {
            crate::delta::fold_deltas(val, deltas);
        }
    }

    /// Invalidate (remove) a cached entry.
    ///
    /// Called after compaction so the next read re-loads the fresh base.
    pub fn invalidate(&self, inode: Inode) {
        let mut cache = self.inner.lock().expect("cache lock poisoned");
        cache.pop(&inode);
    }

    /// Return the current number of cached entries (for testing).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().expect("cache lock poisoned").len()
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_iv(inode: Inode) -> InodeValue {
        InodeValue {
            version: 1,
            inode,
            size: 0,
            mode: 0o040755,
            nlink: 2,
            uid: 0,
            gid: 0,
            atime: 1000,
            mtime: 1000,
            ctime: 1000,
        }
    }

    #[test]
    fn get_returns_none_for_empty_cache() {
        let cache = InodeFoldedCache::new(10);
        assert!(cache.get(42).is_none());
    }

    #[test]
    fn put_and_get() {
        let cache = InodeFoldedCache::new(10);
        let iv = sample_iv(42);
        cache.put(42, iv.clone());
        assert_eq!(cache.get(42), Some(iv));
    }

    #[test]
    fn put_overwrite() {
        let cache = InodeFoldedCache::new(10);
        let mut iv = sample_iv(42);
        cache.put(42, iv.clone());
        iv.nlink = 5;
        cache.put(42, iv.clone());
        assert_eq!(cache.get(42).unwrap().nlink, 5);
    }

    #[test]
    fn lru_eviction() {
        let cache = InodeFoldedCache::new(3);
        cache.put(1, sample_iv(1));
        cache.put(2, sample_iv(2));
        cache.put(3, sample_iv(3));
        // Cache is full [1, 2, 3].  Adding 4 should evict 1.
        cache.put(4, sample_iv(4));
        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_some());
        assert!(cache.get(3).is_some());
        assert!(cache.get(4).is_some());
    }

    #[test]
    fn access_promotes_to_mru() {
        let cache = InodeFoldedCache::new(3);
        cache.put(1, sample_iv(1));
        cache.put(2, sample_iv(2));
        cache.put(3, sample_iv(3));
        // Access 1 to promote it; LRU is now 2.
        cache.get(1);
        // Insert 4; should evict 2 (not 1).
        cache.put(4, sample_iv(4));
        assert!(cache.get(1).is_some());
        assert!(cache.get(2).is_none());
    }

    #[test]
    fn apply_delta_updates_cached_value() {
        let cache = InodeFoldedCache::new(10);
        cache.put(42, sample_iv(42));
        cache.apply_delta(42, &DeltaOp::IncrementNlink(1));
        assert_eq!(cache.get(42).unwrap().nlink, 3);
    }

    #[test]
    fn apply_delta_noop_on_miss() {
        let cache = InodeFoldedCache::new(10);
        // Should not panic or insert anything.
        cache.apply_delta(42, &DeltaOp::IncrementNlink(1));
        assert!(cache.get(42).is_none());
    }

    #[test]
    fn apply_deltas_multiple() {
        let cache = InodeFoldedCache::new(10);
        cache.put(42, sample_iv(42));
        cache.apply_deltas(
            42,
            &[
                DeltaOp::IncrementNlink(3),
                DeltaOp::SetMtime(2000),
                DeltaOp::SetCtime(2000),
            ],
        );
        let iv = cache.get(42).unwrap();
        assert_eq!(iv.nlink, 5);
        assert_eq!(iv.mtime, 2000);
        assert_eq!(iv.ctime, 2000);
    }

    #[test]
    fn invalidate_removes_entry() {
        let cache = InodeFoldedCache::new(10);
        cache.put(42, sample_iv(42));
        cache.invalidate(42);
        assert!(cache.get(42).is_none());
    }

    #[test]
    fn invalidate_nonexistent_is_noop() {
        let cache = InodeFoldedCache::new(10);
        cache.invalidate(42);
        assert!(cache.get(42).is_none());
    }

    #[test]
    fn len_tracks_entries() {
        let cache = InodeFoldedCache::new(10);
        assert_eq!(cache.len(), 0);
        cache.put(1, sample_iv(1));
        assert_eq!(cache.len(), 1);
        cache.put(2, sample_iv(2));
        assert_eq!(cache.len(), 2);
        cache.invalidate(1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn capacity_one() {
        let cache = InodeFoldedCache::new(1);
        cache.put(1, sample_iv(1));
        cache.put(2, sample_iv(2));
        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_some());
    }

    #[test]
    fn concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(InodeFoldedCache::new(100));
        let mut handles = vec![];

        for i in 0..20u64 {
            let c = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                c.put(i, sample_iv(i));
                c.get(i);
                c.apply_delta(i, &DeltaOp::IncrementNlink(1));
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        for i in 0..20u64 {
            let iv = cache.get(i).unwrap();
            assert_eq!(iv.nlink, 3); // 2 (base) + 1 (delta)
        }
    }
}
