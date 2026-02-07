use async_trait::async_trait;
use rucksfs_core::{DirEntry, FileAttr, FsError, FsResult, Inode};

/// Dummy metadata store for testing/demo purposes.
/// All operations return NotImplemented.
pub struct DummyMetadataStore;

impl super::MetadataStore for DummyMetadataStore {
    fn get(&self, _key: &[u8]) -> FsResult<Option<Vec<u8>>> {
        Err(FsError::NotImplemented)
    }

    fn put(&self, _key: &[u8], _value: &[u8]) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }

    fn delete(&self, _key: &[u8]) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }

    fn scan_prefix(&self, _prefix: &[u8]) -> FsResult<Vec<(Vec<u8>, Vec<u8>)>> {
        Err(FsError::NotImplemented)
    }
}

/// Dummy directory index for testing/demo purposes.
/// All operations return NotImplemented.
pub struct DummyDirectoryIndex;

impl super::DirectoryIndex for DummyDirectoryIndex {
    fn resolve_path(&self, _parent: Inode, _name: &str) -> FsResult<Option<Inode>> {
        Err(FsError::NotImplemented)
    }

    fn list_dir(&self, _inode: Inode) -> FsResult<Vec<DirEntry>> {
        Err(FsError::NotImplemented)
    }

    fn insert_child(&self, _parent: Inode, _name: &str, _inode: Inode, _attr: FileAttr) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }

    fn remove_child(&self, _parent: Inode, _name: &str) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }
}

/// Dummy data store for testing/demo purposes.
/// All operations return NotImplemented.
pub struct DummyDataStore;

#[async_trait]
impl super::DataStore for DummyDataStore {
    async fn read_at(&self, _inode: Inode, _offset: u64, _size: u32) -> FsResult<Vec<u8>> {
        Err(FsError::NotImplemented)
    }

    async fn write_at(&self, _inode: Inode, _offset: u64, _data: &[u8]) -> FsResult<u32> {
        Err(FsError::NotImplemented)
    }

    async fn truncate(&self, _inode: Inode, _size: u64) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }

    async fn flush(&self, _inode: Inode) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }
}
