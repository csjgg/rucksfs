//! Background delta compaction worker.
//!
//! Periodically (or on demand) merges pending delta entries into the base
//! inode value stored in the metadata store, then clears the deltas.  This
//! keeps read amplification bounded and prevents unbounded delta growth.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rucksfs_core::{FsResult, Inode};
use rucksfs_storage::encoding::{encode_inode_key, InodeValue};
use rucksfs_storage::{DeltaStore, MetadataStore};

use crate::cache::InodeFoldedCache;
use crate::delta::{self, DeltaOp};

/// Default compaction check interval in milliseconds.
const DEFAULT_INTERVAL_MS: u64 = 5_000;

/// Default: compact an inode once it has accumulated this many deltas.
const DEFAULT_DELTA_THRESHOLD: usize = 32;

/// Configuration for the compaction worker.
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// How often (in ms) the worker scans for dirty inodes.
    pub interval_ms: u64,
    /// Number of pending deltas before an inode is eligible for compaction.
    pub delta_threshold: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            interval_ms: DEFAULT_INTERVAL_MS,
            delta_threshold: DEFAULT_DELTA_THRESHOLD,
        }
    }
}

/// Background worker that compacts delta entries into base inode values.
///
/// The worker maintains a **dirty set** of inodes that have received deltas
/// since the last compaction round.  Writers register inodes via
/// [`mark_dirty`].
pub struct DeltaCompactionWorker<M, DS>
where
    M: MetadataStore,
    DS: DeltaStore,
{
    metadata: Arc<M>,
    delta_store: Arc<DS>,
    cache: Arc<InodeFoldedCache>,
    config: CompactionConfig,
    /// Set of inodes that have pending deltas.
    dirty: Mutex<HashSet<Inode>>,
    /// Flag to stop the background loop.
    running: AtomicBool,
}

