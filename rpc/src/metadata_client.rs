//! Metadata RPC client — implements `MetadataOps` over gRPC.

use async_trait::async_trait;
use tonic::transport::Channel;
use tonic::Request;

use rucksfs_core::{
    DataLocation, DirEntry, FileAttr, FsError, FsResult, Inode, MetadataOps, OpenResponse,
    ReleaseResponse, RenameResponse, SetAttrRequest, SetAttrResponse, StatFs, UnlinkResponse,
};
use crate::metadata_proto::{
    metadata_service_client::MetadataServiceClient,
    CreateRequest, GetattrRequest, LinkRequest, LookupRequest, MkdirRequest, OpenRequest,
    ReadlinkRequest, ReaddirRequest, ReleaseRequest, RenameRequest, ReportWriteRequest,
    RmdirRequest, SetAttrRequest as ProtoSetAttrRequest, StatfsRequest, SymlinkRequest,
    UnlinkRequest,
};
use crate::tls::ClientTlsConfig as TlsConfig;

/// gRPC client that implements `MetadataOps` for remote MetadataServer.
/// 
/// The client wraps MetadataServiceClient directly without additional Mutex wrapping.
/// MetadataServiceClient is already fully thread-safe and can be cloned and shared
/// across async tasks. The underlying tonic Channel supports HTTP/2 multiplexing,
/// allowing concurrent requests without serialization.
#[derive(Clone)]
pub struct MetadataRpcClient {
    client: MetadataServiceClient<Channel>,
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
            client: MetadataServiceClient::new(channel),
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
            client: MetadataServiceClient::new(channel),
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
        tonic::Code::Aborted => FsError::TransactionConflict,
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
        self.client
            .clone()
            .lookup(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        let req = Request::new(GetattrRequest { inode });
        self.client
            .clone()
            .getattr(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        let req = Request::new(ReaddirRequest { inode });
        let resp = self.client
            .clone()
            .readdir(req)
            .await
            .map_err(map_error)?
            .into_inner();
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

    async fn create(
        &self,
        parent: Inode,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> FsResult<FileAttr> {
        let req = Request::new(CreateRequest {
            parent,
            name: name.to_string(),
            mode,
            uid,
            gid,
        });
        self.client
            .clone()
            .create(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn mkdir(
        &self,
        parent: Inode,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> FsResult<FileAttr> {
        let req = Request::new(MkdirRequest {
            parent,
            name: name.to_string(),
            mode,
            uid,
            gid,
        });
        self.client
            .clone()
            .mkdir(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<UnlinkResponse> {
        let req = Request::new(UnlinkRequest {
            parent,
            name: name.to_string(),
        });
        let resp = self.client
            .clone()
            .unlink(req)
            .await
            .map_err(map_error)?
            .into_inner();
        Ok(UnlinkResponse {
            purged_inodes: resp.purged_inodes,
        })
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        let req = Request::new(RmdirRequest {
            parent,
            name: name.to_string(),
        });
        self.client.clone().rmdir(req).await.map(|_| ()).map_err(map_error)
    }

    async fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<RenameResponse> {
        let req = Request::new(RenameRequest {
            parent,
            name: name.to_string(),
            new_parent,
            new_name: new_name.to_string(),
        });
        let resp = self.client
            .clone()
            .rename(req)
            .await
            .map_err(map_error)?
            .into_inner();
        Ok(RenameResponse {
            purged_inodes: resp.purged_inodes,
        })
    }

    async fn setattr(&self, inode: Inode, req: SetAttrRequest) -> FsResult<SetAttrResponse> {
        let proto_req = Request::new(ProtoSetAttrRequest {
            inode,
            mode: req.mode,
            uid: req.uid,
            gid: req.gid,
            size: req.size,
            atime: req.atime,
            mtime: req.mtime,
        });
        let resp = self.client
            .clone()
            .setattr(proto_req)
            .await
            .map_err(map_error)?
            .into_inner();
        let attr = resp
            .attr
            .map(from_proto_attr)
            .unwrap_or_default();
        Ok(SetAttrResponse {
            attr,
            needs_truncate: resp.needs_truncate,
        })
    }

    async fn statfs(&self, inode: Inode) -> FsResult<StatFs> {
        let req = Request::new(StatfsRequest { inode });
        self.client
            .clone()
            .statfs(req)
            .await
            .map(|r| from_proto_statfs(r.into_inner()))
            .map_err(map_error)
    }

    async fn open(&self, inode: Inode, flags: u32) -> FsResult<OpenResponse> {
        let req = Request::new(OpenRequest { inode, flags });
        let resp = self.client
            .clone()
            .open(req)
            .await
            .map_err(map_error)?
            .into_inner();
        let data_location = resp
            .data_location
            .map(|dl| DataLocation {
                server_id: dl.server_id,
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
        self.client
            .clone()
            .report_write(req)
            .await
            .map(|_| ())
            .map_err(map_error)
    }

    async fn link(
        &self,
        parent: Inode,
        name: &str,
        target_inode: Inode,
    ) -> FsResult<FileAttr> {
        let req = Request::new(LinkRequest {
            parent,
            name: name.to_string(),
            target_inode,
        });
        self.client
            .clone()
            .link(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn symlink(
        &self,
        parent: Inode,
        name: &str,
        link_target: &str,
        uid: u32,
        gid: u32,
    ) -> FsResult<FileAttr> {
        let req = Request::new(SymlinkRequest {
            parent,
            name: name.to_string(),
            link_target: link_target.to_string(),
            uid,
            gid,
        });
        self.client
            .clone()
            .symlink(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn readlink(&self, inode: Inode) -> FsResult<String> {
        let req = Request::new(ReadlinkRequest { inode });
        self.client
            .clone()
            .readlink(req)
            .await
            .map(|r| r.into_inner().target)
            .map_err(map_error)
    }

    async fn release(&self, inode: Inode) -> FsResult<ReleaseResponse> {
        let req = Request::new(ReleaseRequest { inode });
        let resp = self.client
            .clone()
            .release(req)
            .await
            .map_err(map_error)?
            .into_inner();
        Ok(ReleaseResponse {
            purged_inodes: resp.purged_inodes,
        })
    }
}
