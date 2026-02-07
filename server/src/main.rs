//! RPC server binary: runs MetadataServer (with dummy storage) and serves RPC.
//!
//! Usage:
//!   rucksfs-server --bind <addr>
//! Example:
//!   rucksfs-server --bind 127.0.0.1:9000

use async_trait::async_trait;
use rucksfs_core::{DirEntry, FileAttr, FsError, FsResult, Inode};
use rucksfs_server::MetadataServer;
use rucksfs_storage::{DataStore, DirectoryIndex, MetadataStore};
use std::sync::Arc;

struct DummyMetadataStore;

impl MetadataStore for DummyMetadataStore {
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

struct DummyDirectoryIndex;

impl DirectoryIndex for DummyDirectoryIndex {
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

struct DummyDataStore;

#[async_trait]
impl DataStore for DummyDataStore {
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

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut bind_addr: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--bind" && i + 1 < args.len() {
            bind_addr = Some(args[i + 1].clone());
            i += 2;
        } else {
            i += 1;
        }
    }

    let addr = bind_addr.unwrap_or_else(|| {
        eprintln!("Usage: rucksfs-server --bind <addr>");
        eprintln!("Example: rucksfs-server --bind 127.0.0.1:9000");
        std::process::exit(1);
    });

    let metadata = Arc::new(DummyMetadataStore);
    let index = Arc::new(DummyDirectoryIndex);
    let data = Arc::new(DummyDataStore);
    let server = MetadataServer::new(metadata, data, index);
    let backend: Arc<dyn rucksfs_core::ClientOps> = Arc::new(server);

    eprintln!("RPC server listening on {}", addr);
    if let Err(e) = rucksfs_rpc::serve(&addr, backend).await {
        eprintln!("serve error: {}", e);
        std::process::exit(1);
    }
}
