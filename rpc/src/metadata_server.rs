//! Metadata RPC server — wraps `MetadataOps` as a gRPC service.

use std::sync::Arc;
use tonic::{Request, Response, Status};

use rucksfs_core::{FsError, MetadataOps};
use crate::metadata_proto::*;

/// gRPC server that forwards requests to a `MetadataOps` backend.
#[derive(Clone)]
pub struct MetadataRpcServer {
    backend: Arc<dyn MetadataOps>,
}

impl MetadataRpcServer {
    pub fn new(backend: Arc<dyn MetadataOps>) -> Self {
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
        FsError::AlreadyExists => Status::already_exists(err.to_string()),
        FsError::NotADirectory => Status::invalid_argument(err.to_string()),
        FsError::IsADirectory => Status::invalid_argument(err.to_string()),
        FsError::DirectoryNotEmpty => Status::failed_precondition(err.to_string()),
        FsError::Other(msg) => Status::internal(msg),
    }
}

fn to_proto_attr(attr: rucksfs_core::FileAttr) -> FileAttr {
    FileAttr {
        inode: attr.inode,
        size: attr.size,
        mode: attr.mode,
        nlink: attr.nlink,
        uid: attr.uid,
        gid: attr.gid,
        atime: attr.atime,
        mtime: attr.mtime,
        ctime: attr.ctime,
    }
}

#[tonic::async_trait]
impl metadata_service_server::MetadataService for MetadataRpcServer {
    async fn lookup(
        &self,
        request: Request<LookupRequest>,
    ) -> Result<Response<FileAttr>, Status> {
        let req = request.into_inner();
        self.backend
            .lookup(req.parent, &req.name)
            .await
            .map(|attr| Response::new(to_proto_attr(attr)))
            .map_err(map_error)
    }

    async fn getattr(
        &self,
        request: Request<GetattrRequest>,
    ) -> Result<Response<FileAttr>, Status> {
        let req = request.into_inner();
        self.backend
            .getattr(req.inode)
            .await
            .map(|attr| Response::new(to_proto_attr(attr)))
            .map_err(map_error)
    }

    async fn readdir(
        &self,
        request: Request<ReaddirRequest>,
    ) -> Result<Response<ReaddirResponse>, Status> {
        let req = request.into_inner();
        let entries = self.backend.readdir(req.inode).await.map_err(map_error)?;
        let proto_entries = entries
            .into_iter()
            .map(|e| DirEntry {
                name: e.name,
                inode: e.inode,
                kind: e.kind,
            })
            .collect();
        Ok(Response::new(ReaddirResponse {
            entries: proto_entries,
        }))
    }

    async fn create(
        &self,
        request: Request<CreateRequest>,
    ) -> Result<Response<FileAttr>, Status> {
        let req = request.into_inner();
        self.backend
            .create(req.parent, &req.name, req.mode)
            .await
            .map(|attr| Response::new(to_proto_attr(attr)))
            .map_err(map_error)
    }

    async fn mkdir(
        &self,
        request: Request<MkdirRequest>,
    ) -> Result<Response<FileAttr>, Status> {
        let req = request.into_inner();
        self.backend
            .mkdir(req.parent, &req.name, req.mode)
            .await
            .map(|attr| Response::new(to_proto_attr(attr)))
            .map_err(map_error)
    }

    async fn unlink(
        &self,
        request: Request<UnlinkRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .unlink(req.parent, &req.name)
            .await
            .map(|_| Response::new(EmptyResponse {}))
            .map_err(map_error)
    }

    async fn rmdir(
        &self,
        request: Request<RmdirRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .rmdir(req.parent, &req.name)
            .await
            .map(|_| Response::new(EmptyResponse {}))
            .map_err(map_error)
    }

    async fn rename(
        &self,
        request: Request<RenameRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .rename(req.parent, &req.name, req.new_parent, &req.new_name)
            .await
            .map(|_| Response::new(EmptyResponse {}))
            .map_err(map_error)
    }

    async fn setattr(
        &self,
        request: Request<SetAttrRequest>,
    ) -> Result<Response<FileAttr>, Status> {
        let req = request.into_inner();
        let core_req = rucksfs_core::SetAttrRequest {
            mode: req.mode,
            uid: req.uid,
            gid: req.gid,
            size: req.size,
            atime: req.atime,
            mtime: req.mtime,
        };
        self.backend
            .setattr(req.inode, core_req)
            .await
            .map(|attr| Response::new(to_proto_attr(attr)))
            .map_err(map_error)
    }

    async fn statfs(
        &self,
        request: Request<StatfsRequest>,
    ) -> Result<Response<StatFs>, Status> {
        let req = request.into_inner();
        self.backend
            .statfs(req.inode)
            .await
            .map(|s| {
                Response::new(StatFs {
                    blocks: s.blocks,
                    bfree: s.bfree,
                    bavail: s.bavail,
                    files: s.files,
                    ffree: s.ffree,
                    bsize: s.bsize,
                    namelen: s.namelen,
                })
            })
            .map_err(map_error)
    }

    async fn open(
        &self,
        request: Request<OpenRequest>,
    ) -> Result<Response<OpenResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .open(req.inode, req.flags)
            .await
            .map(|resp| {
                Response::new(OpenResponse {
                    handle: resp.handle,
                    data_location: Some(DataLocation {
                        address: resp.data_location.address,
                    }),
                })
            })
            .map_err(map_error)
    }

    async fn report_write(
        &self,
        request: Request<ReportWriteRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .report_write(req.inode, req.new_size, req.mtime)
            .await
            .map(|_| Response::new(EmptyResponse {}))
            .map_err(map_error)
    }
}
