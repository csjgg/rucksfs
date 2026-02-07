//! Standalone FUSE client binary: connects to gRPC server and optionally mounts.
//!
//! Usage:
//!   rucksfs-client --server <addr> [--token <token>] [--ca-cert <path>] [--mount <path>]
//!
//! Examples:
//!   rucksfs-client --server http://127.0.0.1:50051
//!   rucksfs-client --server http://127.0.0.1:50051 --token my-secret-token
//!   rucksfs-client --server https://192.168.1.100:50051 --token my-secret-token --ca-cert ca.crt

use rucksfs_client::build_client;
use rucksfs_rpc::{ClientTlsConfig, RpcClientOps};
use std::sync::Arc;

fn print_usage() {
    eprintln!("Usage: rucksfs-client --server <addr> [options]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --server <addr>      Server address (required)");
    eprintln!("  --token <token>      API token for authentication");
    eprintln!("  --ca-cert <path>     CA certificate path for TLS verification");
    eprintln!("  --domain <name>      Server domain name for TLS verification");
    eprintln!("  --mount <path>       Mount point (Linux only)");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  rucksfs-client --server http://127.0.0.1:50051");
    eprintln!("  rucksfs-client --server http://127.0.0.1:50051 --token my-secret-token");
    eprintln!("  rucksfs-client --server https://server.example.com:50051 --token my-secret-token --mount /mnt/rucksfs");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut server_addr: Option<String> = None;
    let mut auth_token: Option<String> = None;
    let mut ca_cert: Option<String> = None;
    let mut domain: Option<String> = None;
    let mut mount_point: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--server" => {
                if i + 1 < args.len() {
                    server_addr = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --server requires an argument");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "--token" => {
                if i + 1 < args.len() {
                    auth_token = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --token requires an argument");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "--ca-cert" => {
                if i + 1 < args.len() {
                    ca_cert = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --ca-cert requires an argument");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "--domain" => {
                if i + 1 < args.len() {
                    domain = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --domain requires an argument");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "--mount" => {
                if i + 1 < args.len() {
                    mount_point = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --mount requires an argument");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            _ => {
                eprintln!("Error: unknown argument: {}", args[i]);
                print_usage();
                std::process::exit(1);
            }
        }
    }

    let server_addr = match server_addr {
        Some(a) => a,
        None => {
            eprintln!("Error: --server is required");
            print_usage();
            std::process::exit(1);
        }
    };

    // Determine if using TLS
    let use_tls = server_addr.starts_with("https://");
    
    // Configure TLS
    let mut tls_config = None;
    if use_tls {
        let mut config = ClientTlsConfig::new();
        if let Some(ca_path) = ca_cert {
            config = config.with_ca_cert(ca_path);
        }
        if let Some(d) = domain {
            config = config.with_domain(d);
        }
        tls_config = Some(config);
        println!("Using TLS with server: {}", server_addr);
    } else {
        println!("WARNING: Connecting without TLS encryption!");
    }

    // Warn if authentication is disabled
    if auth_token.is_none() {
        println!("WARNING: No authentication token provided!");
    }

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let addr_for_print = server_addr.clone();
    let rpc_client = rt.block_on(RpcClientOps::connect_secure(server_addr, tls_config, auth_token));
    
    let rpc_client = match rpc_client {
        Ok(c) => {
            println!("Successfully connected to {}", addr_for_print);
            c
        }
        Err(e) => {
            eprintln!("Failed to connect to {}: {}", addr_for_print, e);
            std::process::exit(1);
        }
    };

    let _client = build_client(Arc::new(rpc_client));

    #[cfg(target_os = "linux")]
    if let Some(mount) = mount_point {
        if let Err(e) = rucksfs_client::mount_fuse(&mount, _client.clone()) {
            eprintln!("Mount failed: {}", e);
            std::process::exit(1);
        }
    } else {
        println!("Connected. Pass --mount <path> to mount the filesystem.");
    }

    #[cfg(not(target_os = "linux"))]
    {
        if let Some(mount) = mount_point {
            eprintln!("Mount is only supported on Linux, ignoring mount point: {}", mount);
        }
        println!("Connected. Mount is only supported on Linux.");
    }

    // Keep the runtime alive
    rt.block_on(async {
        let result = tokio::signal::ctrl_c().await;
        match result {
            Ok(()) => println!("Received shutdown signal, exiting..."),
            Err(e) => eprintln!("Failed to listen for Ctrl+C: {}", e),
        }
    });
}
