pub mod client;
pub mod framing;
pub mod message;
pub mod server;

pub use client::RpcClientOps;
pub use message::{Request, Response};
pub use server::serve;

// ---- Legacy trait aliases (for compatibility) ----

use async_trait::async_trait;
use rucksfs_core::FsResult;

#[async_trait]
pub trait RpcClient: rucksfs_core::ClientOps {}

#[async_trait]
pub trait RpcServer: Send + Sync {
    async fn serve(&self) -> FsResult<()>;
}

#[derive(Debug)]
pub struct RpcPlaceholder;

#[async_trait]
impl RpcServer for RpcPlaceholder {
    async fn serve(&self) -> FsResult<()> {
        Err(rucksfs_core::FsError::NotImplemented)
    }
}
