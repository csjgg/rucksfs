//! Data RPC server — wraps `DataOps` as a gRPC service.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use rucksfs_core::{DataOps, FsError};
use crate::data_proto::*;

/// gRPC server that forwards data requests to a `DataOps` backend.
#[derive(Clone)]
pub struct DataRpcServer {
    backend: Arc<dyn DataOps>,
}

impl DataRpcServer {
    pub fn new(backend: Arc<dyn DataOps>) -> Self {
        Self { backend }
    }
}

/// Convert `FsError` to tonic `Status`.
fn map_error(err: FsError) -> Status {
    match err {
        FsError::NotFound => Status::not_found(err.to_string()),
        FsError::PermissionDenied => Status::permission_denied(err.to_string()),
        FsError::InvalidInput(msg) => Status::invalid_argument(msg),
        FsError::Io(msg) => Status::internal(msg),
        FsError::NotImplemented => Status::unimplemented(err.to_string()),
        _ => Status::internal(err.to_string()),
    }
}

#[tonic::async_trait]
impl data_service_server::DataService for DataRpcServer {
    async fn read_data(
        &self,
        request: Request<ReadDataRequest>,
    ) -> Result<Response<ReadDataResponse>, Status> {
        let req = request.into_inner();
        let data = self
            .backend
            .read_data(req.inode, req.offset, req.size)
            .await
            .map_err(map_error)?;
        Ok(Response::new(ReadDataResponse { data }))
    }

    async fn write_data(
        &self,
        request: Request<WriteDataRequest>,
    ) -> Result<Response<WriteDataResponse>, Status> {
        let req = request.into_inner();
        let bytes_written = self
            .backend
            .write_data(req.inode, req.offset, &req.data)
            .await
            .map_err(map_error)?;
        Ok(Response::new(WriteDataResponse { bytes_written }))
    }

    async fn truncate(
        &self,
        request: Request<TruncateRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .truncate(req.inode, req.size)
            .await
            .map_err(map_error)?;
        Ok(Response::new(EmptyResponse {}))
    }

    async fn flush(
        &self,
        request: Request<FlushRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .flush(req.inode)
            .await
            .map_err(map_error)?;
        Ok(Response::new(EmptyResponse {}))
    }

    async fn delete_data(
        &self,
        request: Request<DeleteDataRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .delete_data(req.inode)
            .await
            .map_err(map_error)?;
        Ok(Response::new(EmptyResponse {}))
    }
}
