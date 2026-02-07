//! RPC server binary: runs MetadataServer (with dummy storage) and serves RPC.
//!
//! Usage:
//!   rucksfs-server --bind <addr>
//! Example:
//!   rucksfs-server --bind 127.0.0.1:9000

use rucksfs_server::MetadataServer;
use rucksfs_storage::{DummyDataStore, DummyDirectoryIndex, DummyMetadataStore};
use std::sync::Arc;

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
