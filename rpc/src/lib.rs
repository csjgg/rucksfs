pub mod auth;
pub mod client;
pub mod tls;

pub mod rucksfs {
    tonic::include_proto!("rucksfs");
}

pub mod server;

pub use client::RpcClientOps;
pub use rucksfs::file_system_service_server::FileSystemServiceServer;
pub use server::{serve, ServerConfig};
pub use tls::{ClientTlsConfig, TlsConfig};
