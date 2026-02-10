use rucksfs_client::build_client;
#[cfg(target_os = "linux")]
use rucksfs_client::mount_fuse;
use rucksfs_server::MetadataServer;
use rucksfs_storage::{MemoryDataStore, MemoryDirectoryIndex, MemoryMetadataStore};
use std::sync::Arc;

fn main() {
    let metadata = Arc::new(MemoryMetadataStore::new());
    let index = Arc::new(MemoryDirectoryIndex::new());
    let data = Arc::new(MemoryDataStore::new());
    let server = MetadataServer::new(metadata, data, index);
    let _client = build_client(Arc::new(server));
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--mount") {
        #[cfg(target_os = "linux")]
        let _ = mount_fuse("/tmp/rucksfs", Arc::new(client));
        #[cfg(not(target_os = "linux"))]
        eprintln!("Mount is only supported on Linux, ignoring --mount flag");
    }
}
