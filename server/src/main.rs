//! RPC server binary: runs MetadataServer (with dummy storage) and serves gRPC.
//!
//! Usage:
//!   rucksfs-server --bind <addr> [options]
//!
//! Options:
//!   --bind <addr>        Bind address (required)
//!   --token <token>      API token for authentication (optional, but recommended)
//!   --tls-cert <path>    TLS certificate file (optional)
//!   --tls-key <path>     TLS private key file (optional)
//!
//! Examples:
//!   rucksfs-server --bind 127.0.0.1:50051
//!   rucksfs-server --bind 127.0.0.1:50051 --token my-secret-token
//!   rucksfs-server --bind 0.0.0.0:50051 --token my-secret-token --tls-cert server.crt --tls-key server.key

use rucksfs_server::MetadataServer;
use rucksfs_storage::{DummyDataStore, DummyDirectoryIndex, DummyMetadataStore};
use rucksfs_rpc::{ServerConfig, TlsConfig};
use std::sync::Arc;

fn print_usage() {
    eprintln!("Usage: rucksfs-server --bind <addr> [options]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --bind <addr>        Bind address (required)");
    eprintln!("  --token <token>      API token for authentication (optional, but recommended)");
    eprintln!("  --tls-cert <path>    TLS certificate file (optional)");
    eprintln!("  --tls-key <path>     TLS private key file (optional)");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  rucksfs-server --bind 127.0.0.1:50051");
    eprintln!("  rucksfs-server --bind 0.0.0.0:50051 --token my-secret-token");
    eprintln!("  rucksfs-server --bind 0.0.0.0:50051 --token my-secret-token --tls-cert server.crt --tls-key server.key");
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let mut bind_addr: Option<String> = None;
    let mut auth_token: Option<String> = None;
    let mut tls_cert: Option<String> = None;
    let mut tls_key: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--bind" => {
                if i + 1 < args.len() {
                    bind_addr = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --bind requires an argument");
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
            "--tls-cert" => {
                if i + 1 < args.len() {
                    tls_cert = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --tls-cert requires an argument");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "--tls-key" => {
                if i + 1 < args.len() {
                    tls_key = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --tls-key requires an argument");
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

    let bind_addr = match bind_addr {
        Some(a) => a,
        None => {
            eprintln!("Error: --bind is required");
            print_usage();
            std::process::exit(1);
        }
    };

    // Validate TLS configuration
    let tls = match (tls_cert, tls_key) {
        (Some(cert), Some(key)) => {
            tracing::info!("TLS enabled with certificate: {}", cert);
            Some(TlsConfig::new(cert, key))
        }
        (Some(_), None) => {
            eprintln!("Error: --tls-key is required when --tls-cert is provided");
            std::process::exit(1);
        }
        (None, Some(_)) => {
            eprintln!("Error: --tls-cert is required when --tls-key is provided");
            std::process::exit(1);
        }
        (None, None) => {
            tracing::warn!("TLS disabled - connection will not be encrypted!");
            None
        }
    };

    // Warn if authentication is disabled
    if auth_token.is_none() {
        tracing::warn!("Authentication disabled - server will accept all connections!");
        tracing::warn!("This is not secure for production use!");
    } else {
        tracing::info!("Authentication enabled with Bearer token");
    }

    // Create backend
    let metadata = Arc::new(DummyMetadataStore);
    let index = Arc::new(DummyDirectoryIndex);
    let data = Arc::new(DummyDataStore);
    let server = MetadataServer::new(metadata, data, index);
    let backend: Arc<dyn rucksfs_core::ClientOps> = Arc::new(server);

    // Configure server
    let config = ServerConfig {
        bind_addr,
        auth_token,
        tls,
        ..Default::default()
    };

    tracing::info!("Starting gRPC server...");
    if let Err(e) = rucksfs_rpc::serve(backend, config).await {
        tracing::error!("Server error: {}", e);
        std::process::exit(1);
    }
}
