//! Standalone FUSE client binary: connects to server via RPC and optionally mounts.
//!
//! Usage:
//!   rucksfs-client --server <addr> [--mount <path>]
//! Example:
//!   rucksfs-client --server 127.0.0.1:9000 --mount /tmp/rucksfs

use rucksfs_client::build_client;
use rucksfs_rpc::RpcClientOps;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut server_addr: Option<String> = None;
    let mut mount_point: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--server" => {
                i += 1;
                if i < args.len() {
                    server_addr = Some(args[i].clone());
                }
                i += 1;
            }
            "--mount" => {
                i += 1;
                if i < args.len() {
                    mount_point = Some(args[i].clone());
                }
                i += 1;
            }
            _ => i += 1,
        }
    }

    let addr = match server_addr {
        Some(a) => a,
        None => {
            eprintln!("Usage: rucksfs-client --server <addr> [--mount <path>]");
            eprintln!("Example: rucksfs-client --server 127.0.0.1:9000 --mount /tmp/rucksfs");
            std::process::exit(1);
        }
    };

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let rpc_client = rt.block_on(RpcClientOps::connect(&addr));
    let rpc_client = match rpc_client {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    let _client = build_client(Arc::new(rpc_client));

    #[cfg(target_os = "linux")]
    if let Some(mount) = mount_point {
        if let Err(e) = rucksfs_client::mount_fuse(&mount, Arc::new(client)) {
            eprintln!("Mount failed: {}", e);
            std::process::exit(1);
        }
    } else {
        println!("Connected to {}. Pass --mount <path> to mount.", addr);
    }

    #[cfg(not(target_os = "linux"))]
    if let Some(mount) = mount_point {
        eprintln!("Mount is only supported on Linux, ignoring mount point: {}", mount);
    }
    println!("Connected to {}. Pass --mount <path> to mount.", addr);
}
