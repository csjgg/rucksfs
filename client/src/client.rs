use async_trait::async_trait;
use rucksfs_core::{ClientOps, DirEntry, FileAttr, FsResult, Inode, StatFs};
use std::sync::Arc;

/// `Client` is a type alias for `ClientOps`, maintained for backward compatibility.
/// All client implementations should implement `ClientOps` from `rucksfs_core`.
pub use rucksfs_core::ClientOps as Client;

/// In-process client: directly delegates to any `ClientOps` implementation.
pub struct InProcessClient<S>
where
    S: ClientOps,
{
    server: Arc<S>,
}

impl<S> InProcessClient<S>
where
    S: ClientOps,
{
    pub fn new(server: Arc<S>) -> Self {
        Self { server }
    }
}

#[async_trait]
impl<S> ClientOps for InProcessClient<S>
where
    S: ClientOps,
{
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
        self.server.lookup(parent, name).await
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        self.server.getattr(inode).await
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        self.server.readdir(inode).await
    }

    async fn open(&self, inode: Inode, flags: u32) -> FsResult<u64> {
        self.server.open(inode, flags).await
    }

    async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        self.server.read(inode, offset, size).await
    }

    async fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32> {
        self.server.write(inode, offset, data, flags).await
    }

    async fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        self.server.create(parent, name, mode).await
    }

    async fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        self.server.mkdir(parent, name, mode).await
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
        self.server.unlink(parent, name).await
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        self.server.rmdir(parent, name).await
    }

    async fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<()> {
        self.server
            .rename(parent, name, new_parent, new_name)
            .await
    }

    async fn setattr(&self, inode: Inode, attr: FileAttr) -> FsResult<FileAttr> {
        self.server.setattr(inode, attr).await
    }

    async fn statfs(&self, inode: Inode) -> FsResult<StatFs> {
        self.server.statfs(inode).await
    }

    async fn flush(&self, inode: Inode) -> FsResult<()> {
        self.server.flush(inode).await
    }

    async fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()> {
        self.server.fsync(inode, datasync).await
    }
}

/// Build a Client from any `ClientOps` implementation (in-process or RPC).
pub fn build_client<S: ClientOps>(ops: Arc<S>) -> InProcessClient<S> {
    InProcessClient::new(ops)
}
