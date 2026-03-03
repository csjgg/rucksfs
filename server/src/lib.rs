//! Metadata server — orchestrates storage backends to implement POSIX metadata operations.
//!
//! Data I/O (read/write/flush/fsync) is NOT handled here; instead,
//! clients talk to a separate DataServer directly.

pub mod cache;
pub mod compaction;
pub mod delta;

use std::sync::Arc;

use async_trait::async_trait;
use rucksfs_core::{
    DataLocation, DataOps, DirEntry, FileAttr, FsError, FsResult, Inode, MetadataOps,
    OpenResponse, SetAttrRequest, StatFs,
};
use rucksfs_storage::allocator::{InodeAllocator, ROOT_INODE};
use rucksfs_storage::encoding::{encode_dir_entry_key, encode_inode_key, InodeValue};
use rucksfs_storage::{
    AtomicWriteBatch, BatchOp, DeltaStore, DirectoryIndex, MetadataStore, StorageBundle,
};

use crate::cache::InodeFoldedCache;
use crate::compaction::{CompactionConfig, DeltaCompactionWorker};
use crate::delta::DeltaOp;

/// File-type mode bits (S_IFDIR, S_IFREG).
const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;

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
                mode: S_IFDIR | 0o755,
                nlink: 2, // "." and parent
                uid: 0,
                gid: 0,
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
    fn execute_with_retry<F, T>(&self, mut f: F) -> FsResult<T>
    where
        F: FnMut() -> FsResult<T>,
    {
        for attempt in 0..TXN_MAX_RETRIES {
            match f() {
                Ok(v) => return Ok(v),
                Err(FsError::TransactionConflict) if attempt + 1 < TXN_MAX_RETRIES => {
                    // Retry on transient conflict.
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
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

            // Track whether we need to truncate after commit.
            let mut do_truncate = false;
            if let Some(new_size) = truncate_size {
                if new_size != iv.size {
                    iv.size = new_size;
                    do_truncate = true;
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

    async fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
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
                uid: 0,
                gid: 0,
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
        self.allocator.persist(self.metadata.as_ref())?;

        // Update in-memory state after successful commit.
        self.cache.put(new_inode, iv.clone());
        let _ = self
            .index
            .insert_child(parent, &name_owned, new_inode, iv.to_attr());

        // Delta append outside transaction — losing it on crash only affects parent mtime/ctime.
        let ts = now_secs();
        let _ = self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        );

        Ok(iv.to_attr())
    }

    async fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
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
                uid: 0,
                gid: 0,
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
        self.allocator.persist(self.metadata.as_ref())?;

        // Update in-memory state.
        self.cache.put(new_inode, iv.clone());
        let _ = self
            .index
            .insert_child(parent, &name_owned, new_inode, iv.to_attr());

        // Delta append outside transaction.
        let ts = now_secs();
        let _ = self.append_parent_deltas(
            parent,
            &[
                DeltaOp::IncrementNlink(1),
                DeltaOp::SetMtime(ts),
                DeltaOp::SetCtime(ts),
            ],
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
            let _ = self.index.remove_child(parent, &name_owned);
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
            self.data_client.delete_data(inode).await?;
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
            batch.commit()?;

            // Update in-memory state after commit.
            let _ = self.index.remove_child(parent, &name_owned);
            let _ = self.delta_store.clear_deltas(child_inode);
            self.cache.invalidate(child_inode);

            Ok(())
        })?;

        let ts = now_secs();
        let _ = self.append_parent_deltas(
            parent,
            &[
                DeltaOp::IncrementNlink(-1),
                DeltaOp::SetMtime(ts),
                DeltaOp::SetCtime(ts),
            ],
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

        let need_delete_data = self.execute_with_retry(|| {
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

            batch.commit()?;

            // Update in-memory state after commit.
            if let Some((dst_inode, _)) = dst_inode_opt {
                let _ = self.delta_store.clear_deltas(dst_inode);
                self.cache.invalidate(dst_inode);
            }
            let _ = self.index.remove_child(new_parent, &new_name_owned);
            let _ = self.index.remove_child(parent, &name_owned);
            let _ = self.index.insert_child(
                new_parent,
                &new_name_owned,
                src_inode,
                updated_src.to_attr(),
            );
            self.cache.put(src_inode, updated_src);

            // Delta appends outside batch.
            if dst_was_dir {
                let _ = self.append_parent_deltas(
                    new_parent,
                    &[
                        DeltaOp::IncrementNlink(-1),
                        DeltaOp::SetMtime(ts),
                        DeltaOp::SetCtime(ts),
                    ],
                );
            }

            if src_is_dir && parent != new_parent {
                let _ = self.append_parent_deltas(
                    parent,
                    &[
                        DeltaOp::IncrementNlink(-1),
                        DeltaOp::SetMtime(ts),
                        DeltaOp::SetCtime(ts),
                    ],
                );
                let _ = self.append_parent_deltas(
                    new_parent,
                    &[
                        DeltaOp::IncrementNlink(1),
                        DeltaOp::SetMtime(ts),
                        DeltaOp::SetCtime(ts),
                    ],
                );
            } else {
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
            }

            Ok(delete_inode)
        })?;

        // Ask DataServer to clean up data (outside transaction scope).
        if let Some(inode) = need_delete_data {
            self.data_client.delete_data(inode).await?;
        }

        Ok(())
    }

    async fn open(&self, inode: Inode, _flags: u32) -> FsResult<OpenResponse> {
        let iv = self.load_inode(inode)?;
        if Self::is_dir(iv.mode) {
            return Err(FsError::IsADirectory);
        }
        Ok(OpenResponse {
            handle: 0, // We don't track open files yet.
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
}
