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
use rucksfs_storage::{
    BatchOp, DeltaStore, MetadataStore, StorageBundle,
};

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
    #[allow(dead_code)]
    metadata: Arc<M>,
    delta_store: Arc<DS>,
    cache: Arc<InodeFoldedCache>,
    config: CompactionConfig,
    /// Set of inodes that have pending deltas.
    dirty: Mutex<HashSet<Inode>>,
    /// Flag to stop the background loop.
    running: AtomicBool,
    /// Storage bundle for atomic writes (put merged inode + clear deltas).
    storage_bundle: Arc<dyn StorageBundle>,
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
        storage_bundle: Arc<dyn StorageBundle>,
    ) -> Self {
        Self {
            metadata,
            delta_store,
            cache,
            config,
            dirty: Mutex::new(HashSet::new()),
            running: AtomicBool::new(false),
            storage_bundle,
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
        // 1. Scan deltas (outside transaction — delta keys are append-only
        //    and won't conflict with other transactions).
        let raw_deltas = self.delta_store.scan_deltas(inode)?;
        if raw_deltas.is_empty() {
            return Ok(false);
        }

        // 2. Begin transaction and lock the base inode.
        let mut batch = self.storage_bundle.begin_write();
        let key = encode_inode_key(inode);
        let mut base = match batch.get_for_update_inode(&key)? {
            Some(bytes) => InodeValue::deserialize(&bytes)?,
            None => return Ok(false), // inode was deleted
        };

        // 3. Fold.
        let ops: Vec<DeltaOp> = raw_deltas
            .iter()
            .filter_map(|bytes| DeltaOp::deserialize(bytes).ok())
            .collect();
        delta::fold_deltas(&mut base, &ops);

        // 4. Write merged inode + delete delta keys within the transaction.
        batch.push(BatchOp::PutInode {
            key: key.clone(),
            value: base.serialize(),
        });
        let delta_keys = self.delta_store.scan_delta_keys(inode)?;
        for dk in &delta_keys {
            batch.push(BatchOp::DeleteDelta { key: dk.clone() });
        }

        // 5. Commit — if the inode was concurrently deleted/modified,
        //    PCC will detect the conflict and we safely skip.
        match batch.commit() {
            Ok(()) => {}
            Err(rucksfs_core::FsError::TransactionConflict) => {
                // Another transaction modified this inode (e.g. unlink).
                // Safe to skip — the inode will be re-compacted later or
                // was already deleted.
                return Ok(false);
            }
            Err(e) => return Err(e),
        }

        // 6. Update in-memory state after successful commit.
        let _ = self.delta_store.clear_deltas(inode);

        // 7. Invalidate cache so the next read picks up the fresh base.
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
    use rucksfs_storage::encoding::encode_inode_key;
    use rucksfs_storage::{open_rocks_db, RocksDeltaStore, RocksMetadataStore, RocksStorageBundle};

    fn make_worker(
        threshold: usize,
    ) -> (
        tempfile::TempDir,
        Arc<RocksMetadataStore>,
        Arc<RocksDeltaStore>,
        Arc<InodeFoldedCache>,
        DeltaCompactionWorker<RocksMetadataStore, RocksDeltaStore>,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let db = open_rocks_db(tmp.path().join("meta.db")).unwrap();
        let meta = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
        let ds = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
        let cache = Arc::new(InodeFoldedCache::new(100));
        let config = CompactionConfig {
            delta_threshold: threshold,
            interval_ms: 50,
        };
        let bundle: Arc<dyn StorageBundle> = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));
        let worker = DeltaCompactionWorker::new(
            Arc::clone(&meta),
            Arc::clone(&ds),
            Arc::clone(&cache),
            config,
            bundle,
        );
        (tmp, meta, ds, cache, worker)
    }

    fn write_base(meta: &RocksMetadataStore, inode: Inode, nlink: u32) {
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

    fn append_nlink_delta(ds: &RocksDeltaStore, inode: Inode, amount: i32) {
        let op = DeltaOp::IncrementNlink(amount);
        ds.append_deltas(inode, &[op.serialize()]).unwrap();
    }

    fn read_base(meta: &RocksMetadataStore, inode: Inode) -> InodeValue {
        let bytes = meta.get(&encode_inode_key(inode)).unwrap().unwrap();
        InodeValue::deserialize(&bytes).unwrap()
    }

    #[test]
    fn compact_inode_below_threshold_skips() {
        let (_tmp, meta, ds, _cache, worker) = make_worker(5);
        write_base(&meta, 42, 2);
        for _ in 0..3 {
            append_nlink_delta(&ds, 42, 1);
        }
        let result = worker.compact_inode(42).unwrap();
        assert!(!result);
        assert_eq!(ds.scan_deltas(42).unwrap().len(), 3);
    }

    #[test]
    fn compact_inode_at_threshold_compacts() {
        let (_tmp, meta, ds, _cache, worker) = make_worker(5);
        write_base(&meta, 42, 2);
        for _ in 0..5 {
            append_nlink_delta(&ds, 42, 1);
        }
        let result = worker.compact_inode(42).unwrap();
        assert!(result);

        let iv = read_base(&meta, 42);
        assert_eq!(iv.nlink, 7); // 2 + 5

        assert!(ds.scan_deltas(42).unwrap().is_empty());
    }

    #[test]
    fn force_compact_ignores_threshold() {
        let (_tmp, meta, ds, _cache, worker) = make_worker(100);
        write_base(&meta, 42, 2);
        append_nlink_delta(&ds, 42, 1);
        let result = worker.force_compact_inode(42).unwrap();
        assert!(result);
        assert_eq!(read_base(&meta, 42).nlink, 3);
        assert!(ds.scan_deltas(42).unwrap().is_empty());
    }

    #[test]
    fn compact_nonexistent_inode_returns_false() {
        let (_tmp, _meta, _ds, _cache, worker) = make_worker(1);
        let result = worker.force_compact_inode(999).unwrap();
        assert!(!result);
    }

    #[test]
    fn compact_no_deltas_returns_false() {
        let (_tmp, meta, _ds, _cache, worker) = make_worker(1);
        write_base(&meta, 42, 2);
        let result = worker.force_compact_inode(42).unwrap();
        assert!(!result);
    }

    #[test]
    fn compact_dirty_batch() {
        let (_tmp, meta, ds, _cache, worker) = make_worker(2);
        for inode in [10, 20, 30] {
            write_base(&meta, inode, 1);
            for _ in 0..3 {
                append_nlink_delta(&ds, inode, 1);
            }
            worker.mark_dirty(inode);
        }

        let compacted = worker.compact_dirty().unwrap();
        assert_eq!(compacted, 3);

        for inode in [10, 20, 30] {
            assert_eq!(read_base(&meta, inode).nlink, 4); // 1 + 3
            assert!(ds.scan_deltas(inode).unwrap().is_empty());
        }
    }

    #[test]
    fn compact_dirty_re_marks_below_threshold() {
        let (_tmp, meta, ds, _cache, worker) = make_worker(10);
        write_base(&meta, 42, 2);
        append_nlink_delta(&ds, 42, 1);
        worker.mark_dirty(42);

        let compacted = worker.compact_dirty().unwrap();
        assert_eq!(compacted, 0);

        let dirty = worker.dirty.lock().unwrap();
        assert!(dirty.contains(&42));
    }

    #[test]
    fn flush_all_forces_all() {
        let (_tmp, meta, ds, _cache, worker) = make_worker(100);
        write_base(&meta, 42, 2);
        append_nlink_delta(&ds, 42, 1);
        worker.mark_dirty(42);

        let flushed = worker.flush_all().unwrap();
        assert_eq!(flushed, 1);
        assert_eq!(read_base(&meta, 42).nlink, 3);
    }

    #[test]
    fn compaction_invalidates_cache() {
        let (_tmp, meta, ds, cache, worker) = make_worker(1);
        write_base(&meta, 42, 2);

        let stale = InodeValue {
            version: 1,
            inode: 42,
            size: 0,
            mode: 0o040755,
            nlink: 999,
            uid: 0,
            gid: 0,
            atime: 1000,
            mtime: 1000,
            ctime: 1000,
        };
        cache.put(42, stale);
        assert!(cache.get(42).is_some());

        append_nlink_delta(&ds, 42, 1);
        worker.force_compact_inode(42).unwrap();

        assert!(cache.get(42).is_none());
    }

    #[test]
    fn run_loop_can_be_stopped() {
        let (_tmp, _meta, _ds, _cache, worker) = make_worker(1);
        let worker = Arc::new(worker);
        let w = Arc::clone(&worker);

        let handle = std::thread::spawn(move || {
            w.run_loop();
        });

        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(worker.is_running());

        worker.stop();
        handle.join().unwrap();
        assert!(!worker.is_running());
    }

    #[test]
    fn mixed_delta_types_compact_correctly() {
        let (_tmp, meta, ds, _cache, worker) = make_worker(1);
        write_base(&meta, 42, 2);

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
