//! Metadata server — orchestrates storage backends to implement POSIX semantics.

pub mod cache;
pub mod delta;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use rucksfs_core::{ClientOps, DirEntry, FileAttr, FsError, FsResult, Inode, PosixOps, StatFs};
use rucksfs_storage::allocator::{InodeAllocator, ROOT_INODE};
use rucksfs_storage::encoding::{encode_inode_key, InodeValue};
use rucksfs_storage::{DataStore, DeltaStore, DirectoryIndex, MetadataStore};

use crate::cache::InodeFoldedCache;
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

/// Core metadata server that composes [`MetadataStore`], [`DataStore`],
/// [`DirectoryIndex`], and [`DeltaStore`] to implement full POSIX
/// file-system operations.
///
/// Parent-directory attribute updates (nlink, mtime, ctime) are written
/// as append-only delta entries rather than read-modify-write, following
/// the Mantle delta-entries design.
pub struct MetadataServer<M, D, I, DS>
where
    M: MetadataStore,
    D: DataStore,
    I: DirectoryIndex,
    DS: DeltaStore,
{
    pub metadata: Arc<M>,
    pub data: Arc<D>,
    pub index: Arc<I>,
    pub delta_store: Arc<DS>,
    /// LRU cache of folded inode values.
    pub cache: Arc<InodeFoldedCache>,
    allocator: InodeAllocator,
    /// Per-directory lock to serialize mutations under the same parent.
    dir_locks: Mutex<HashMap<Inode, Arc<Mutex<()>>>>,
}

