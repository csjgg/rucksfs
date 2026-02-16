//! Data RPC client — implements `DataOps` over gRPC.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;
use tonic::Request;

use rucksfs_core::{DataOps, FsError, FsResult, Inode};
use crate::data_proto::{
    data_service_client::DataServiceClient, DeleteDataRequest, FlushRequest,
    ReadDataRequest, TruncateRequest, WriteDataRequest,
};
use crate::tls::ClientTlsConfig as TlsConfig;

/// gRPC client that implements `DataOps` for remote DataServer.
#[derive(Clone)]
pub struct DataRpcClient {
    client: Arc<Mutex<DataServiceClient<Channel>>>,
}

impl DataRpcClient {
    /// Connect to a DataServer at the given address.
    pub async fn connect(addr: String) -> FsResult<Self> {
        let channel = Channel::from_shared(addr)
            .map_err(|e| FsError::InvalidInput(e.to_string()))?
            .connect()
            .await
            .map_err(|e| FsError::Io(e.to_string()))?;

        Ok(Self {
            client: Arc::new(Mutex::new(DataServiceClient::new(channel))),
        })
    }

    /// Connect with TLS.
    pub async fn connect_secure(
        addr: String,
        tls_config: Option<TlsConfig>,
    ) -> FsResult<Self> {
        let endpoint = Channel::from_shared(addr)
            .map_err(|e| FsError::InvalidInput(e.to_string()))?;

        let endpoint = if let Some(tls) = tls_config {
            let tls = tls
                .load()
                .map_err(|e| FsError::Io(format!("Failed to load TLS config: {}", e)))?;
            if let Some(tls) = tls {
                endpoint
                    .tls_config(tls)
                    .map_err(|e| FsError::Io(e.to_string()))?
            } else {
                endpoint
            }
        } else {
            endpoint
        };

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| FsError::Io(e.to_string()))?;

        Ok(Self {
            client: Arc::new(Mutex::new(DataServiceClient::new(channel))),
        })
    }
}

/// Convert tonic Status to FsError.
fn map_error(err: tonic::Status) -> FsError {
    match err.code() {
        tonic::Code::NotFound => FsError::NotFound,
        tonic::Code::PermissionDenied => FsError::PermissionDenied,
        tonic::Code::InvalidArgument => FsError::InvalidInput(err.message().to_string()),
        tonic::Code::Unimplemented => FsError::NotImplemented,
        _ => FsError::Io(err.message().to_string()),
    }
}

#[async_trait]
impl DataOps for DataRpcClient {
    async fn read_data(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        let req = Request::new(ReadDataRequest {
            inode,
            offset,
            size,
        });
        let mut client = self.client.lock().await;
        client
            .read_data(req)
            .await
            .map(|r| r.into_inner().data)
            .map_err(map_error)
    }

    async fn write_data(&self, inode: Inode, offset: u64, data: &[u8]) -> FsResult<u32> {
        let req = Request::new(WriteDataRequest {
            inode,
            offset,
            data: data.to_vec(),
        });
        let mut client = self.client.lock().await;
        client
            .write_data(req)
            .await
            .map(|r| r.into_inner().bytes_written)
            .map_err(map_error)
    }

    async fn truncate(&self, inode: Inode, size: u64) -> FsResult<()> {
        let req = Request::new(TruncateRequest { inode, size });
        let mut client = self.client.lock().await;
        client.truncate(req).await.map(|_| ()).map_err(map_error)
    }

    async fn flush(&self, inode: Inode) -> FsResult<()> {
        let req = Request::new(FlushRequest { inode });
        let mut client = self.client.lock().await;
        client.flush(req).await.map(|_| ()).map_err(map_error)
    }

    async fn delete_data(&self, inode: Inode) -> FsResult<()> {
        let req = Request::new(DeleteDataRequest { inode });
        let mut client = self.client.lock().await;
        client
            .delete_data(req)
            .await
            .map(|_| ())
            .map_err(map_error)
    }
}
