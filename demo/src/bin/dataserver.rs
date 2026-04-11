//! rucksfs-dataserver — standalone DataServer gRPC daemon.
//!
//! Usage:
//!   rucksfs-dataserver --listen 0.0.0.0:8002 --data-dir /var/rucksfs

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tonic::transport::Server;

use rucksfs_dataserver::DataServer;
use rucksfs_rpc::{DataRpcServer, DataServiceServer};
use rucksfs_storage::RawDiskDataStore;

#[derive(Parser, Debug)]
#[command(name = "rucksfs-dataserver", version, about = "RucksFS Data Server")]
struct Cli {
    /// gRPC listen address.
    #[arg(long, default_value = "0.0.0.0:8002")]
    listen: String,

    /// Data directory for the raw data file.
    #[arg(long, value_name = "DIR")]
    data_dir: PathBuf,

    /// Maximum bytes per file (default: 64 MiB).
    #[arg(long, value_name = "BYTES", default_value = "67108864")]
    max_file_size: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.data_dir)?;

    let data_path = cli.data_dir.join("data.raw");
    let data_store = RawDiskDataStore::open(&data_path, cli.max_file_size)?;
    let data_server = DataServer::new(data_store);

    let rpc_server = DataRpcServer::new(Arc::new(data_server));
    let addr = cli.listen.parse()?;

    println!("RucksFS DataServer listening on {}", addr);
    println!("  data-dir: {}", cli.data_dir.display());
    println!("  max-file-size: {} bytes", cli.max_file_size);

    Server::builder()
        .add_service(DataServiceServer::new(rpc_server))
        .serve(addr)
        .await?;

    Ok(())
}
