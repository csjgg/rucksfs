pub mod auth;
pub mod tls;

pub mod metadata_proto {
    tonic::include_proto!("rucksfs.metadata");
}

pub mod data_proto {
    tonic::include_proto!("rucksfs.data");
}

pub mod metadata_server;
pub mod metadata_client;
pub mod data_server;
pub mod data_client;

pub use metadata_server::MetadataRpcServer;
pub use metadata_client::MetadataRpcClient;
pub use data_server::DataRpcServer;
pub use data_client::DataRpcClient;
pub use metadata_proto::metadata_service_server::MetadataServiceServer;
pub use data_proto::data_service_server::DataServiceServer;
pub use tls::{ClientTlsConfig, TlsConfig};
