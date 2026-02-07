use async_trait::async_trait;
use rucksfs_core::{ClientOps, DirEntry, FileAttr, FsError, FsResult, Inode, PosixOps, StatFs};
use rucksfs_storage::{DataStore, DirectoryIndex, MetadataStore};
use std::sync::Arc;

pub struct MetadataServer<M, D, I>
where
    M: MetadataStore,
    D: DataStore,
    I: DirectoryIndex,
{
    pub metadata: Arc<M>,
    pub data: Arc<D>,
    pub index: Arc<I>,
}

impl<M, D, I> MetadataServer<M, D, I>
where
    M: MetadataStore,
    D: DataStore,
    I: DirectoryIndex,
{
    pub fn new(metadata: Arc<M>, data: Arc<D>, index: Arc<I>) -> Self {
        Self {
            metadata,
            data,
            index,
        }
    }
}

impl<M, D, I> PosixOps for MetadataServer<M, D, I>
where
    M: MetadataStore,
    D: DataStore,
    I: DirectoryIndex,
{
    fn lookup(&self, _parent: Inode, _name: &str) -> FsResult<FileAttr> {
        Err(FsError::NotImplemented)
    }

    fn getattr(&self, _inode: Inode) -> FsResult<FileAttr> {
        Err(FsError::NotImplemented)
    }

    fn readdir(&self, _inode: Inode) -> FsResult<Vec<DirEntry>> {
        Err(FsError::NotImplemented)
    }

    fn open(&self, _inode: Inode, _flags: u32) -> FsResult<u64> {
        Err(FsError::NotImplemented)
    }

    fn read(&self, _inode: Inode, _offset: u64, _size: u32) -> FsResult<Vec<u8>> {
        Err(FsError::NotImplemented)
    }

    fn write(&self, _inode: Inode, _offset: u64, _data: &[u8], _flags: u32) -> FsResult<u32> {
        Err(FsError::NotImplemented)
    }

    fn create(&self, _parent: Inode, _name: &str, _mode: u32) -> FsResult<FileAttr> {
        Err(FsError::NotImplemented)
    }

    fn mkdir(&self, _parent: Inode, _name: &str, _mode: u32) -> FsResult<FileAttr> {
        Err(FsError::NotImplemented)
    }

    fn unlink(&self, _parent: Inode, _name: &str) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }

    fn rmdir(&self, _parent: Inode, _name: &str) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }

    fn rename(&self, _parent: Inode, _name: &str, _new_parent: Inode, _new_name: &str) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }

    fn setattr(&self, _inode: Inode, _attr: FileAttr) -> FsResult<FileAttr> {
        Err(FsError::NotImplemented)
    }

    fn statfs(&self, _inode: Inode) -> FsResult<StatFs> {
        Err(FsError::NotImplemented)
    }

    fn flush(&self, _inode: Inode) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }

    fn fsync(&self, _inode: Inode, _datasync: bool) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }
}

#[async_trait]
impl<M, D, I> ClientOps for MetadataServer<M, D, I>
where
    M: MetadataStore,
    D: DataStore,
    I: DirectoryIndex,
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

    async fn rename(&self, parent: Inode, name: &str, new_parent: Inode, new_name: &str) -> FsResult<()> {
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
