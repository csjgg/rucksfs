use async_trait::async_trait;
use rucksfs_core::{DirEntry, FileAttr, FsResult, Inode};

pub mod allocator;
pub mod encoding;
pub mod memory;
pub mod rawdisk;

#[cfg(feature = "rocksdb")]
pub mod rocks;

pub use allocator::InodeAllocator;
pub use memory::{MemoryDataStore, MemoryDeltaStore, MemoryDirectoryIndex, MemoryMetadataStore};
pub use rawdisk::RawDiskDataStore;

#[cfg(feature = "rocksdb")]
pub use rocks::{open_rocks_db, RocksDirectoryIndex, RocksMetadataStore};

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

    /// Delete all deltas for `inode` (called after compaction merges them
    /// into the base inode value).
    fn clear_deltas(&self, inode: Inode) -> FsResult<()>;
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
}

pub trait DirectoryIndex: Send + Sync {
    fn resolve_path(&self, parent: Inode, name: &str) -> FsResult<Option<Inode>>;
    fn list_dir(&self, inode: Inode) -> FsResult<Vec<DirEntry>>;
    fn insert_child(&self, parent: Inode, name: &str, inode: Inode, attr: FileAttr) -> FsResult<()>;
    fn remove_child(&self, parent: Inode, name: &str) -> FsResult<()>;
}
