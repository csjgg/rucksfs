//! Metadata server — orchestrates storage backends to implement POSIX metadata operations.
//!
//! Data I/O (read/write/flush/fsync) is NOT handled here; instead,
//! clients talk to a separate DataServer directly.

pub mod cache;
pub mod compaction;
pub mod delta;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use rucksfs_core::{
    DataLocation, DataOps, DirEntry, FileAttr, FsError, FsResult, Inode, MetadataOps,
    OpenResponse, SetAttrRequest, StatFs,
};
use rucksfs_storage::allocator::{InodeAllocator, ROOT_INODE};
use rucksfs_storage::encoding::{encode_inode_key, InodeValue};
use rucksfs_storage::{DeltaStore, DirectoryIndex, MetadataStore};

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

/// RAII guard that owns an `Arc<Mutex<()>>` and its `MutexGuard`.
///
/// This avoids the lifetime issue of returning a `MutexGuard` that
/// references a function-local `Arc`.
pub struct DirLockGuard {
    // Order matters: `_guard` must be dropped before `_mutex`.
    _guard: MutexGuard<'static, ()>,
    _mutex: Arc<Mutex<()>>,
}

impl DirLockGuard {
    fn new(mutex: Arc<Mutex<()>>) -> Self {
        // SAFETY: We extend the lifetime of the MutexGuard to 'static.
        // This is safe because `_mutex` (the Arc) is stored in the same
        // struct and will outlive `_guard`. Drop order in Rust is
        // declaration order, so `_guard` is dropped first.
        let guard = mutex.lock().expect("per-dir lock poisoned");
        let guard: MutexGuard<'static, ()> = unsafe { std::mem::transmute(guard) };
        Self {
            _guard: guard,
            _mutex: mutex,
        }
    }
}

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
    /// Per-directory lock to serialize mutations under the same parent.
    dir_locks: Mutex<HashMap<Inode, Arc<Mutex<()>>>>,
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
    ) -> Self {
        let allocator = InodeAllocator::load(metadata.as_ref())
            .unwrap_or_else(|_| InodeAllocator::new());

        let cache = Arc::new(InodeFoldedCache::new(DEFAULT_CACHE_CAPACITY));
        let compaction = Arc::new(DeltaCompactionWorker::new(
            Arc::clone(&metadata),
            Arc::clone(&delta_store),
            Arc::clone(&cache),
            CompactionConfig::default(),
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
            dir_locks: Mutex::new(HashMap::new()),
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
    ) -> Self {
        let allocator = InodeAllocator::load(metadata.as_ref())
            .unwrap_or_else(|_| InodeAllocator::new());

        let cache = Arc::new(InodeFoldedCache::new(cache_capacity));
        let compaction = Arc::new(DeltaCompactionWorker::new(
            Arc::clone(&metadata),
            Arc::clone(&delta_store),
            Arc::clone(&cache),
            compaction_config,
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
            dir_locks: Mutex::new(HashMap::new()),
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

    /// Serialize and save an inode to the metadata store, and update the
    /// cache to reflect the new base value.
    fn save_inode(&self, inode: Inode, val: &InodeValue) -> FsResult<()> {
        let key = encode_inode_key(inode);
        self.metadata.put(&key, &val.serialize())?;
        // Update the cache so subsequent reads see the latest value.
        self.cache.put(inode, val.clone());
        Ok(())
    }

    /// Delete an inode from the metadata store.
    fn delete_inode(&self, inode: Inode) -> FsResult<()> {
        let key = encode_inode_key(inode);
        self.metadata.delete(&key)?;
        // Also clean up any pending deltas and cache for this inode.
        let _ = self.delta_store.clear_deltas(inode);
        self.cache.invalidate(inode);
        Ok(())
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

    // -- helper: per-directory lock -----------------------------------------

    /// Acquire the per-directory mutex for `parent`.
    fn lock_dir(&self, parent: Inode) -> DirLockGuard {
        let mutex = {
            let mut map = self.dir_locks.lock().expect("dir_locks poisoned");
            map.entry(parent)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        DirLockGuard::new(mutex)
    }

    /// Helper: check whether a mode represents a directory.
    fn is_dir(mode: u32) -> bool {
        mode & S_IFDIR != 0
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
        let mut iv = self.load_inode(inode)?;
        let ts = now_secs();

        if let Some(mode) = req.mode {
            // Preserve file-type bits, update permission bits only.
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

        // Handle size change → delegate truncate to DataServer.
        if let Some(new_size) = req.size {
            if new_size != iv.size {
                self.data_client.truncate(iv.inode, new_size).await?;
                iv.size = new_size;
            }
        }

        iv.ctime = ts;
        self.save_inode(inode, &iv)?;
        Ok(iv.to_attr())
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
        let _guard = self.lock_dir(parent);

        if self.index.resolve_path(parent, name)?.is_some() {
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

        self.save_inode(new_inode, &iv)?;
        self.index
            .insert_child(parent, name, new_inode, iv.to_attr())?;

        self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        )?;

        Ok(iv.to_attr())
    }

    async fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        let _guard = self.lock_dir(parent);

        if self.index.resolve_path(parent, name)?.is_some() {
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

        self.save_inode(new_inode, &iv)?;
        self.index
            .insert_child(parent, name, new_inode, iv.to_attr())?;

        self.append_parent_deltas(
            parent,
            &[
                DeltaOp::IncrementNlink(1),
                DeltaOp::SetMtime(ts),
                DeltaOp::SetCtime(ts),
            ],
        )?;

        Ok(iv.to_attr())
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
        // Collect what needs to be done under the lock, then release it
        // before any .await to keep the future Send.
        let need_delete_data = {
            let _guard = self.lock_dir(parent);

            let child_inode = self
                .index
                .resolve_path(parent, name)?
                .ok_or(FsError::NotFound)?;

            let mut child_iv = self.load_inode(child_inode)?;
            if Self::is_dir(child_iv.mode) {
                return Err(FsError::IsADirectory);
            }

            self.index.remove_child(parent, name)?;
            child_iv.nlink = child_iv.nlink.saturating_sub(1);

            if child_iv.nlink == 0 {
                self.delete_inode(child_inode)?;
                Some(child_inode)
            } else {
                let ts = now_secs();
                child_iv.ctime = ts;
                self.save_inode(child_inode, &child_iv)?;
                None
            }
        };

        // Ask DataServer to clean up file data (outside lock scope).
        if let Some(inode) = need_delete_data {
            self.data_client.delete_data(inode).await?;
        }

        let ts = now_secs();
        self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        )?;

        Ok(())
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        let _guard = self.lock_dir(parent);

        let child_inode = self
            .index
            .resolve_path(parent, name)?
            .ok_or(FsError::NotFound)?;

        let child_iv = self.load_inode(child_inode)?;
        if !Self::is_dir(child_iv.mode) {
            return Err(FsError::NotADirectory);
        }

        let entries = self.index.list_dir(child_inode)?;
        if !entries.is_empty() {
            return Err(FsError::DirectoryNotEmpty);
        }

        self.index.remove_child(parent, name)?;
        self.delete_inode(child_inode)?;

        let ts = now_secs();
        self.append_parent_deltas(
            parent,
            &[
                DeltaOp::IncrementNlink(-1),
                DeltaOp::SetMtime(ts),
                DeltaOp::SetCtime(ts),
            ],
        )?;

        Ok(())
    }

    async fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<()> {
        // Do all metadata mutations under the lock, collect any data
        // deletion needed, then release the lock before .await.
        let need_delete_data = {
            // Acquire locks in inode order to prevent deadlock.
            let (_guard1, _guard2) = if parent == new_parent {
                let g = self.lock_dir(parent);
                (g, None)
            } else {
                let (first, second) = if parent < new_parent {
                    (parent, new_parent)
                } else {
                    (new_parent, parent)
                };
                let g1 = self.lock_dir(first);
                let g2 = self.lock_dir(second);
                (g1, Some(g2))
            };

            let src_inode = self
                .index
                .resolve_path(parent, name)?
                .ok_or(FsError::NotFound)?;

            let src_iv = self.load_inode(src_inode)?;
            let src_is_dir = Self::is_dir(src_iv.mode);
            let ts = now_secs();
            let mut delete_inode: Option<Inode> = None;

            // Check if target already exists.
            if let Some(dst_inode) = self.index.resolve_path(new_parent, new_name)? {
                let dst_iv = self.load_inode(dst_inode)?;
                let dst_is_dir = Self::is_dir(dst_iv.mode);

                if src_is_dir && !dst_is_dir {
                    return Err(FsError::NotADirectory);
                }
                if !src_is_dir && dst_is_dir {
                    return Err(FsError::IsADirectory);
                }

                if dst_is_dir {
                    let entries = self.index.list_dir(dst_inode)?;
                    if !entries.is_empty() {
                        return Err(FsError::DirectoryNotEmpty);
                    }
                    self.delete_inode(dst_inode)?;
                    self.append_parent_deltas(
                        new_parent,
                        &[
                            DeltaOp::IncrementNlink(-1),
                            DeltaOp::SetMtime(ts),
                            DeltaOp::SetCtime(ts),
                        ],
                    )?;
                } else {
                    self.delete_inode(dst_inode)?;
                    delete_inode = Some(dst_inode);
                }

                self.index.remove_child(new_parent, new_name)?;
            }

            // Move the entry.
            self.index.remove_child(parent, name)?;
            self.index
                .insert_child(new_parent, new_name, src_inode, src_iv.to_attr())?;

            // Update nlink for cross-directory dir rename.
            if src_is_dir && parent != new_parent {
                self.append_parent_deltas(
                    parent,
                    &[
                        DeltaOp::IncrementNlink(-1),
                        DeltaOp::SetMtime(ts),
                        DeltaOp::SetCtime(ts),
                    ],
                )?;
                self.append_parent_deltas(
                    new_parent,
                    &[
                        DeltaOp::IncrementNlink(1),
                        DeltaOp::SetMtime(ts),
                        DeltaOp::SetCtime(ts),
                    ],
                )?;
            } else {
                self.append_parent_deltas(
                    parent,
                    &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
                )?;
                if parent != new_parent {
                    self.append_parent_deltas(
                        new_parent,
                        &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
                    )?;
                }
            }

            // Update source ctime.
            let mut src_iv = self.load_inode(src_inode)?;
            src_iv.ctime = ts;
            self.save_inode(src_inode, &src_iv)?;

            delete_inode
        };

        // Ask DataServer to clean up data (outside lock scope).
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
        let mut iv = self.load_inode(inode)?;
        if new_size > iv.size {
            iv.size = new_size;
        }
        iv.mtime = mtime;
        iv.ctime = mtime;
        self.save_inode(inode, &iv)?;
        Ok(())
    }
}
