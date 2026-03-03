//! EmbeddedClient — in-process client for testing and demo.
//!
//! Directly holds `Arc<dyn MetadataOps>` and `Arc<dyn DataOps>` references,
//! using the shared `VfsCore` routing logic without any network overhead.

use async_trait::async_trait;
use rucksfs_core::{
    DataOps, DirEntry, FileAttr, FsResult, Inode, MetadataOps, SetAttrRequest, StatFs, VfsOps,
};
use std::sync::Arc;

use crate::vfs_core::VfsCore;

/// In-process client that embeds MetadataServer and DataServer references.
///
/// Uses the same VFS routing logic as `RucksClient`, but communicates
/// via direct function calls instead of network RPC.
pub struct EmbeddedClient {
    vfs: VfsCore,
}

impl EmbeddedClient {
    /// Create a new `EmbeddedClient` from metadata and data service references.
    pub fn new(metadata: Arc<dyn MetadataOps>, data: Arc<dyn DataOps>) -> Self {
        Self {
            vfs: VfsCore::new(metadata, data),
        }
    }
}

#[async_trait]
impl VfsOps for EmbeddedClient {
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
        self.vfs.lookup(parent, name).await
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        self.vfs.getattr(inode).await
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        self.vfs.readdir(inode).await
    }

    async fn create(&self, parent: Inode, name: &str, mode: u32, uid: u32, gid: u32) -> FsResult<FileAttr> {
        self.vfs.create(parent, name, mode, uid, gid).await
    }

    async fn mkdir(&self, parent: Inode, name: &str, mode: u32, uid: u32, gid: u32) -> FsResult<FileAttr> {
        self.vfs.mkdir(parent, name, mode, uid, gid).await
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
        self.vfs.unlink(parent, name).await
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        self.vfs.rmdir(parent, name).await
    }

    async fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<()> {
        self.vfs.rename(parent, name, new_parent, new_name).await
    }

    async fn setattr(&self, inode: Inode, req: SetAttrRequest) -> FsResult<FileAttr> {
        self.vfs.setattr(inode, req).await
    }

    async fn statfs(&self, inode: Inode) -> FsResult<StatFs> {
        self.vfs.statfs(inode).await
    }

    async fn open(&self, inode: Inode, flags: u32) -> FsResult<u64> {
        self.vfs.open(inode, flags).await
    }

    async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        self.vfs.read(inode, offset, size).await
    }

    async fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32> {
        self.vfs.write(inode, offset, data, flags).await
    }

    async fn flush(&self, inode: Inode) -> FsResult<()> {
        self.vfs.flush(inode).await
    }

    async fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()> {
        self.vfs.fsync(inode, datasync).await
    }
}
