//! Metadata server — orchestrates storage backends to implement POSIX metadata operations.
//!
//! Data I/O (read/write/flush/fsync) is NOT handled here; instead,
//! clients talk to a separate DataServer directly.

pub mod cache;
pub mod compaction;
pub mod delta;
pub mod fsck;

use std::collections::{HashMap as StdHashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rucksfs_core::{
    DataLocation, DataOps, DirEntry, FileAttr, FsError, FsResult, Inode, MetadataOps,
    OpenResponse, SetAttrRequest, StatFs,
};
use rucksfs_storage::allocator::{InodeAllocator, ROOT_INODE};
use rucksfs_storage::encoding::{encode_delta_key, encode_dir_entry_key, encode_inode_key, InodeValue};
use rucksfs_storage::{
    AtomicWriteBatch, BatchOp, DeltaStore, DirectoryIndex, MetadataStore, StorageBundle,
};

use crate::cache::InodeFoldedCache;
use crate::compaction::{CompactionConfig, DeltaCompactionWorker};
use crate::delta::DeltaOp;

/// File-type mode bits (S_IFDIR, S_IFREG, S_IFLNK).
const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;

/// Return current UNIX timestamp in seconds.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Maximum number of retries for transient transaction conflicts.
const TXN_MAX_RETRIES: usize = 3;

/// Default capacity for the folded inode cache.
const DEFAULT_CACHE_CAPACITY: usize = 10_000;

/// Core metadata server that composes [`MetadataStore`],
/// [`DirectoryIndex`], and [`DeltaStore`] to implement metadata-only
/// POSIX file-system operations.
///
/// Data I/O is delegated to a separate DataServer via the
/// `data_client: Arc<dyn DataOps>` field.
pub struct MetadataServer<M, I, DS>
where
    M: MetadataStore,
    I: DirectoryIndex,
    DS: DeltaStore,
{
    pub metadata: Arc<M>,
    pub index: Arc<I>,
    pub delta_store: Arc<DS>,
    /// Client for talking to the DataServer (for truncate/delete on
    /// setattr size change or unlink with nlink=0).
    pub data_client: Arc<dyn DataOps>,
    /// DataServer endpoint info returned in OpenResponse.
    pub data_location: DataLocation,
    /// LRU cache of folded inode values.
    pub cache: Arc<InodeFoldedCache>,
    /// Background compaction worker (shared with the MetadataServer).
    pub compaction: Arc<DeltaCompactionWorker<M, DS>>,
    allocator: InodeAllocator,
    /// Storage bundle for atomic cross-store writes.
    storage_bundle: Arc<dyn StorageBundle>,
    /// Open file handle counter per inode. Tracks how many open() calls
    /// have not been balanced by release() for each inode.
    open_handles: Arc<Mutex<StdHashMap<Inode, u32>>>,
    /// Inodes whose nlink reached 0 while open handles > 0.
    /// Actual deletion is deferred until the last handle is closed.
    pending_deletes: Arc<Mutex<HashSet<Inode>>>,
}

impl<M, I, DS> MetadataServer<M, I, DS>
where
    M: MetadataStore,
    I: DirectoryIndex,
    DS: DeltaStore,
{
    /// Create a new `MetadataServer` and initialise the root directory
    /// (inode 1) if it does not already exist.
    pub fn new(
        metadata: Arc<M>,
        index: Arc<I>,
        delta_store: Arc<DS>,
        data_client: Arc<dyn DataOps>,
        data_location: DataLocation,
        storage_bundle: Arc<dyn StorageBundle>,
    ) -> Self {
        let allocator = InodeAllocator::load(metadata.as_ref())
            .unwrap_or_else(|_| InodeAllocator::new());

        let cache = Arc::new(InodeFoldedCache::new(DEFAULT_CACHE_CAPACITY));
        let compaction = Arc::new(DeltaCompactionWorker::new(
            Arc::clone(&metadata),
            Arc::clone(&delta_store),
            Arc::clone(&cache),
            CompactionConfig::default(),
            Arc::clone(&storage_bundle),
        ));

        let server = Self {
            metadata,
            index,
            delta_store,
            data_client,
            data_location,
            cache,
            compaction,
            allocator,
            storage_bundle,
            open_handles: Arc::new(Mutex::new(StdHashMap::new())),
            pending_deletes: Arc::new(Mutex::new(HashSet::new())),
        };

        // Ensure root directory exists.
        server.init_root();
        server
    }

    /// Create a new `MetadataServer` with a custom cache capacity and
    /// compaction configuration.
    pub fn with_config(
        metadata: Arc<M>,
        index: Arc<I>,
        delta_store: Arc<DS>,
        data_client: Arc<dyn DataOps>,
        data_location: DataLocation,
        cache_capacity: usize,
        compaction_config: CompactionConfig,
        storage_bundle: Arc<dyn StorageBundle>,
    ) -> Self {
        let allocator = InodeAllocator::load(metadata.as_ref())
            .unwrap_or_else(|_| InodeAllocator::new());

        let cache = Arc::new(InodeFoldedCache::new(cache_capacity));
        let compaction = Arc::new(DeltaCompactionWorker::new(
            Arc::clone(&metadata),
            Arc::clone(&delta_store),
            Arc::clone(&cache),
            compaction_config,
            Arc::clone(&storage_bundle),
        ));

        let server = Self {
            metadata,
            index,
            delta_store,
            data_client,
            data_location,
            cache,
            compaction,
            allocator,
            storage_bundle,
            open_handles: Arc::new(Mutex::new(StdHashMap::new())),
            pending_deletes: Arc::new(Mutex::new(HashSet::new())),
        };

        // Ensure root directory exists.
        server.init_root();
        server
    }

    /// Ensure the root directory (inode 1) exists in the metadata store.
    fn init_root(&self) {
        let key = encode_inode_key(ROOT_INODE);
        if let Ok(None) = self.metadata.get(&key) {
            let ts = now_secs();
            let root = InodeValue {
                version: 1,
                inode: ROOT_INODE,
                size: 0,
                mode: S_IFDIR | 0o777,
                nlink: 2, // "." and parent
                uid: unsafe { libc::getuid() },
                gid: unsafe { libc::getgid() },
                atime: ts,
                mtime: ts,
                ctime: ts,
            };
            let _ = self.metadata.put(&key, &root.serialize());
        }
    }

    // -- helper: load / save inode ------------------------------------------

    /// Load an inode's **effective** attributes (base + pending deltas).
    ///
    /// Resolution order:
    /// 1. Check the folded-state cache → hit → return immediately.
    /// 2. Read the base value from the metadata store.
    /// 3. Scan and fold any pending deltas from the delta store.
    /// 4. Populate the cache with the folded result.
    fn load_inode(&self, inode: Inode) -> FsResult<InodeValue> {
        // 1. Cache hit?
        if let Some(cached) = self.cache.get(inode) {
            return Ok(cached);
        }

        // 2. Read base.
        let key = encode_inode_key(inode);
        let mut iv = match self.metadata.get(&key)? {
            Some(bytes) => InodeValue::deserialize(&bytes)?,
            None => return Err(FsError::NotFound),
        };

        // 3. Fold pending deltas.
        let raw_deltas = self.delta_store.scan_deltas(inode)?;
        if !raw_deltas.is_empty() {
            let ops: Vec<DeltaOp> = raw_deltas
                .iter()
                .filter_map(|bytes| DeltaOp::deserialize(bytes).ok())
                .collect();
            delta::fold_deltas(&mut iv, &ops);
        }

        // 4. Populate cache.
        self.cache.put(inode, iv.clone());

        Ok(iv)
    }

    /// Delete an inode from the metadata store (non-batch fallback).
    #[allow(dead_code)]
    fn delete_inode(&self, inode: Inode) -> FsResult<()> {
        let key = encode_inode_key(inode);
        self.metadata.delete(&key)?;
        // Also clean up any pending deltas and cache for this inode.
        let _ = self.delta_store.clear_deltas(inode);
        self.cache.invalidate(inode);
        Ok(())
    }

    // -- helper: batch building ---------------------------------------------

    /// Begin a new atomic write batch from the storage bundle.
    fn begin_write(&self) -> Box<dyn AtomicWriteBatch + '_> {
        self.storage_bundle.begin_write()
    }

    /// Add a "put inode" operation to the batch.
    fn batch_put_inode(batch: &mut dyn AtomicWriteBatch, inode: Inode, val: &InodeValue) {
        batch.push(BatchOp::PutInode {
            key: encode_inode_key(inode),
            value: val.serialize(),
        });
    }

    /// Add a "delete inode" operation to the batch.
    fn batch_delete_inode(batch: &mut dyn AtomicWriteBatch, inode: Inode) {
        batch.push(BatchOp::DeleteInode {
            key: encode_inode_key(inode),
        });
    }

    /// Add a "put dir entry" operation to the batch.
    ///
    /// Value format: `[inode: u64 BE][mode: u32 BE]` (12 bytes).
    fn batch_put_dir_entry(
        batch: &mut dyn AtomicWriteBatch,
        parent: Inode,
        name: &str,
        child_inode: Inode,
        mode: u32,
    ) {
        let key = encode_dir_entry_key(parent, name);
        let mut value = Vec::with_capacity(12);
        value.extend_from_slice(&child_inode.to_be_bytes());
        value.extend_from_slice(&mode.to_be_bytes());
        batch.push(BatchOp::PutDirEntry { key, value });
    }

    /// Add a "delete dir entry" operation to the batch.
    fn batch_delete_dir_entry(
        batch: &mut dyn AtomicWriteBatch,
        parent: Inode,
        name: &str,
    ) {
        batch.push(BatchOp::DeleteDirEntry {
            key: encode_dir_entry_key(parent, name),
        });
    }

    /// Write nlink delta operations for a parent directory inside the
    /// transaction batch.  Each delta is stored as a `PutDelta` operation
    /// using the shared `delta_store`'s sequence allocator.
    fn batch_nlink_deltas(
        batch: &mut dyn AtomicWriteBatch,
        delta_store: &dyn DeltaStore,
        parent: Inode,
        deltas: &[DeltaOp],
    ) {
        for delta in deltas {
            let seq = delta_store.next_seq(parent);
            let key = encode_delta_key(parent, seq);
            batch.push(BatchOp::PutDelta {
                key: key.to_vec(),
                value: delta.serialize(),
            });
        }
    }

    // -- helper: delta append -----------------------------------------------

    /// Append delta operations for a parent directory and update the cache.
    fn append_parent_deltas(&self, parent: Inode, deltas: &[DeltaOp]) -> FsResult<()> {
        let serialized: Vec<Vec<u8>> = deltas.iter().map(|d| d.serialize()).collect();
        self.delta_store.append_deltas(parent, &serialized)?;

        self.cache.apply_deltas(parent, deltas);
        self.compaction.mark_dirty(parent);

        Ok(())
    }

    /// Helper: check whether a mode represents a directory.
    fn is_dir(mode: u32) -> bool {
        mode & S_IFDIR != 0
    }

    /// Helper: check whether a mode represents a symbolic link.
    fn is_symlink(mode: u32) -> bool {
        (mode & 0o170000) == S_IFLNK
    }

    /// Decode a dir-entry value (`[inode: u64 BE][mode: u32 BE]`).
    fn decode_dir_entry_value(data: &[u8]) -> FsResult<(Inode, u32)> {
        if data.len() < 12 {
            return Err(FsError::InvalidInput("dir entry value too short".into()));
        }
        let inode = u64::from_be_bytes(data[0..8].try_into().unwrap());
        let mode = u32::from_be_bytes(data[8..12].try_into().unwrap());
        Ok((inode, mode))
    }

    /// Execute a closure that creates and commits a transaction, retrying
    /// up to `TXN_MAX_RETRIES` times on `FsError::TransactionConflict`.
    ///
    /// Uses exponential backoff with jitter: 1ms, 2ms, 4ms base delays.
    fn execute_with_retry<F, T>(&self, mut f: F) -> FsResult<T>
    where
        F: FnMut() -> FsResult<T>,
    {
        for attempt in 0..TXN_MAX_RETRIES {
            match f() {
                Ok(v) => return Ok(v),
                Err(FsError::TransactionConflict) if attempt + 1 < TXN_MAX_RETRIES => {
                    let base_us = 1000u64 << attempt; // 1ms, 2ms, 4ms
                    // Simple deterministic jitter from pointer address + attempt.
                    let jitter_us = {
                        let seed = (&attempt as *const usize as u64)
                            .wrapping_mul(0x9E3779B97F4A7C15)
                            .wrapping_add(attempt as u64);
                        seed % (base_us / 2 + 1)
                    };
                    std::thread::sleep(Duration::from_micros(base_us + jitter_us));
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    /// Decrement the open handle count for `inode` and check if a deferred
    /// delete should be performed now that the last handle is closed.
    ///
    /// **Lock order**: `open_handles` → `pending_deletes` (always).
    fn check_and_clear_deferred_delete(&self, inode: Inode) -> bool {
        let mut handles = self.open_handles.lock().expect("open_handles poisoned");
        if let Some(count) = handles.get_mut(&inode) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                handles.remove(&inode);
                let mut pending = self.pending_deletes.lock().expect("pending_deletes poisoned");
                return pending.remove(&inode);
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// MetadataOps implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl<M, I, DS> MetadataOps for MetadataServer<M, I, DS>
where
    M: MetadataStore,
    I: DirectoryIndex,
    DS: DeltaStore,
{
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
        let child_inode = self
            .index
            .resolve_path(parent, name)?
            .ok_or(FsError::NotFound)?;
        let iv = self.load_inode(child_inode)?;
        Ok(iv.to_attr())
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        let iv = self.load_inode(inode)?;
        Ok(iv.to_attr())
    }

    async fn setattr(&self, inode: Inode, req: SetAttrRequest) -> FsResult<FileAttr> {
        // Handle size change outside transaction — delegate truncate to DataServer.
        let truncate_size = req.size;

        let (attr, needs_truncate) = self.execute_with_retry(|| {
            let mut batch = self.begin_write();
            let key = encode_inode_key(inode);
            let raw = batch
                .get_for_update_inode(&key)?
                .ok_or(FsError::NotFound)?;
            let mut iv = InodeValue::deserialize(&raw)?;
            let ts = now_secs();

            if let Some(mode) = req.mode {
                iv.mode = (iv.mode & 0o170000) | (mode & 0o7777);
            }
            if let Some(uid) = req.uid {
                iv.uid = uid;
            }
            if let Some(gid) = req.gid {
                iv.gid = gid;
            }
            if let Some(atime) = req.atime {
                iv.atime = atime;
            }
            if let Some(mtime) = req.mtime {
                iv.mtime = mtime;
            }

            // Track whether we need to truncate (zero-fill) after commit.
            // Only shrink requires zeroing the truncated region in the data
            // store; extending is a metadata-only operation because the data
            // store returns zeros for unwritten regions (sparse semantics).
            let mut do_truncate = false;
            if let Some(new_size) = truncate_size {
                if new_size != iv.size {
                    let is_shrink = new_size < iv.size;
                    iv.size = new_size;
                    do_truncate = is_shrink;
                }
            }

            iv.ctime = ts;
            Self::batch_put_inode(batch.as_mut(), inode, &iv);
            batch.commit()?;

            self.cache.put(inode, iv.clone());
            Ok((iv.to_attr(), do_truncate))
        })?;

        // Perform the actual truncate after transaction commit.
        if needs_truncate {
            if let Some(new_size) = truncate_size {
                self.data_client.truncate(inode, new_size).await?;
            }
        }

        Ok(attr)
    }

    async fn statfs(&self, _inode: Inode) -> FsResult<StatFs> {
        Ok(StatFs {
            blocks: 1_000_000,
            bfree: 500_000,
            bavail: 500_000,
            files: 1_000_000,
            ffree: 999_000,
            bsize: 4096,
            namelen: 255,
        })
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        let iv = self.load_inode(inode)?;
        if !Self::is_dir(iv.mode) {
            return Err(FsError::NotADirectory);
        }
        self.index.list_dir(inode)
    }

    async fn create(&self, parent: Inode, name: &str, mode: u32, uid: u32, gid: u32) -> FsResult<FileAttr> {
        let name_owned = name.to_string();

        let (iv, new_inode) = self.execute_with_retry(|| {
            let mut batch = self.begin_write();

            // Check if the name already exists (with row lock).
            let dir_key = encode_dir_entry_key(parent, &name_owned);
            if batch.get_for_update_dir_entry(&dir_key)?.is_some() {
                return Err(FsError::AlreadyExists);
            }

            let new_inode = self.allocator.alloc();
            let ts = now_secs();
            let iv = InodeValue {
                version: 1,
                inode: new_inode,
                size: 0,
                mode: S_IFREG | (mode & 0o7777),
                nlink: 1,
                uid,
                gid,
                atime: ts,
                mtime: ts,
                ctime: ts,
            };

            Self::batch_put_inode(batch.as_mut(), new_inode, &iv);
            Self::batch_put_dir_entry(batch.as_mut(), parent, &name_owned, new_inode, iv.mode);
            batch.commit()?;

            Ok((iv, new_inode))
        })?;

        // Persist allocator counter outside the transaction (hot-key avoidance).
        self.allocator.maybe_persist(self.metadata.as_ref())?;

        // Update in-memory state after successful commit.
        self.cache.put(new_inode, iv.clone());
        if !self.index.shares_batch_storage() {
            let _ = self
                .index
                .insert_child(parent, &name_owned, new_inode, iv.to_attr());
        }

        // Delta append outside transaction — losing it on crash only affects parent mtime/ctime.
        let ts = now_secs();
        let _ = self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        );

        Ok(iv.to_attr())
    }

    async fn mkdir(&self, parent: Inode, name: &str, mode: u32, uid: u32, gid: u32) -> FsResult<FileAttr> {
        let name_owned = name.to_string();

        let (iv, new_inode) = self.execute_with_retry(|| {
            let mut batch = self.begin_write();

            // Check if the name already exists (with row lock).
            let dir_key = encode_dir_entry_key(parent, &name_owned);
            if batch.get_for_update_dir_entry(&dir_key)?.is_some() {
                return Err(FsError::AlreadyExists);
            }

            let new_inode = self.allocator.alloc();
            let ts = now_secs();
            let iv = InodeValue {
                version: 1,
                inode: new_inode,
                size: 0,
                mode: S_IFDIR | (mode & 0o7777),
                nlink: 2,
                uid,
                gid,
                atime: ts,
                mtime: ts,
                ctime: ts,
            };

            Self::batch_put_inode(batch.as_mut(), new_inode, &iv);
            Self::batch_put_dir_entry(batch.as_mut(), parent, &name_owned, new_inode, iv.mode);

            // Nlink delta inside transaction for correctness.
            Self::batch_nlink_deltas(
                batch.as_mut(),
                self.delta_store.as_ref(),
                parent,
                &[DeltaOp::IncrementNlink(1)],
            );

            batch.commit()?;

            Ok((iv, new_inode))
        })?;

        // Persist allocator counter outside the transaction.
        self.allocator.maybe_persist(self.metadata.as_ref())?;

        // Update in-memory state.
        self.cache.put(new_inode, iv.clone());
        self.cache.apply_delta(parent, &DeltaOp::IncrementNlink(1));
        self.compaction.mark_dirty(parent);
        if !self.index.shares_batch_storage() {
            let _ = self
                .index
                .insert_child(parent, &name_owned, new_inode, iv.to_attr());
        }

        // Timestamp deltas remain outside transaction.
        let ts = now_secs();
        let _ = self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        );

        Ok(iv.to_attr())
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
        let name_owned = name.to_string();

        let need_delete_data = self.execute_with_retry(|| {
            let mut batch = self.begin_write();

            // Read and lock the dir entry.
            let dir_key = encode_dir_entry_key(parent, &name_owned);
            let dir_val = batch
                .get_for_update_dir_entry(&dir_key)?
                .ok_or(FsError::NotFound)?;
            let (child_inode, child_mode) = Self::decode_dir_entry_value(&dir_val)?;

            if Self::is_dir(child_mode) {
                return Err(FsError::IsADirectory);
            }

            // Read and lock the child inode.
            let inode_key = encode_inode_key(child_inode);
            let inode_raw = batch
                .get_for_update_inode(&inode_key)?
                .ok_or(FsError::NotFound)?;
            let mut child_iv = InodeValue::deserialize(&inode_raw)?;

            child_iv.nlink = child_iv.nlink.saturating_sub(1);

            Self::batch_delete_dir_entry(batch.as_mut(), parent, &name_owned);

            let result = if child_iv.nlink == 0 {
                Self::batch_delete_inode(batch.as_mut(), child_inode);
                Some(child_inode)
            } else {
                let ts = now_secs();
                child_iv.ctime = ts;
                Self::batch_put_inode(batch.as_mut(), child_inode, &child_iv);
                None
            };
            batch.commit()?;

            // Update in-memory state after commit.
            if !self.index.shares_batch_storage() {
                let _ = self.index.remove_child(parent, &name_owned);
            }
            if result.is_some() {
                let _ = self.delta_store.clear_deltas(child_inode);
                self.cache.invalidate(child_inode);
            } else {
                self.cache.put(child_inode, child_iv);
            }

            Ok(result)
        })?;

        // Ask DataServer to clean up file data (outside transaction scope).
        if let Some(inode) = need_delete_data {
            let has_handles = {
                let handles = self.open_handles.lock().expect("open_handles poisoned");
                handles.get(&inode).copied().unwrap_or(0) > 0
            };
            if has_handles {
                // Defer deletion until last handle is closed.
                let mut pending = self.pending_deletes.lock().expect("pending_deletes poisoned");
                pending.insert(inode);
            } else {
                self.data_client.delete_data(inode).await?;
            }
        }

        let ts = now_secs();
        let _ = self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        );

        Ok(())
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        let name_owned = name.to_string();

        self.execute_with_retry(|| {
            let mut batch = self.begin_write();

            // Read and lock the dir entry.
            let dir_key = encode_dir_entry_key(parent, &name_owned);
            let dir_val = batch
                .get_for_update_dir_entry(&dir_key)?
                .ok_or(FsError::NotFound)?;
            let (child_inode, child_mode) = Self::decode_dir_entry_value(&dir_val)?;

            if !Self::is_dir(child_mode) {
                return Err(FsError::NotADirectory);
            }

            // Read and lock the child inode.
            let inode_key = encode_inode_key(child_inode);
            let _inode_raw = batch
                .get_for_update_inode(&inode_key)?
                .ok_or(FsError::NotFound)?;

            // Check if directory is empty (inside transaction to avoid TOCTOU).
            if !batch.is_dir_empty(child_inode)? {
                return Err(FsError::DirectoryNotEmpty);
            }

            Self::batch_delete_dir_entry(batch.as_mut(), parent, &name_owned);
            Self::batch_delete_inode(batch.as_mut(), child_inode);

            // Nlink delta inside transaction for correctness.
            Self::batch_nlink_deltas(
                batch.as_mut(),
                self.delta_store.as_ref(),
                parent,
                &[DeltaOp::IncrementNlink(-1)],
            );

            batch.commit()?;

            // Update in-memory state after commit.
            if !self.index.shares_batch_storage() {
                let _ = self.index.remove_child(parent, &name_owned);
            }
            let _ = self.delta_store.clear_deltas(child_inode);
            self.cache.invalidate(child_inode);

            Ok(())
        })?;

        // Update cache for the nlink delta written inside the transaction.
        self.cache.apply_delta(parent, &DeltaOp::IncrementNlink(-1));
        self.compaction.mark_dirty(parent);

        // Timestamp deltas remain outside transaction.
        let ts = now_secs();
        let _ = self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        );

        Ok(())
    }

    async fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<()> {
        let name_owned = name.to_string();
        let new_name_owned = new_name.to_string();

        /// Tracks which nlink deltas were written inside the transaction
        /// so we can update the cache after commit.
        struct RenameResult {
            delete_inode: Option<Inode>,
            src_is_dir: bool,
            dst_was_dir: bool,
        }

        let result = self.execute_with_retry(|| {
            let mut batch = self.begin_write();

            // Read and lock the source dir entry.
            let src_dir_key = encode_dir_entry_key(parent, &name_owned);
            let src_dir_val = batch
                .get_for_update_dir_entry(&src_dir_key)?
                .ok_or(FsError::NotFound)?;
            let (src_inode, _) = Self::decode_dir_entry_value(&src_dir_val)?;

            // Read and lock the destination dir entry (may not exist).
            let dst_dir_key = encode_dir_entry_key(new_parent, &new_name_owned);
            let existing_dst = batch.get_for_update_dir_entry(&dst_dir_key)?;

            // Lock all involved inodes in inode-ID order to prevent deadlocks.
            let mut inode_ids = vec![src_inode];
            let mut dst_inode_opt: Option<(Inode, u32)> = None;
            if let Some(ref dst_val) = existing_dst {
                let (dst_ino, dst_mode) = Self::decode_dir_entry_value(dst_val)?;
                inode_ids.push(dst_ino);
                dst_inode_opt = Some((dst_ino, dst_mode));
            }
            // Also lock parent inodes if different from src/dst.
            if !inode_ids.contains(&parent) {
                inode_ids.push(parent);
            }
            if parent != new_parent && !inode_ids.contains(&new_parent) {
                inode_ids.push(new_parent);
            }
            inode_ids.sort_unstable();
            inode_ids.dedup();

            // Acquire row locks in sorted order.
            let mut inode_values: std::collections::HashMap<Inode, InodeValue> =
                std::collections::HashMap::new();
            for &ino in &inode_ids {
                let ino_key = encode_inode_key(ino);
                if let Some(raw) = batch.get_for_update_inode(&ino_key)? {
                    inode_values.insert(ino, InodeValue::deserialize(&raw)?);
                }
            }

            let src_iv = inode_values
                .get(&src_inode)
                .ok_or(FsError::NotFound)?
                .clone();
            let src_is_dir = Self::is_dir(src_iv.mode);
            let ts = now_secs();
            let mut delete_inode: Option<Inode> = None;
            let mut dst_was_dir = false;

            // Check if target already exists.
            if let Some((dst_inode, dst_mode)) = dst_inode_opt {
                let dst_is_dir = Self::is_dir(dst_mode);

                if src_is_dir && !dst_is_dir {
                    return Err(FsError::NotADirectory);
                }
                if !src_is_dir && dst_is_dir {
                    return Err(FsError::IsADirectory);
                }

                if dst_is_dir {
                    if !batch.is_dir_empty(dst_inode)? {
                        return Err(FsError::DirectoryNotEmpty);
                    }
                    dst_was_dir = true;
                } else {
                    delete_inode = Some(dst_inode);
                }
            }

            // Build atomic batch.
            if let Some((dst_inode, _)) = dst_inode_opt {
                Self::batch_delete_dir_entry(batch.as_mut(), new_parent, &new_name_owned);
                Self::batch_delete_inode(batch.as_mut(), dst_inode);
            }

            Self::batch_delete_dir_entry(batch.as_mut(), parent, &name_owned);
            Self::batch_put_dir_entry(
                batch.as_mut(),
                new_parent,
                &new_name_owned,
                src_inode,
                src_iv.mode,
            );

            let mut updated_src = src_iv.clone();
            updated_src.ctime = ts;
            Self::batch_put_inode(batch.as_mut(), src_inode, &updated_src);

            // Nlink deltas inside transaction for correctness.
            if dst_was_dir {
                Self::batch_nlink_deltas(
                    batch.as_mut(),
                    self.delta_store.as_ref(),
                    new_parent,
                    &[DeltaOp::IncrementNlink(-1)],
                );
            }
            if src_is_dir && parent != new_parent {
                Self::batch_nlink_deltas(
                    batch.as_mut(),
                    self.delta_store.as_ref(),
                    parent,
                    &[DeltaOp::IncrementNlink(-1)],
                );
                Self::batch_nlink_deltas(
                    batch.as_mut(),
                    self.delta_store.as_ref(),
                    new_parent,
                    &[DeltaOp::IncrementNlink(1)],
                );
            }

            batch.commit()?;

            // Update in-memory state after commit.
            if let Some((dst_inode, _)) = dst_inode_opt {
                let _ = self.delta_store.clear_deltas(dst_inode);
                self.cache.invalidate(dst_inode);
            }
            if !self.index.shares_batch_storage() {
                let _ = self.index.remove_child(new_parent, &new_name_owned);
                let _ = self.index.remove_child(parent, &name_owned);
                let _ = self.index.insert_child(
                    new_parent,
                    &new_name_owned,
                    src_inode,
                    updated_src.to_attr(),
                );
            }
            self.cache.put(src_inode, updated_src);

            Ok(RenameResult {
                delete_inode,
                src_is_dir,
                dst_was_dir,
            })
        })?;

        // Update cache for nlink deltas written inside the transaction.
        let ts = now_secs();
        if result.dst_was_dir {
            self.cache.apply_delta(new_parent, &DeltaOp::IncrementNlink(-1));
            self.compaction.mark_dirty(new_parent);
        }
        if result.src_is_dir && parent != new_parent {
            self.cache.apply_delta(parent, &DeltaOp::IncrementNlink(-1));
            self.compaction.mark_dirty(parent);
            self.cache.apply_delta(new_parent, &DeltaOp::IncrementNlink(1));
            self.compaction.mark_dirty(new_parent);
        }

        // Timestamp deltas remain outside transaction.
        let _ = self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        );
        if parent != new_parent {
            let _ = self.append_parent_deltas(
                new_parent,
                &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
            );
        }

        // Ask DataServer to clean up data (outside transaction scope).
        if let Some(inode) = result.delete_inode {
            self.data_client.delete_data(inode).await?;
        }

        Ok(())
    }

    async fn open(&self, inode: Inode, _flags: u32) -> FsResult<OpenResponse> {
        let iv = self.load_inode(inode)?;
        if Self::is_dir(iv.mode) {
            return Err(FsError::IsADirectory);
        }
        // Increment open handle count.
        {
            let mut handles = self.open_handles.lock().expect("open_handles poisoned");
            *handles.entry(inode).or_insert(0) += 1;
        }
        Ok(OpenResponse {
            handle: inode, // Use inode as handle for simplicity.
            data_location: self.data_location.clone(),
        })
    }

    async fn report_write(
        &self,
        inode: Inode,
        new_size: u64,
        mtime: u64,
    ) -> FsResult<()> {
        self.execute_with_retry(|| {
            let mut batch = self.begin_write();
            let key = encode_inode_key(inode);
            let raw = batch
                .get_for_update_inode(&key)?
                .ok_or(FsError::NotFound)?;
            let mut iv = InodeValue::deserialize(&raw)?;

            if new_size > iv.size {
                iv.size = new_size;
            }
            iv.mtime = mtime;
            iv.ctime = mtime;

            Self::batch_put_inode(batch.as_mut(), inode, &iv);
            batch.commit()?;

            self.cache.put(inode, iv);
            Ok(())
        })
    }

    async fn link(&self, parent: Inode, name: &str, target_inode: Inode) -> FsResult<FileAttr> {
        let name_owned = name.to_string();

        let target_iv = self.execute_with_retry(|| {
            let mut batch = self.begin_write();

            // Lock and check the target inode exists.
            let inode_key = encode_inode_key(target_inode);
            let inode_raw = batch
                .get_for_update_inode(&inode_key)?
                .ok_or(FsError::NotFound)?;
            let mut target_iv = InodeValue::deserialize(&inode_raw)?;

            // POSIX: hard links to directories are not allowed.
            if Self::is_dir(target_iv.mode) {
                return Err(FsError::PermissionDenied);
            }

            // Check if the name already exists in the parent (with row lock).
            let dir_key = encode_dir_entry_key(parent, &name_owned);
            if batch.get_for_update_dir_entry(&dir_key)?.is_some() {
                return Err(FsError::AlreadyExists);
            }

            // Verify parent directory exists.
            let parent_key = encode_inode_key(parent);
            let parent_raw = batch
                .get_for_update_inode(&parent_key)?
                .ok_or(FsError::NotFound)?;
            let parent_iv = InodeValue::deserialize(&parent_raw)?;
            if !Self::is_dir(parent_iv.mode) {
                return Err(FsError::NotADirectory);
            }

            // Increment nlink and update ctime.
            target_iv.nlink += 1;
            target_iv.ctime = now_secs();

            Self::batch_put_inode(batch.as_mut(), target_inode, &target_iv);
            Self::batch_put_dir_entry(
                batch.as_mut(),
                parent,
                &name_owned,
                target_inode,
                target_iv.mode,
            );
            batch.commit()?;

            // Update in-memory state after commit.
            self.cache.put(target_inode, target_iv.clone());
            if !self.index.shares_batch_storage() {
                let _ = self.index.insert_child(
                    parent,
                    &name_owned,
                    target_inode,
                    target_iv.to_attr(),
                );
            }

            Ok(target_iv)
        })?;

        // Update parent dir timestamps via delta.
        let ts = now_secs();
        let _ = self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        );

        Ok(target_iv.to_attr())
    }

    async fn symlink(
        &self,
        parent: Inode,
        name: &str,
        link_target: &str,
        uid: u32,
        gid: u32,
    ) -> FsResult<FileAttr> {
        let name_owned = name.to_string();
        let target_owned = link_target.to_string();

        let (iv, new_inode) = self.execute_with_retry(|| {
            let mut batch = self.begin_write();

            // Check if the name already exists (with row lock).
            let dir_key = encode_dir_entry_key(parent, &name_owned);
            if batch.get_for_update_dir_entry(&dir_key)?.is_some() {
                return Err(FsError::AlreadyExists);
            }

            let new_inode = self.allocator.alloc();
            let ts = now_secs();
            let iv = InodeValue {
                version: 1,
                inode: new_inode,
                size: target_owned.len() as u64,
                mode: S_IFLNK | 0o777,
                nlink: 1,
                uid,
                gid,
                atime: ts,
                mtime: ts,
                ctime: ts,
            };

            Self::batch_put_inode(batch.as_mut(), new_inode, &iv);
            Self::batch_put_dir_entry(batch.as_mut(), parent, &name_owned, new_inode, iv.mode);
            batch.commit()?;

            Ok((iv, new_inode))
        })?;

        // Persist allocator counter outside the transaction.
        self.allocator.maybe_persist(self.metadata.as_ref())?;

        // Store the symlink target as file data.
        self.data_client
            .write_data(new_inode, 0, target_owned.as_bytes())
            .await?;

        // Update in-memory state.
        self.cache.put(new_inode, iv.clone());
        if !self.index.shares_batch_storage() {
            let _ = self
                .index
                .insert_child(parent, &name_owned, new_inode, iv.to_attr());
        }

        // Update parent dir timestamps via delta.
        let ts = now_secs();
        let _ = self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        );

        Ok(iv.to_attr())
    }

    async fn readlink(&self, inode: Inode) -> FsResult<String> {
        let iv = self.load_inode(inode)?;

        // POSIX: readlink on non-symlink returns EINVAL.
        if !Self::is_symlink(iv.mode) {
            return Err(FsError::InvalidInput("not a symbolic link".into()));
        }

        let data = self
            .data_client
            .read_data(inode, 0, iv.size as u32)
            .await?;

        String::from_utf8(data).map_err(|e| FsError::Io(format!("invalid symlink target: {}", e)))
    }

    async fn release(&self, inode: Inode) -> FsResult<()> {
        if self.check_and_clear_deferred_delete(inode) {
            self.data_client.delete_data(inode).await?;
        }
        Ok(())
    }
}