impl<M, D, I, DS> MetadataServer<M, D, I, DS>
where
    M: MetadataStore,
    D: DataStore,
    I: DirectoryIndex,
    DS: DeltaStore,
{
    /// Create a new `MetadataServer` and initialise the root directory
    /// (inode 1) if it does not already exist.
    pub fn new(
        metadata: Arc<M>,
        data: Arc<D>,
        index: Arc<I>,
        delta_store: Arc<DS>,
    ) -> Self {
        let allocator = InodeAllocator::load(metadata.as_ref())
            .unwrap_or_else(|_| InodeAllocator::new());

        let server = Self {
            metadata,
            data,
            index,
            delta_store,
            cache: Arc::new(InodeFoldedCache::new(DEFAULT_CACHE_CAPACITY)),
            allocator,
            dir_locks: Mutex::new(HashMap::new()),
        };

        // Ensure root directory exists.
        server.init_root();
        server
    }

    /// Create a new `MetadataServer` with a custom cache capacity.
    pub fn with_cache_capacity(
        metadata: Arc<M>,
        data: Arc<D>,
        index: Arc<I>,
        delta_store: Arc<DS>,
        cache_capacity: usize,
    ) -> Self {
        let allocator = InodeAllocator::load(metadata.as_ref())
            .unwrap_or_else(|_| InodeAllocator::new());

        let server = Self {
            metadata,
            data,
            index,
            delta_store,
            cache: Arc::new(InodeFoldedCache::new(cache_capacity)),
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

    /// Load and deserialize an inode from the metadata store.
    fn load_inode(&self, inode: Inode) -> FsResult<InodeValue> {
        let key = encode_inode_key(inode);
        match self.metadata.get(&key)? {
            Some(bytes) => InodeValue::deserialize(&bytes),
            None => Err(FsError::NotFound),
        }
    }

    /// Serialize and save an inode to the metadata store.
    fn save_inode(&self, inode: Inode, val: &InodeValue) -> FsResult<()> {
        let key = encode_inode_key(inode);
        self.metadata.put(&key, &val.serialize())
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
    ///
    /// This replaces the old read-modify-write pattern for parent attribute
    /// updates.  The deltas are persisted to the `DeltaStore` and applied
    /// to the in-memory cache in a single logical step.
    fn append_parent_deltas(&self, parent: Inode, deltas: &[DeltaOp]) -> FsResult<()> {
        // Serialize and persist.
        let serialized: Vec<Vec<u8>> = deltas.iter().map(|d| d.serialize()).collect();
        self.delta_store.append_deltas(parent, &serialized)?;

        // Update the cache.  If the parent is not cached, the next read
        // will do a full fold.  This keeps the hot path fast.
        self.cache.apply_deltas(parent, deltas);
        Ok(())
    }

    // -- helper: per-directory lock -----------------------------------------

    /// Acquire the per-directory mutex for `parent`.
    ///
    /// Returns a [`DirLockGuard`] that owns the `Arc<Mutex<()>>` so the
    /// borrow is valid for the guard's lifetime.
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
// PosixOps implementation
// ---------------------------------------------------------------------------

impl<M, D, I, DS> PosixOps for MetadataServer<M, D, I, DS>
where
    M: MetadataStore,
    D: DataStore,
    I: DirectoryIndex,
    DS: DeltaStore,
{
    fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
        let child_inode = self
            .index
            .resolve_path(parent, name)?
            .ok_or(FsError::NotFound)?;
        let iv = self.load_inode(child_inode)?;
        Ok(iv.to_attr())
    }

    fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        let iv = self.load_inode(inode)?;
        Ok(iv.to_attr())
    }

    fn setattr(&self, inode: Inode, attr: FileAttr) -> FsResult<FileAttr> {
        let mut iv = self.load_inode(inode)?;
        let ts = now_secs();

        // Selectively update non-zero fields
        if attr.mode != 0 {
            iv.mode = attr.mode;
        }
        if attr.uid != 0 {
            iv.uid = attr.uid;
        }
        if attr.gid != 0 {
            iv.gid = attr.gid;
        }
        if attr.atime != 0 {
            iv.atime = attr.atime;
        }
        if attr.mtime != 0 {
            iv.mtime = attr.mtime;
        }
        // Handle size change → truncate data
        if attr.size != iv.size {
            // Use tokio runtime to call async truncate synchronously
            let inode_id = iv.inode;
            let new_size = attr.size;
            let data = Arc::clone(&self.data);
            // Run async truncate on current thread
            let rt = tokio::runtime::Handle::try_current();
            match rt {
                Ok(handle) => {
                    handle
                        .block_on(async { data.truncate(inode_id, new_size).await })?;
                }
                Err(_) => {
                    // No async runtime; create a blocking one
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| FsError::Io(e.to_string()))?;
                    rt.block_on(async { data.truncate(inode_id, new_size).await })?;
                }
            }
            iv.size = attr.size;
        }

        iv.ctime = ts;
        self.save_inode(inode, &iv)?;
        Ok(iv.to_attr())
    }

    fn statfs(&self, _inode: Inode) -> FsResult<StatFs> {
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

    fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        let iv = self.load_inode(inode)?;
        if !Self::is_dir(iv.mode) {
            return Err(FsError::NotADirectory);
        }
        self.index.list_dir(inode)
    }

    fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        let _guard = self.lock_dir(parent);

        // Check for duplicate
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

        // Update parent times via delta append (replaces read-modify-write).
        self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        )?;

        Ok(iv.to_attr())
    }

    fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
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
            nlink: 2, // "." and parent entry
            uid: 0,
            gid: 0,
            atime: ts,
            mtime: ts,
            ctime: ts,
        };

        self.save_inode(new_inode, &iv)?;
        self.index
            .insert_child(parent, name, new_inode, iv.to_attr())?;

        // Parent nlink +1 (for the ".." in new dir) + update times via delta.
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

    fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
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
        } else {
            let ts = now_secs();
            child_iv.ctime = ts;
            self.save_inode(child_inode, &child_iv)?;
        }

        // Update parent times via delta append.
        let ts = now_secs();
        self.append_parent_deltas(
            parent,
            &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
        )?;

        Ok(())
    }

    fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        let _guard = self.lock_dir(parent);

        let child_inode = self
            .index
            .resolve_path(parent, name)?
            .ok_or(FsError::NotFound)?;

        let child_iv = self.load_inode(child_inode)?;
        if !Self::is_dir(child_iv.mode) {
            return Err(FsError::NotADirectory);
        }

        // Check if directory is empty
        let entries = self.index.list_dir(child_inode)?;
        if !entries.is_empty() {
            return Err(FsError::DirectoryNotEmpty);
        }

        self.index.remove_child(parent, name)?;
        self.delete_inode(child_inode)?;

        // Parent nlink -1 + update times via delta.
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

    fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<()> {
        // Acquire locks in inode order to prevent deadlock
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

        // Source must exist
        let src_inode = self
            .index
            .resolve_path(parent, name)?
            .ok_or(FsError::NotFound)?;

        let src_iv = self.load_inode(src_inode)?;
        let src_is_dir = Self::is_dir(src_iv.mode);
        let ts = now_secs();

        // Check if target already exists
        if let Some(dst_inode) = self.index.resolve_path(new_parent, new_name)? {
            let dst_iv = self.load_inode(dst_inode)?;
            let dst_is_dir = Self::is_dir(dst_iv.mode);

            // Cannot overwrite dir with file or file with dir
            if src_is_dir && !dst_is_dir {
                return Err(FsError::NotADirectory);
            }
            if !src_is_dir && dst_is_dir {
                return Err(FsError::IsADirectory);
            }

            // If target is a dir, it must be empty
            if dst_is_dir {
                let entries = self.index.list_dir(dst_inode)?;
                if !entries.is_empty() {
                    return Err(FsError::DirectoryNotEmpty);
                }
                // Remove the target dir
                self.delete_inode(dst_inode)?;
                // Adjust new_parent nlink via delta.
                self.append_parent_deltas(
                    new_parent,
                    &[
                        DeltaOp::IncrementNlink(-1),
                        DeltaOp::SetMtime(ts),
                        DeltaOp::SetCtime(ts),
                    ],
                )?;
            } else {
                // Remove the target file
                self.delete_inode(dst_inode)?;
            }

            self.index.remove_child(new_parent, new_name)?;
        }

        // Move the entry
        self.index.remove_child(parent, name)?;
        self.index
            .insert_child(new_parent, new_name, src_inode, src_iv.to_attr())?;

        // Update nlink for cross-directory dir rename
        if src_is_dir && parent != new_parent {
            // Old parent loses a ".." reference
            self.append_parent_deltas(
                parent,
                &[
                    DeltaOp::IncrementNlink(-1),
                    DeltaOp::SetMtime(ts),
                    DeltaOp::SetCtime(ts),
                ],
            )?;

            // New parent gains a ".." reference
            self.append_parent_deltas(
                new_parent,
                &[
                    DeltaOp::IncrementNlink(1),
                    DeltaOp::SetMtime(ts),
                    DeltaOp::SetCtime(ts),
                ],
            )?;
        } else {
            // Same parent: just update times
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

        // Update source ctime
        let mut src_iv = self.load_inode(src_inode)?;
        src_iv.ctime = ts;
        self.save_inode(src_inode, &src_iv)?;

        Ok(())
    }

    fn open(&self, inode: Inode, _flags: u32) -> FsResult<u64> {
        let iv = self.load_inode(inode)?;
        if Self::is_dir(iv.mode) {
            return Err(FsError::IsADirectory);
        }
        // Return 0 as file handle (we don't track open files)
        Ok(0)
    }

    fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        let iv = self.load_inode(inode)?;

        // Clamp to file size
        if offset >= iv.size {
            return Ok(Vec::new());
        }
        let available = iv.size - offset;
        let actual_size = (size as u64).min(available) as u32;

        // Call async DataStore synchronously
        let data = Arc::clone(&self.data);
        let result = Self::block_on_async(async move {
            data.read_at(inode, offset, actual_size).await
        })?;

        Ok(result)
    }

    fn write(&self, inode: Inode, offset: u64, data: &[u8], _flags: u32) -> FsResult<u32> {
        let data_store = Arc::clone(&self.data);
        let data_vec = data.to_vec();
        let written = Self::block_on_async(async move {
            data_store.write_at(inode, offset, &data_vec).await
        })?;

        // Update inode metadata
        let mut iv = self.load_inode(inode)?;
        let new_end = offset + written as u64;
        if new_end > iv.size {
            iv.size = new_end;
        }
        let ts = now_secs();
        iv.mtime = ts;
        iv.ctime = ts;
        self.save_inode(inode, &iv)?;

        Ok(written)
    }

    fn flush(&self, inode: Inode) -> FsResult<()> {
        let data = Arc::clone(&self.data);
        Self::block_on_async(async move { data.flush(inode).await })
    }

    fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()> {
        let data = Arc::clone(&self.data);
        Self::block_on_async(async move { data.flush(inode).await })?;

        if !datasync {
            // Ensure metadata is also persisted (re-save inode)
            let iv = self.load_inode(inode)?;
            self.save_inode(inode, &iv)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Async bridge
// ---------------------------------------------------------------------------

impl<M, D, I, DS> MetadataServer<M, D, I, DS>
where
    M: MetadataStore,
    D: DataStore,
    I: DirectoryIndex,
    DS: DeltaStore,
{
    /// Execute an async future synchronously.
    ///
    /// Tries to use the current tokio runtime; falls back to creating a
    /// temporary one-shot runtime.
    fn block_on_async<F, T>(fut: F) -> FsResult<T>
    where
        F: std::future::Future<Output = FsResult<T>>,
    {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                // We're inside an async context – must not block.
                // Use `block_in_place` if multi-threaded, otherwise spawn a task.
                tokio::task::block_in_place(|| handle.block_on(fut))
            }
            Err(_) => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| FsError::Io(e.to_string()))?;
                rt.block_on(fut)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ClientOps (async wrapper)
// ---------------------------------------------------------------------------

#[async_trait]
impl<M, D, I, DS> ClientOps for MetadataServer<M, D, I, DS>
where
    M: MetadataStore,
    D: DataStore,
    I: DirectoryIndex,
    DS: DeltaStore,
{
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
        PosixOps::lookup(self, parent, name)
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        PosixOps::getattr(self, inode)
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        PosixOps::readdir(self, inode)
    }

    async fn open(&self, inode: Inode, flags: u32) -> FsResult<u64> {
        PosixOps::open(self, inode, flags)
    }

    async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        PosixOps::read(self, inode, offset, size)
    }

    async fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32> {
        PosixOps::write(self, inode, offset, data, flags)
    }

    async fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        PosixOps::create(self, parent, name, mode)
    }

    async fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        PosixOps::mkdir(self, parent, name, mode)
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
        PosixOps::unlink(self, parent, name)
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        PosixOps::rmdir(self, parent, name)
    }

    async fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<()> {
        PosixOps::rename(self, parent, name, new_parent, new_name)
    }

    async fn setattr(&self, inode: Inode, attr: FileAttr) -> FsResult<FileAttr> {
        PosixOps::setattr(self, inode, attr)
    }

    async fn statfs(&self, inode: Inode) -> FsResult<StatFs> {
        PosixOps::statfs(self, inode)
    }

    async fn flush(&self, inode: Inode) -> FsResult<()> {
        PosixOps::flush(self, inode)
    }

    async fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()> {
        PosixOps::fsync(self, inode, datasync)
    }
}
