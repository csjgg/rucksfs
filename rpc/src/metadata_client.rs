//! Metadata RPC client — implements `MetadataOps` over gRPC.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;
use tonic::Request;

use rucksfs_core::{
    DataLocation, DirEntry, FileAttr, FsError, FsResult, Inode, MetadataOps, OpenResponse,
    SetAttrRequest, StatFs,
};
use crate::metadata_proto::{
    metadata_service_client::MetadataServiceClient,
    CreateRequest, GetattrRequest, LookupRequest, MkdirRequest, OpenRequest,
    ReaddirRequest, RenameRequest, ReportWriteRequest, RmdirRequest,
    SetAttrRequest as ProtoSetAttrRequest, StatfsRequest, UnlinkRequest,
};
use crate::tls::ClientTlsConfig as TlsConfig;

/// gRPC client that implements `MetadataOps` for remote MetadataServer.
#[derive(Clone)]
pub struct MetadataRpcClient {
    client: Arc<Mutex<MetadataServiceClient<Channel>>>,
}

impl MetadataRpcClient {
    /// Connect to a MetadataServer at the given address.
    pub async fn connect(addr: String) -> FsResult<Self> {
        let channel = Channel::from_shared(addr)
            .map_err(|e| FsError::InvalidInput(e.to_string()))?
            .connect()
            .await
            .map_err(|e| FsError::Io(e.to_string()))?;

        Ok(Self {
            client: Arc::new(Mutex::new(MetadataServiceClient::new(channel))),
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
            client: Arc::new(Mutex::new(MetadataServiceClient::new(channel))),
        })
    }
}

/// Convert tonic Status to FsError.
fn map_error(err: tonic::Status) -> FsError {
    match err.code() {
        tonic::Code::NotFound => FsError::NotFound,
        tonic::Code::PermissionDenied => FsError::PermissionDenied,
        tonic::Code::InvalidArgument => FsError::InvalidInput(err.message().to_string()),
        tonic::Code::Unauthenticated => FsError::PermissionDenied,
        tonic::Code::Unimplemented => FsError::NotImplemented,
        tonic::Code::AlreadyExists => FsError::AlreadyExists,
        tonic::Code::FailedPrecondition => FsError::DirectoryNotEmpty,
        _ => FsError::Io(err.message().to_string()),
    }
}

fn from_proto_attr(attr: crate::metadata_proto::FileAttr) -> FileAttr {
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

fn from_proto_statfs(s: crate::metadata_proto::StatFs) -> StatFs {
    StatFs {
        blocks: s.blocks,
        bfree: s.bfree,
        bavail: s.bavail,
        files: s.files,
        ffree: s.ffree,
        bsize: s.bsize,
        namelen: s.namelen,
    }
}

#[async_trait]
impl MetadataOps for MetadataRpcClient {
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
        let req = Request::new(LookupRequest {
            parent,
            name: name.to_string(),
        });
        let mut client = self.client.lock().await;
        client
            .lookup(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        let req = Request::new(GetattrRequest { inode });
        let mut client = self.client.lock().await;
        client
            .getattr(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        let req = Request::new(ReaddirRequest { inode });
        let mut client = self.client.lock().await;
        let resp = client.readdir(req).await.map_err(map_error)?.into_inner();
        Ok(resp
            .entries
            .into_iter()
            .map(|e| DirEntry {
                name: e.name,
                inode: e.inode,
                kind: e.kind,
            })
            .collect())
    }

    async fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        let req = Request::new(CreateRequest {
            parent,
            name: name.to_string(),
            mode,
        });
        let mut client = self.client.lock().await;
        client
            .create(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        let req = Request::new(MkdirRequest {
            parent,
            name: name.to_string(),
            mode,
        });
        let mut client = self.client.lock().await;
        client
            .mkdir(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
        let req = Request::new(UnlinkRequest {
            parent,
            name: name.to_string(),
        });
        let mut client = self.client.lock().await;
        client.unlink(req).await.map(|_| ()).map_err(map_error)
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        let req = Request::new(RmdirRequest {
            parent,
            name: name.to_string(),
        });
        let mut client = self.client.lock().await;
        client.rmdir(req).await.map(|_| ()).map_err(map_error)
    }

    async fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<()> {
        let req = Request::new(RenameRequest {
            parent,
            name: name.to_string(),
            new_parent,
            new_name: new_name.to_string(),
        });
        let mut client = self.client.lock().await;
        client.rename(req).await.map(|_| ()).map_err(map_error)
    }

    async fn setattr(&self, inode: Inode, req: SetAttrRequest) -> FsResult<FileAttr> {
        let proto_req = Request::new(ProtoSetAttrRequest {
            inode,
            mode: req.mode,
            uid: req.uid,
            gid: req.gid,
            size: req.size,
            atime: req.atime,
            mtime: req.mtime,
        });
        let mut client = self.client.lock().await;
        client
            .setattr(proto_req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn statfs(&self, inode: Inode) -> FsResult<StatFs> {
        let req = Request::new(StatfsRequest { inode });
        let mut client = self.client.lock().await;
        client
            .statfs(req)
            .await
            .map(|r| from_proto_statfs(r.into_inner()))
            .map_err(map_error)
    }

    async fn open(&self, inode: Inode, flags: u32) -> FsResult<OpenResponse> {
        let req = Request::new(OpenRequest { inode, flags });
        let mut client = self.client.lock().await;
        let resp = client.open(req).await.map_err(map_error)?.into_inner();
        let data_location = resp
            .data_location
            .map(|dl| DataLocation {
                address: dl.address,
            })
            .unwrap_or_default();
        Ok(OpenResponse {
            handle: resp.handle,
            data_location,
        })
    }

    async fn report_write(
        &self,
        inode: Inode,
        new_size: u64,
        mtime: u64,
    ) -> FsResult<()> {
        let req = Request::new(ReportWriteRequest {
            inode,
            new_size,
            mtime,
        });
        let mut client = self.client.lock().await;
        client
            .report_write(req)
            .await
            .map(|_| ())
            .map_err(map_error)
    }
}
