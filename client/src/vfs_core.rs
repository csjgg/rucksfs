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
    default_data: Arc<dyn DataOps>,
    /// Registry of DataServer addresses to their DataOps implementations.
    /// Used for routing read/write to the correct DataServer.
    data_servers: Mutex<HashMap<String, Arc<dyn DataOps>>>,
    /// Maps open file handles (inode) to their DataServer address.
    handle_map: Mutex<HashMap<u64, String>>,
}

impl VfsCore {
    pub fn new(metadata: Arc<dyn MetadataOps>, data: Arc<dyn DataOps>) -> Self {
        Self {
            metadata,
            default_data: data,
            data_servers: Mutex::new(HashMap::new()),
            handle_map: Mutex::new(HashMap::new()),
        }
    }

    /// Create a VfsCore with additional DataServer registrations.
    pub fn with_data_servers(
        metadata: Arc<dyn MetadataOps>,
        default_data: Arc<dyn DataOps>,
        data_servers: HashMap<String, Arc<dyn DataOps>>,
    ) -> Self {
        Self {
            metadata,
            default_data,
            data_servers: Mutex::new(data_servers),
            handle_map: Mutex::new(HashMap::new()),
        }
    }

    /// Look up the DataOps for a given inode based on its open handle mapping.
    /// Falls back to default_data if the inode has no mapping or the address
    /// is not in data_servers.
    fn resolve_data(&self, inode: u64) -> Arc<dyn DataOps> {
        let handle_map = self.handle_map.lock().expect("handle_map poisoned");
        if let Some(address) = handle_map.get(&inode) {
            let servers = self.data_servers.lock().expect("data_servers poisoned");
            if let Some(ds) = servers.get(address) {
                return Arc::clone(ds);
            }
        }
        Arc::clone(&self.default_data)
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
        {
            let mut map = self.handle_map.lock().expect("handle_map poisoned");
            map.insert(resp.handle, resp.data_location.address);
        }
        Ok(resp.handle)
    }

    async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        self.resolve_data(inode).read_data(inode, offset, size).await
    }

    async fn write(&self, inode: Inode, offset: u64, data: &[u8], _flags: u32) -> FsResult<u32> {
        let ds = self.resolve_data(inode);
        let written = ds.write_data(inode, offset, data).await?;
        let new_end = offset + written as u64;
        let ts = now_secs();
        self.metadata.report_write(inode, new_end, ts).await?;
        Ok(written)
    }

    async fn flush(&self, inode: Inode) -> FsResult<()> {
        self.resolve_data(inode).flush(inode).await
    }

    async fn fsync(&self, inode: Inode, _datasync: bool) -> FsResult<()> {
        self.resolve_data(inode).flush(inode).await
    }

    async fn link(&self, parent: Inode, name: &str, target_inode: Inode) -> FsResult<FileAttr> {
        self.metadata.link(parent, name, target_inode).await
    }

    async fn symlink(
        &self,
        parent: Inode,
        name: &str,
        link_target: &str,
        uid: u32,
        gid: u32,
    ) -> FsResult<FileAttr> {
        self.metadata
            .symlink(parent, name, link_target, uid, gid)
            .await
    }

    async fn readlink(&self, inode: Inode) -> FsResult<String> {
        self.metadata.readlink(inode).await
    }

    async fn release(&self, inode: Inode) -> FsResult<()> {
        self.metadata.release(inode).await?;
        let mut map = self.handle_map.lock().expect("handle_map poisoned");
        map.remove(&inode);
        Ok(())
    }
}
