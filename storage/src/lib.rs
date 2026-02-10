use async_trait::async_trait;
use rucksfs_core::{DirEntry, FileAttr, FsResult, Inode};

pub mod dummy;
pub mod encoding;

pub use dummy::{DummyDataStore, DummyDirectoryIndex, DummyMetadataStore};

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
