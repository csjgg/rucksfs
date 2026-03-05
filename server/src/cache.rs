//! Sharded LRU-based inode folded-state cache.
//!
//! Keeps the most-recently-accessed inode values in memory so that
//! `getattr` can be served without scanning delta entries from the store.
//!
//! Uses 16 shards with `parking_lot::RwLock` to reduce contention when
//! accessed from multiple FUSE / RPC handler threads.

use std::num::NonZeroUsize;

use lru::LruCache;
use parking_lot::Mutex;
use rucksfs_core::Inode;
use rucksfs_storage::encoding::InodeValue;

use crate::delta::DeltaOp;

const NUM_SHARDS: usize = 16;

/// Thread-safe sharded LRU cache for folded inode values.
///
/// Each shard is independently locked, reducing contention to 1/16th of a
/// single-mutex design.  Sequential inode numbers are distributed across
/// shards via Fibonacci hashing.
pub struct InodeFoldedCache {
    shards: Vec<Mutex<LruCache<Inode, InodeValue>>>,
}

impl InodeFoldedCache {
    /// Create a new cache with the given maximum total capacity.
    ///
    /// The capacity is divided evenly among shards (minimum 1 per shard).
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "cache capacity must be > 0");
        let per_shard = (capacity / NUM_SHARDS).max(1);
        let shards = (0..NUM_SHARDS)
            .map(|_| {
                Mutex::new(LruCache::new(
                    NonZeroUsize::new(per_shard).expect("per_shard must be > 0"),
                ))
            })
            .collect();
        Self { shards }
    }

    /// Fibonacci-hash an inode to a shard index (0..NUM_SHARDS).
    #[inline]
    fn shard_index(inode: Inode) -> usize {
        let hash = inode.wrapping_mul(0x9E3779B97F4A7C15);
        (hash >> 60) as usize
    }

    /// Look up a cached folded inode value.
    ///
    /// If found, the entry is promoted to most-recently-used.
    pub fn get(&self, inode: Inode) -> Option<InodeValue> {
        let idx = Self::shard_index(inode);
        let mut shard = self.shards[idx].lock();
        shard.get(&inode).cloned()
    }

    /// Insert (or overwrite) a folded inode value.
    ///
    /// If the shard is full, the least-recently-used entry is evicted.
    pub fn put(&self, inode: Inode, value: InodeValue) {
        let idx = Self::shard_index(inode);
        let mut shard = self.shards[idx].lock();
        shard.put(inode, value);
    }

    /// Apply a single delta operation to a cached entry **in place**.
    ///
    /// If the inode is not in the cache this is a no-op (the caller will
    /// do a full fold on the next read miss).
    pub fn apply_delta(&self, inode: Inode, delta: &DeltaOp) {
        let idx = Self::shard_index(inode);
        let mut shard = self.shards[idx].lock();
        if let Some(val) = shard.get_mut(&inode) {
            crate::delta::fold_deltas(val, std::slice::from_ref(delta));
        }
    }

    /// Apply multiple delta operations to a cached entry **in place**.
    pub fn apply_deltas(&self, inode: Inode, deltas: &[DeltaOp]) {
        let idx = Self::shard_index(inode);
        let mut shard = self.shards[idx].lock();
        if let Some(val) = shard.get_mut(&inode) {
            crate::delta::fold_deltas(val, deltas);
        }
    }

    /// Invalidate (remove) a cached entry.
    ///
    /// Called after compaction so the next read re-loads the fresh base.
    pub fn invalidate(&self, inode: Inode) {
        let idx = Self::shard_index(inode);
        let mut shard = self.shards[idx].lock();
        shard.pop(&inode);
    }

    /// Return the current number of cached entries across all shards (for testing).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.lock().len()).sum()
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
        // Use capacity=16 so each shard gets capacity=1, making eviction testable.
        // We need to find inodes that land in the same shard.
        let cache = InodeFoldedCache::new(16);
        // Find 3 inodes that hash to the same shard.
        let shard_0_inodes: Vec<u64> = (1..1000)
            .filter(|&i| InodeFoldedCache::shard_index(i) == InodeFoldedCache::shard_index(1))
            .take(3)
            .collect();
        assert!(
            shard_0_inodes.len() >= 3,
            "Need 3 inodes in same shard for test"
        );
        let (a, b, c) = (shard_0_inodes[0], shard_0_inodes[1], shard_0_inodes[2]);
        cache.put(a, sample_iv(a));
        // Adding b should evict a (per-shard capacity=1).
        cache.put(b, sample_iv(b));
        assert!(cache.get(a).is_none());
        assert!(cache.get(b).is_some());
        // Adding c should evict b.
        cache.put(c, sample_iv(c));
        assert!(cache.get(b).is_none());
        assert!(cache.get(c).is_some());
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
        let cache = InodeFoldedCache::new(100);
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
        // With capacity=1, per-shard capacity is 1.
        // Find two inodes in the same shard.
        let shard_0_inodes: Vec<u64> = (1..1000)
            .filter(|&i| InodeFoldedCache::shard_index(i) == InodeFoldedCache::shard_index(1))
            .take(2)
            .collect();
        let (a, b) = (shard_0_inodes[0], shard_0_inodes[1]);
        cache.put(a, sample_iv(a));
        cache.put(b, sample_iv(b));
        assert!(cache.get(a).is_none());
        assert!(cache.get(b).is_some());
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

    #[test]
    fn shard_distribution() {
        // Verify sequential inodes are distributed across multiple shards.
        let mut seen_shards = std::collections::HashSet::new();
        for i in 1..=64u64 {
            seen_shards.insert(InodeFoldedCache::shard_index(i));
        }
        // With Fibonacci hashing, 64 sequential inodes should hit all 16 shards.
        assert_eq!(seen_shards.len(), NUM_SHARDS);
    }
}