impl<M, DS> DeltaCompactionWorker<M, DS>
where
    M: MetadataStore,
    DS: DeltaStore,
{
    /// Create a new compaction worker.
    pub fn new(
        metadata: Arc<M>,
        delta_store: Arc<DS>,
        cache: Arc<InodeFoldedCache>,
        config: CompactionConfig,
    ) -> Self {
        Self {
            metadata,
            delta_store,
            cache,
            config,
            dirty: Mutex::new(HashSet::new()),
            running: AtomicBool::new(false),
        }
    }

    /// Mark an inode as dirty (has pending deltas that may need compaction).
    ///
    /// This is called by the write path after appending deltas.
    pub fn mark_dirty(&self, inode: Inode) {
        if let Ok(mut set) = self.dirty.lock() {
            set.insert(inode);
        }
    }

    /// Signal the background loop to stop.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Check whether the worker loop is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    // -----------------------------------------------------------------------
    // Core compaction logic
    // -----------------------------------------------------------------------

    /// Compact a single inode: read base, fold deltas, write back, clear.
    ///
    /// Returns `true` if compaction was performed, `false` if there were
    /// no deltas to compact.
    pub fn compact_inode(&self, inode: Inode) -> FsResult<bool> {
        // 1. Scan deltas.
        let raw_deltas = self.delta_store.scan_deltas(inode)?;
        if raw_deltas.is_empty() {
            return Ok(false);
        }

        // Only compact if threshold is met (or force).
        if raw_deltas.len() < self.config.delta_threshold {
            return Ok(false);
        }

        self.force_compact_inode(inode)
    }

    /// Compact a single inode regardless of threshold.
    pub fn force_compact_inode(&self, inode: Inode) -> FsResult<bool> {
        // 1. Scan deltas.
        let raw_deltas = self.delta_store.scan_deltas(inode)?;
        if raw_deltas.is_empty() {
            return Ok(false);
        }

        // 2. Read base.
        let key = encode_inode_key(inode);
        let mut base = match self.metadata.get(&key)? {
            Some(bytes) => InodeValue::deserialize(&bytes)?,
            None => return Ok(false), // inode was deleted
        };

        // 3. Fold.
        let ops: Vec<DeltaOp> = raw_deltas
            .iter()
            .filter_map(|bytes| DeltaOp::deserialize(bytes).ok())
            .collect();
        delta::fold_deltas(&mut base, &ops);

        // 4. Write back the new base.
        self.metadata.put(&key, &base.serialize())?;

        // 5. Clear compacted deltas.
        self.delta_store.clear_deltas(inode)?;

        // 6. Invalidate cache so the next read picks up the fresh base.
        self.cache.invalidate(inode);

        Ok(true)
    }

    /// Compact all currently-dirty inodes that exceed the threshold.
    ///
    /// Returns the number of inodes that were actually compacted.
    pub fn compact_dirty(&self) -> FsResult<usize> {
        // Swap out the dirty set atomically.
        let inodes: Vec<Inode> = {
            let mut set = self.dirty.lock().expect("dirty lock poisoned");
            let v: Vec<Inode> = set.drain().collect();
            v
        };

        let mut compacted = 0;
        for inode in inodes {
            match self.compact_inode(inode) {
                Ok(true) => compacted += 1,
                Ok(false) => {
                    // Below threshold: re-mark as dirty for next round.
                    self.mark_dirty(inode);
                }
                Err(e) => {
                    // Re-mark and log.
                    self.mark_dirty(inode);
                    tracing::warn!(inode, error = %e, "delta compaction failed");
                }
            }
        }
        Ok(compacted)
    }

    /// Force-compact **all** dirty inodes regardless of threshold.
    ///
    /// Useful in tests and during shutdown.
    pub fn flush_all(&self) -> FsResult<usize> {
        let inodes: Vec<Inode> = {
            let mut set = self.dirty.lock().expect("dirty lock poisoned");
            set.drain().collect()
        };

        let mut compacted = 0;
        for inode in inodes {
            match self.force_compact_inode(inode) {
                Ok(true) => compacted += 1,
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(inode, error = %e, "delta compaction (flush) failed");
                }
            }
        }
        Ok(compacted)
    }

    /// Run the compaction loop in the current thread (blocking).
    ///
    /// Typically called via `std::thread::spawn`.  The loop runs until
    /// [`stop`] is called.
    pub fn run_loop(&self) {
        self.running.store(true, Ordering::Relaxed);
        let interval = std::time::Duration::from_millis(self.config.interval_ms);

        while self.running.load(Ordering::Relaxed) {
            std::thread::sleep(interval);
            if let Err(e) = self.compact_dirty() {
                tracing::error!(error = %e, "compaction round failed");
            }
        }

        // Final flush on shutdown.
        let _ = self.flush_all();
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use rucksfs_storage::{MemoryDeltaStore, MemoryMetadataStore};

    fn make_worker(
        threshold: usize,
    ) -> (
        Arc<MemoryMetadataStore>,
        Arc<MemoryDeltaStore>,
        Arc<InodeFoldedCache>,
        DeltaCompactionWorker<MemoryMetadataStore, MemoryDeltaStore>,
    ) {
        let meta = Arc::new(MemoryMetadataStore::new());
        let ds = Arc::new(MemoryDeltaStore::new());
        let cache = Arc::new(InodeFoldedCache::new(100));
        let config = CompactionConfig {
            delta_threshold: threshold,
            interval_ms: 50,
        };
        let worker = DeltaCompactionWorker::new(
            Arc::clone(&meta),
            Arc::clone(&ds),
            Arc::clone(&cache),
            config,
        );
        (meta, ds, cache, worker)
    }

    fn write_base(meta: &MemoryMetadataStore, inode: Inode, nlink: u32) {
        let iv = InodeValue {
            version: 1,
            inode,
            size: 0,
            mode: 0o040755,
            nlink,
            uid: 0,
            gid: 0,
            atime: 1000,
            mtime: 1000,
            ctime: 1000,
        };
        meta.put(&encode_inode_key(inode), &iv.serialize()).unwrap();
    }

    fn append_nlink_delta(ds: &MemoryDeltaStore, inode: Inode, amount: i32) {
        let op = DeltaOp::IncrementNlink(amount);
        ds.append_deltas(inode, &[op.serialize()]).unwrap();
    }

    fn read_base(meta: &MemoryMetadataStore, inode: Inode) -> InodeValue {
        let bytes = meta.get(&encode_inode_key(inode)).unwrap().unwrap();
        InodeValue::deserialize(&bytes).unwrap()
    }

    #[test]
    fn compact_inode_below_threshold_skips() {
        let (meta, ds, _cache, worker) = make_worker(5);
        write_base(&meta, 42, 2);
        // Append 3 deltas (below threshold of 5)
        for _ in 0..3 {
            append_nlink_delta(&ds, 42, 1);
        }
        let result = worker.compact_inode(42).unwrap();
        assert!(!result); // Not compacted
        // Deltas should still be there
        assert_eq!(ds.scan_deltas(42).unwrap().len(), 3);
    }

    #[test]
    fn compact_inode_at_threshold_compacts() {
        let (meta, ds, _cache, worker) = make_worker(5);
        write_base(&meta, 42, 2);
        // Append exactly 5 deltas
        for _ in 0..5 {
            append_nlink_delta(&ds, 42, 1);
        }
        let result = worker.compact_inode(42).unwrap();
        assert!(result);

        // Base should be updated
        let iv = read_base(&meta, 42);
        assert_eq!(iv.nlink, 7); // 2 + 5

        // Deltas should be cleared
        assert!(ds.scan_deltas(42).unwrap().is_empty());
    }

    #[test]
    fn force_compact_ignores_threshold() {
        let (meta, ds, _cache, worker) = make_worker(100);
        write_base(&meta, 42, 2);
        append_nlink_delta(&ds, 42, 1);
        // Only 1 delta, threshold is 100
        let result = worker.force_compact_inode(42).unwrap();
        assert!(result);
        assert_eq!(read_base(&meta, 42).nlink, 3);
        assert!(ds.scan_deltas(42).unwrap().is_empty());
    }

    #[test]
    fn compact_nonexistent_inode_returns_false() {
        let (_meta, _ds, _cache, worker) = make_worker(1);
        let result = worker.force_compact_inode(999).unwrap();
        assert!(!result);
    }

    #[test]
    fn compact_no_deltas_returns_false() {
        let (meta, _ds, _cache, worker) = make_worker(1);
        write_base(&meta, 42, 2);
        let result = worker.force_compact_inode(42).unwrap();
        assert!(!result);
    }

    #[test]
    fn compact_dirty_batch() {
        let (meta, ds, _cache, worker) = make_worker(2);
        // Setup: 3 inodes, each with 2+ deltas
        for inode in [10, 20, 30] {
            write_base(&meta, inode, 1);
            for _ in 0..3 {
                append_nlink_delta(&ds, inode, 1);
            }
            worker.mark_dirty(inode);
        }

        let compacted = worker.compact_dirty().unwrap();
        assert_eq!(compacted, 3);

        // All bases should be updated
        for inode in [10, 20, 30] {
            assert_eq!(read_base(&meta, inode).nlink, 4); // 1 + 3
            assert!(ds.scan_deltas(inode).unwrap().is_empty());
        }
    }

    #[test]
    fn compact_dirty_re_marks_below_threshold() {
        let (meta, ds, _cache, worker) = make_worker(10);
        write_base(&meta, 42, 2);
        append_nlink_delta(&ds, 42, 1); // 1 delta, threshold 10
        worker.mark_dirty(42);

        let compacted = worker.compact_dirty().unwrap();
        assert_eq!(compacted, 0);

        // Should be re-marked as dirty
        let dirty = worker.dirty.lock().unwrap();
        assert!(dirty.contains(&42));
    }

    #[test]
    fn flush_all_forces_all() {
        let (meta, ds, _cache, worker) = make_worker(100);
        write_base(&meta, 42, 2);
        append_nlink_delta(&ds, 42, 1);
        worker.mark_dirty(42);

        let flushed = worker.flush_all().unwrap();
        assert_eq!(flushed, 1);
        assert_eq!(read_base(&meta, 42).nlink, 3);
    }

    #[test]
    fn compaction_invalidates_cache() {
        let (meta, ds, cache, worker) = make_worker(1);
        write_base(&meta, 42, 2);

        // Put stale value in cache
        let stale = InodeValue {
            version: 1,
            inode: 42,
            size: 0,
            mode: 0o040755,
            nlink: 999, // clearly stale
            uid: 0,
            gid: 0,
            atime: 1000,
            mtime: 1000,
            ctime: 1000,
        };
        cache.put(42, stale);
        assert!(cache.get(42).is_some());

        // Compact
        append_nlink_delta(&ds, 42, 1);
        worker.force_compact_inode(42).unwrap();

        // Cache should be invalidated
        assert!(cache.get(42).is_none());
    }

    #[test]
    fn run_loop_can_be_stopped() {
        let (_meta, _ds, _cache, worker) = make_worker(1);
        let worker = Arc::new(worker);
        let w = Arc::clone(&worker);

        let handle = std::thread::spawn(move || {
            w.run_loop();
        });

        // Give it a moment to start
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(worker.is_running());

        worker.stop();
        handle.join().unwrap();
        assert!(!worker.is_running());
    }

    #[test]
    fn mixed_delta_types_compact_correctly() {
        let (meta, ds, _cache, worker) = make_worker(1);
        write_base(&meta, 42, 2);

        // Append mixed deltas
        let ops = vec![
            DeltaOp::IncrementNlink(3),
            DeltaOp::SetMtime(5000),
            DeltaOp::SetCtime(5000),
            DeltaOp::IncrementNlink(-1),
        ];
        let serialized: Vec<Vec<u8>> = ops.iter().map(|o| o.serialize()).collect();
        ds.append_deltas(42, &serialized).unwrap();

        worker.force_compact_inode(42).unwrap();

        let iv = read_base(&meta, 42);
        assert_eq!(iv.nlink, 4); // 2 + 3 - 1
        assert_eq!(iv.mtime, 5000);
        assert_eq!(iv.ctime, 5000);
    }
}
