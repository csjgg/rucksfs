//! rucksfs-metaserver — standalone MetadataServer gRPC daemon.
//!
//! Usage:
//!   rucksfs-metaserver --listen 0.0.0.0:8001 --data-dir /var/rucksfs

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tonic::transport::Server;

use rucksfs_core::DataLocation;
use rucksfs_rpc::{MetadataRpcServer, MetadataServiceServer};
use rucksfs_server::MetadataServer;
use rucksfs_storage::{
    open_rocks_db, RocksDeltaStore, RocksDirectoryIndex, RocksMetadataStore, RocksStorageBundle,
};

#[derive(Parser, Debug)]
#[command(name = "rucksfs-metaserver", version, about = "RucksFS Metadata Server")]
struct Cli {
    /// gRPC listen address.
    #[arg(long, default_value = "0.0.0.0:8001")]
    listen: String,

    /// Data directory for RocksDB metadata.
    #[arg(long, value_name = "DIR")]
    data_dir: PathBuf,

    /// Default DataServer identifier for newly created files.
    #[arg(long, default_value = "default")]
    default_data_server: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let db_path = cli.data_dir.join("metadata.db");
    std::fs::create_dir_all(&cli.data_dir)?;

    let db = open_rocks_db(&db_path)?;
    let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
    let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
    let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
    let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));

    let server = MetadataServer::new(
        metadata,
        index,
        delta_store,
        DataLocation {
            server_id: cli.default_data_server.clone(),
        },
        storage_bundle,
    );

    let rpc_server = MetadataRpcServer::new(Arc::new(server));
    let addr = cli.listen.parse()?;

    println!("RucksFS MetadataServer listening on {}", addr);
    println!("  data-dir: {}", cli.data_dir.display());
    println!("  default-data-server: {}", cli.default_data_server);

    Server::builder()
        .add_service(MetadataServiceServer::new(rpc_server))
        .serve(addr)
        .await?;

    Ok(())
}
