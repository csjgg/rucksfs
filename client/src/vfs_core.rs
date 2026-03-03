//! VFS core routing logic.
//!
//! Routes metadata requests to `MetadataOps` and data requests to `DataOps`.
//! Shared by both `EmbeddedClient` and `RucksClient`.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use rucksfs_core::{
    DataOps, DirEntry, FileAttr, FsResult, Inode, MetadataOps, SetAttrRequest, StatFs, VfsOps,
};
use std::sync::Arc;

/// Core VFS router that delegates to MetadataOps and DataOps.
pub struct VfsCore {
    metadata: Arc<dyn MetadataOps>,
    data: Arc<dyn DataOps>,
    /// Cache of handle → DataLocation (currently unused in single-DataServer
    /// mode, but kept for future multi-DataServer support).
    #[allow(dead_code)]
    handle_cache: Mutex<HashMap<u64, String>>,
}

impl VfsCore {
    pub fn new(metadata: Arc<dyn MetadataOps>, data: Arc<dyn DataOps>) -> Self {
        Self {
            metadata,
            data,
            handle_cache: Mutex::new(HashMap::new()),
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[async_trait]
impl VfsOps for VfsCore {
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
        self.metadata.lookup(parent, name).await
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        self.metadata.getattr(inode).await
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        self.metadata.readdir(inode).await
    }

    async fn create(&self, parent: Inode, name: &str, mode: u32, uid: u32, gid: u32) -> FsResult<FileAttr> {
        self.metadata.create(parent, name, mode, uid, gid).await
    }

    async fn mkdir(&self, parent: Inode, name: &str, mode: u32, uid: u32, gid: u32) -> FsResult<FileAttr> {
        self.metadata.mkdir(parent, name, mode, uid, gid).await
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
        self.metadata.unlink(parent, name).await
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        self.metadata.rmdir(parent, name).await
    }

    async fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<()> {
        self.metadata
            .rename(parent, name, new_parent, new_name)
            .await
    }

    async fn setattr(&self, inode: Inode, req: SetAttrRequest) -> FsResult<FileAttr> {
        self.metadata.setattr(inode, req).await
    }

    async fn statfs(&self, inode: Inode) -> FsResult<StatFs> {
        self.metadata.statfs(inode).await
    }

    async fn open(&self, inode: Inode, flags: u32) -> FsResult<u64> {
        let resp = self.metadata.open(inode, flags).await?;
        // Cache the DataLocation for this handle (for future multi-DataServer).
        {
            let mut cache = self.handle_cache.lock().expect("handle_cache poisoned");
            cache.insert(resp.handle, resp.data_location.address);
        }
        Ok(resp.handle)
    }

    async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        // Read directly from DataServer, bypassing MetadataServer.
        self.data.read_data(inode, offset, size).await
    }

    async fn write(&self, inode: Inode, offset: u64, data: &[u8], _flags: u32) -> FsResult<u32> {
        // Write directly to DataServer.
        let written = self.data.write_data(inode, offset, data).await?;
        // Report the write back to MetadataServer to update size/mtime.
        let new_end = offset + written as u64;
        let ts = now_secs();
        self.metadata.report_write(inode, new_end, ts).await?;
        Ok(written)
    }

    async fn flush(&self, inode: Inode) -> FsResult<()> {
        self.data.flush(inode).await
    }

    async fn fsync(&self, inode: Inode, _datasync: bool) -> FsResult<()> {
        self.data.flush(inode).await
    }
}
