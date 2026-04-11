//! rucksfs-client — remote FUSE client that connects to MetadataServer and
//! DataServer via gRPC.
//!
//! Usage:
//!   rucksfs-client --mount /mnt/rucksfs --meta-addr http://10.0.1.5:8001 --data-addr http://10.0.1.6:8002

use std::sync::Arc;

use clap::Parser;

use rucksfs_client::VfsCore;
use rucksfs_rpc::{DataRpcClient, MetadataRpcClient};

#[derive(Parser, Debug)]
#[command(name = "rucksfs-client", version, about = "RucksFS Remote FUSE Client")]
struct Cli {
    /// Mount point for the FUSE filesystem.
    #[arg(long, value_name = "MOUNTPOINT")]
    mount: String,

    /// MetadataServer gRPC address (e.g., http://10.0.1.5:8001).
    #[arg(long, value_name = "ADDR")]
    meta_addr: String,

    /// DataServer gRPC address (e.g., http://10.0.1.6:8002).
    #[arg(long, value_name = "ADDR")]
    data_addr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    println!("RucksFS Remote Client");
    println!("  mount:     {}", cli.mount);
    println!("  meta-addr: {}", cli.meta_addr);
    println!("  data-addr: {}", cli.data_addr);

    // Connect to remote servers.
    let meta_client = MetadataRpcClient::connect(cli.meta_addr.clone()).await
        .map_err(|e| format!("failed to connect to MetadataServer at {}: {}", cli.meta_addr, e))?;
    let data_client = DataRpcClient::connect(cli.data_addr.clone()).await
        .map_err(|e| format!("failed to connect to DataServer at {}: {}", cli.data_addr, e))?;

    let vfs = Arc::new(VfsCore::new(
        Arc::new(meta_client),
        Arc::new(data_client),
    ));

    println!("  Connected. Mounting FUSE...");

    // Create mountpoint if it doesn't exist.
    if let Err(e) = std::fs::create_dir_all(&cli.mount) {
        eprintln!("Error: failed to create mountpoint '{}': {}", cli.mount, e);
        std::process::exit(1);
    }

    #[cfg(target_os = "linux")]
    {
        println!("  Press Ctrl+C or run `fusermount -u {}` to unmount.", cli.mount);
        let rt = tokio::runtime::Handle::current();
        let mountpoint = cli.mount.clone();
        // Run FUSE event loop on a blocking thread so it doesn't block tokio workers.
        let fuse_result = tokio::task::spawn_blocking(move || {
            rucksfs_client::mount_fuse(&mountpoint, vfs, rt)
        }).await.expect("FUSE thread panicked");
        if let Err(e) = fuse_result {
            eprintln!("FUSE mount error: {}", e);
            std::process::exit(1);
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("FUSE mount is only supported on Linux.");
        std::process::exit(1);
    }

    Ok(())
}
