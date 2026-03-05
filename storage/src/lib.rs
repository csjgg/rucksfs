use async_trait::async_trait;
use rucksfs_core::{DirEntry, FileAttr, FsResult, Inode};

pub mod allocator;
pub mod encoding;
pub mod rawdisk;
pub mod rocks;

pub use allocator::InodeAllocator;
pub use rawdisk::RawDiskDataStore;
pub use rocks::{open_rocks_db, RocksDeltaStore, RocksDirectoryIndex, RocksMetadataStore, RocksStorageBundle};

/// Append-only delta store for incremental inode attribute updates.
///
/// Implementations work at the raw-byte level.  The caller (server layer)
/// is responsible for encoding/decoding [`DeltaOp`] values.
///
/// Each delta is identified by `(inode, seq)` where `seq` is a per-inode
/// monotonically increasing sequence number.
pub trait DeltaStore: Send + Sync {
    /// Atomically append one or more serialized delta values to the given
    /// `inode`.  Returns the sequence numbers assigned to each delta.
    fn append_deltas(&self, inode: Inode, values: &[Vec<u8>]) -> FsResult<Vec<u64>>;

    /// Scan all pending (un-compacted) deltas for `inode`, returning them
    /// in sequence-number order as raw bytes.
    fn scan_deltas(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>>;

    /// Scan all pending delta keys for `inode`, returning the raw key bytes.
    /// Used by compaction to build atomic delete batches.
    fn scan_delta_keys(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>>;

    /// Delete all deltas for `inode` (called after compaction merges them
    /// into the base inode value).
    fn clear_deltas(&self, inode: Inode) -> FsResult<()>;

    /// Allocate the next sequence number for `inode`. Used by the server
    /// layer to write delta entries inside a transaction.
    fn next_seq(&self, inode: Inode) -> u64;

    /// Scan all pending deltas for `inode`, returning `(key, value)` pairs
    /// from a single consistent iterator pass. Used by compaction to ensure
    /// the set of keys deleted matches exactly the set of values folded.
    fn scan_deltas_with_keys(&self, inode: Inode) -> FsResult<Vec<(Vec<u8>, Vec<u8>)>>;
}

pub trait MetadataStore: Send + Sync {
    fn get(&self, key: &[u8]) -> FsResult<Option<Vec<u8>>>;
    fn put(&self, key: &[u8], value: &[u8]) -> FsResult<()>;
    fn delete(&self, key: &[u8]) -> FsResult<()>;
    fn scan_prefix(&self, prefix: &[u8]) -> FsResult<Vec<(Vec<u8>, Vec<u8>)>>;
}

#[async_trait]
pub trait DataStore: Send + Sync {
    async fn read_at(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>>;
    async fn write_at(&self, inode: Inode, offset: u64, data: &[u8]) -> FsResult<u32>;
    async fn truncate(&self, inode: Inode, size: u64) -> FsResult<()>;
    async fn flush(&self, inode: Inode) -> FsResult<()>;
    /// Delete all data for the given inode (used for GC / unlink).
    async fn delete(&self, inode: Inode) -> FsResult<()>;
}

pub trait DirectoryIndex: Send + Sync {
    fn resolve_path(&self, parent: Inode, name: &str) -> FsResult<Option<Inode>>;
    fn list_dir(&self, inode: Inode) -> FsResult<Vec<DirEntry>>;
    fn insert_child(&self, parent: Inode, name: &str, inode: Inode, attr: FileAttr) -> FsResult<()>;
    fn remove_child(&self, parent: Inode, name: &str) -> FsResult<()>;

    /// Whether this directory index shares the same storage backend as the
    /// [`AtomicWriteBatch`].  When `true`, dir-entry mutations committed via
    /// the batch are already visible through this index, making post-commit
    /// `insert_child` / `remove_child` calls redundant.
    fn shares_batch_storage(&self) -> bool {
        false
    }
}

// ===========================================================================
// Atomic write batch abstraction
// ===========================================================================

/// Operation types that can be collected into an atomic write batch.
///
/// Each variant corresponds to a write to a specific column family / store.
#[derive(Debug, Clone)]
pub enum BatchOp {
    /// Put an inode value: CF:inodes
    PutInode { key: Vec<u8>, value: Vec<u8> },
    /// Delete an inode: CF:inodes
    DeleteInode { key: Vec<u8> },
    /// Put a directory entry: CF:dir_entries
    PutDirEntry { key: Vec<u8>, value: Vec<u8> },
    /// Delete a directory entry: CF:dir_entries
    DeleteDirEntry { key: Vec<u8> },
    /// Put a delta entry: CF:delta_entries
    PutDelta { key: Vec<u8>, value: Vec<u8> },
    /// Delete a delta entry: CF:delta_entries
    DeleteDelta { key: Vec<u8> },
    /// Put a system key: CF:system (via MetadataStore, e.g. next_inode)
    PutSystem { key: Vec<u8>, value: Vec<u8> },
}

/// A batch of write operations that will be committed atomically.
///
/// Maps to a RocksDB `Transaction` spanning multiple column families.
pub trait AtomicWriteBatch: Send {
    /// Add an operation to the batch.
    fn push(&mut self, op: BatchOp);

    /// Commit all collected operations atomically.
    ///
    /// After a successful commit, all operations are visible. If the process
    /// crashes before commit returns, none of the operations are visible.
    fn commit(self: Box<Self>) -> FsResult<()>;

    /// Read an inode value inside the transaction and acquire a pessimistic
    /// row lock on the key (PCC: `GetForUpdate`).
    ///
    /// Calls `Transaction::get_for_update_cf` on the inodes column family.
    fn get_for_update_inode(&self, _key: &[u8]) -> FsResult<Option<Vec<u8>>> {
        unimplemented!("get_for_update_inode not supported by this backend")
    }

    /// Read a directory-entry value inside the transaction and acquire a
    /// pessimistic row lock on the key (PCC: `GetForUpdate`).
    ///
    /// Calls `Transaction::get_for_update_cf` on the dir_entries column family.
    fn get_for_update_dir_entry(&self, _key: &[u8]) -> FsResult<Option<Vec<u8>>> {
        unimplemented!("get_for_update_dir_entry not supported by this backend")
    }

    /// Check whether a directory has any child entries, reading inside the
    /// transaction's snapshot to avoid TOCTOU races.
    ///
    /// Uses a prefix scan over the `dir_entries` CF within the transaction,
    /// returning `true` if no children exist.
    fn is_dir_empty(&self, _parent: Inode) -> FsResult<bool> {
        unimplemented!("is_dir_empty not supported by this backend")
    }
}

/// Trait for storage backends that support atomic cross-store writes.
///
/// Implemented by a "bundle" that owns references to all underlying stores
/// (metadata, directory index, delta store) and can create an
/// [`AtomicWriteBatch`] that spans all of them.
pub trait StorageBundle: Send + Sync {
    /// Begin a new atomic write batch.
    fn begin_write(&self) -> Box<dyn AtomicWriteBatch + '_>;
}
