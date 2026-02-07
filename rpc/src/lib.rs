use async_trait::async_trait;
use rucksfs_core::{ClientOps, FsError, FsResult};

#[async_trait]
pub trait RpcClient: ClientOps {}

#[async_trait]
pub trait RpcServer: Send + Sync {
    async fn serve(&self) -> FsResult<()>;
}

#[derive(Debug)]
pub struct RpcPlaceholder;

#[async_trait]
impl RpcServer for RpcPlaceholder {
    async fn serve(&self) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }
}
