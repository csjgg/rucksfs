//! Metadata RPC client — implements `MetadataOps` over gRPC.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;

use rucksfs_core::{
    CreateAndOpenResponse, DataLocation, DirEntry, FileAttr, FsError, FsResult, Inode, MetadataOps,
    OpenResponse, ReleaseResponse, RenameResponse, SetAttrRequest, SetAttrResponse, StatFs,
    UnlinkResponse,
};
use crate::metadata_proto::{
    metadata_service_client::MetadataServiceClient,
    CreateAndOpenRequest, CreateRequest, GetattrRequest, LinkRequest, LookupRequest, MkdirRequest,
    OpenRequest, ReadlinkRequest, ReaddirRequest, ReleaseRequest, RenameRequest,
    ReportWriteRequest, RmdirRequest, SetAttrRequest as ProtoSetAttrRequest, StatfsRequest,
    SymlinkRequest, UnlinkRequest,
};
use crate::tls::ClientTlsConfig as TlsConfig;

/// Environment variable controlling the per-client gRPC connection pool size.
/// Each connection corresponds to a separate HTTP/2 channel; round-robin across
/// them avoids the single-connection frame-serialization bottleneck observed
/// in high-concurrency benchmarks.
///
/// Default is 1 (preserves prior behavior). Set to 4 or 8 for high concurrency.
const POOL_SIZE_ENV: &str = "RUCKSFS_CLIENT_POOL_SIZE";
const DEFAULT_POOL_SIZE: usize = 1;

fn pool_size_from_env() -> usize {
    std::env::var(POOL_SIZE_ENV)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_POOL_SIZE)
}

/// gRPC client that implements `MetadataOps` for remote MetadataServer.
///
/// Internally maintains a pool of N independent HTTP/2 connections (controlled
/// by `RUCKSFS_CLIENT_POOL_SIZE`, default 1). Each RPC call round-robins over
/// the pool. Because tonic/HTTP/2 serializes request framing per TCP connection,
/// a single-connection client plateaus at ~34k ops/s on localhost; using
/// multiple connections lets the server process requests on multiple cores
/// in parallel.
pub struct MetadataRpcClient {
    clients: Arc<Vec<MetadataServiceClient<Channel>>>,
    counter: Arc<AtomicUsize>,
}

impl Clone for MetadataRpcClient {
    fn clone(&self) -> Self {
        Self {
            clients: Arc::clone(&self.clients),
            counter: Arc::clone(&self.counter),
        }
    }
}

impl MetadataRpcClient {
    /// Select the next client in the pool (round-robin).
    fn next_client(&self) -> MetadataServiceClient<Channel> {
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % self.clients.len();
        // `MetadataServiceClient<Channel>` is cheap to clone (it wraps an
        // `Arc<Channel>` internally); cloning lets each RPC take a mutable
        // reference without contending on a Mutex.
        self.clients[idx].clone()
    }

    /// Connect to a MetadataServer at the given address. Pool size is read
    /// from `RUCKSFS_CLIENT_POOL_SIZE` (default 1).
    pub async fn connect(addr: String) -> FsResult<Self> {
        let n = pool_size_from_env();
        let mut clients = Vec::with_capacity(n);
        for _ in 0..n {
            let channel = Channel::from_shared(addr.clone())
                .map_err(|e| FsError::InvalidInput(e.to_string()))?
                .connect()
                .await
                .map_err(|e| FsError::Io(e.to_string()))?;
            clients.push(MetadataServiceClient::new(channel));
        }
        Ok(Self {
            clients: Arc::new(clients),
            counter: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Connect with TLS. Pool size is read from `RUCKSFS_CLIENT_POOL_SIZE`.
    pub async fn connect_secure(
        addr: String,
        tls_config: Option<TlsConfig>,
    ) -> FsResult<Self> {
        let n = pool_size_from_env();
        let mut clients = Vec::with_capacity(n);
        let loaded_tls = if let Some(tls) = tls_config {
            tls.load()
                .map_err(|e| FsError::Io(format!("Failed to load TLS config: {}", e)))?
        } else {
            None
        };
        for _ in 0..n {
            let endpoint: Endpoint = Channel::from_shared(addr.clone())
                .map_err(|e| FsError::InvalidInput(e.to_string()))?;
            let endpoint = if let Some(ref tls) = loaded_tls {
                endpoint
                    .tls_config(tls.clone())
                    .map_err(|e| FsError::Io(e.to_string()))?
            } else {
                endpoint
            };
            let channel = endpoint
                .connect()
                .await
                .map_err(|e| FsError::Io(e.to_string()))?;
            clients.push(MetadataServiceClient::new(channel));
        }
        Ok(Self {
            clients: Arc::new(clients),
            counter: Arc::new(AtomicUsize::new(0)),
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
        tonic::Code::OutOfRange => FsError::NameTooLong,
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
        self.next_client()
            .lookup(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        let req = Request::new(GetattrRequest { inode });
        self.next_client()
            .getattr(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        let req = Request::new(ReaddirRequest { inode });
        let resp = self.next_client()
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
        self.next_client()
            .create(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn create_and_open(
        &self,
        parent: Inode,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
        flags: u32,
    ) -> FsResult<CreateAndOpenResponse> {
        let req = Request::new(CreateAndOpenRequest {
            parent,
            name: name.to_string(),
            mode,
            uid,
            gid,
            flags,
        });
        let resp = self
            .next_client()
            .create_and_open(req)
            .await
            .map_err(map_error)?
            .into_inner();
        let attr = resp.attr.map(from_proto_attr).unwrap_or_default();
        let data_location = resp
            .data_location
            .map(|dl| DataLocation {
                server_id: dl.server_id,
            })
            .unwrap_or_default();
        Ok(CreateAndOpenResponse {
            attr,
            handle: resp.handle,
            data_location,
        })
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
        self.next_client()
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
        let resp = self.next_client()
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
        self.next_client().rmdir(req).await.map(|_| ()).map_err(map_error)
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
        let resp = self.next_client()
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
        let resp = self.next_client()
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
        self.next_client()
            .statfs(req)
            .await
            .map(|r| from_proto_statfs(r.into_inner()))
            .map_err(map_error)
    }

    async fn open(&self, inode: Inode, flags: u32) -> FsResult<OpenResponse> {
        let req = Request::new(OpenRequest { inode, flags });
        let resp = self.next_client()
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
        self.next_client()
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
        self.next_client()
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
        self.next_client()
            .symlink(req)
            .await
            .map(|r| from_proto_attr(r.into_inner()))
            .map_err(map_error)
    }

    async fn readlink(&self, inode: Inode) -> FsResult<String> {
        let req = Request::new(ReadlinkRequest { inode });
        self.next_client()
            .readlink(req)
            .await
            .map(|r| r.into_inner().target)
            .map_err(map_error)
    }

    async fn release(&self, inode: Inode) -> FsResult<ReleaseResponse> {
        let req = Request::new(ReleaseRequest { inode });
        let resp = self.next_client()
            .release(req)
            .await
            .map_err(map_error)?
            .into_inner();
        Ok(ReleaseResponse {
            purged_inodes: resp.purged_inodes,
        })
    }
}
