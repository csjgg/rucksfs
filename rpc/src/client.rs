use async_trait::async_trait;
use tonic::transport::Channel;
use tonic::Request;
use tokio::sync::Mutex;
use std::sync::Arc;

use rucksfs_core::{ClientOps, DirEntry, FileAttr, FsError, FsResult, Inode, StatFs};
use crate::rucksfs::{
    file_system_service_client::FileSystemServiceClient,
    FileAttr as ProtoFileAttr, StatFs as ProtoStatFs,
    LookupRequest, GetattrRequest, ReaddirRequest, OpenRequest, ReadRequest,
    WriteRequest, CreateRequest, MkdirRequest, UnlinkRequest, RmdirRequest,
    RenameRequest, SetattrRequest, StatfsRequest, FlushRequest, FsyncRequest,
};
use crate::tls::ClientTlsConfig as TlsConfig;

/// gRPC client that implements ClientOps
#[derive(Clone)]
pub struct RpcClientOps {
    client: Arc<Mutex<FileSystemServiceClient<Channel>>>,
}

impl RpcClientOps {
    /// Connect to the gRPC server
    pub async fn connect(addr: String) -> FsResult<Self> {
        let channel = Channel::from_shared(addr)
            .map_err(|e| FsError::InvalidInput(e.to_string()))?
            .connect()
            .await
            .map_err(|e| FsError::Io(e.to_string()))?;
        
        Ok(Self {
            client: Arc::new(Mutex::new(FileSystemServiceClient::new(channel))),
        })
    }

    /// Connect with TLS and authentication
    pub async fn connect_secure(
        addr: String,
        tls_config: Option<TlsConfig>,
        _auth_token: Option<String>,
    ) -> FsResult<Self> {
        let endpoint = Channel::from_shared(addr)
            .map_err(|e| FsError::InvalidInput(e.to_string()))?;

        // Configure TLS
        let endpoint = if let Some(tls) = tls_config {
            let tls = tls.load()
                .map_err(|e| FsError::Io(format!("Failed to load TLS config: {}", e)))?;
            if let Some(tls) = tls {
                endpoint.tls_config(tls).map_err(|e| FsError::Io(e.to_string()))?
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
            client: Arc::new(Mutex::new(FileSystemServiceClient::new(channel))),
        })
    }

    /// Convert tonic Status to FsError
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

    /// Convert proto FileAttr to core FileAttr
    fn from_proto_file_attr(attr: ProtoFileAttr) -> FileAttr {
        FileAttr {
            inode: attr.inode,
            size: attr.size,
            mode: attr.mode,
            nlink: 0, // proto does not carry nlink yet
            uid: attr.uid,
            gid: attr.gid,
            atime: attr.atime,
            mtime: attr.mtime,
            ctime: attr.ctime,
        }
    }

    /// Convert core FileAttr to proto FileAttr
    fn to_proto_file_attr(attr: FileAttr) -> ProtoFileAttr {
        ProtoFileAttr {
            inode: attr.inode,
            size: attr.size,
            mode: attr.mode,
            uid: attr.uid,
            gid: attr.gid,
            atime: attr.atime,
            mtime: attr.mtime,
            ctime: attr.ctime,
            // nlink not in proto yet
        }
    }

    /// Convert proto StatFs to core StatFs
    fn from_proto_statfs(statfs: ProtoStatFs) -> StatFs {
        StatFs {
            blocks: statfs.blocks,
            bfree: statfs.bfree,
            bavail: statfs.bavail,
            files: statfs.files,
            ffree: statfs.ffree,
            bsize: statfs.bsize,
            namelen: statfs.namelen,
        }
    }

    /// Create a request with authentication header
    #[allow(dead_code)]
    fn with_auth<T>(&self, mut req: Request<T>, token: Option<&str>) -> Request<T> {
        if let Some(token) = token {
            req.metadata_mut().append(
                "authorization",
                format!("Bearer {}", token).parse().unwrap(),
            );
        }
        req
    }
}

#[async_trait]
impl ClientOps for RpcClientOps {
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
        let req = Request::new(LookupRequest {
            parent,
            name: name.to_string(),
        });
        
        let mut client = self.client.lock().await;
        client
            .lookup(req)
            .await
            .map(|r| Self::from_proto_file_attr(r.into_inner()))
            .map_err(Self::map_error)
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        let req = Request::new(GetattrRequest { inode });
        
        let mut client = self.client.lock().await;
        client
            .getattr(req)
            .await
            .map(|r| Self::from_proto_file_attr(r.into_inner()))
            .map_err(Self::map_error)
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        let req = Request::new(ReaddirRequest { inode });
        
        let mut client = self.client.lock().await;
        let response = client
            .readdir(req)
            .await
            .map_err(Self::map_error)?
            .into_inner();

        let entries = response
            .entries
            .into_iter()
            .map(|e| DirEntry {
                name: e.name,
                inode: e.inode,
                kind: e.kind,
            })
            .collect();

        Ok(entries)
    }

    async fn open(&self, inode: Inode, flags: u32) -> FsResult<u64> {
        let req = Request::new(OpenRequest { inode, flags });
        
        let mut client = self.client.lock().await;
        client
            .open(req)
            .await
            .map(|r| r.into_inner().handle)
            .map_err(Self::map_error)
    }

    async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        let req = Request::new(ReadRequest { inode, offset, size });
        
        let mut client = self.client.lock().await;
        client
            .read(req)
            .await
            .map(|r| r.into_inner().data)
            .map_err(Self::map_error)
    }

    async fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32> {
        let req = Request::new(WriteRequest {
            inode,
            offset,
            data: data.to_vec(),
            flags,
        });
        
        let mut client = self.client.lock().await;
        client
            .write(req)
            .await
            .map(|r| r.into_inner().bytes_written)
            .map_err(Self::map_error)
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
            .map(|r| Self::from_proto_file_attr(r.into_inner()))
            .map_err(Self::map_error)
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
            .map(|r| Self::from_proto_file_attr(r.into_inner()))
            .map_err(Self::map_error)
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
        let req = Request::new(UnlinkRequest {
            parent,
            name: name.to_string(),
        });
        
        let mut client = self.client.lock().await;
        client
            .unlink(req)
            .await
            .map(|_| ())
            .map_err(Self::map_error)
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        let req = Request::new(RmdirRequest {
            parent,
            name: name.to_string(),
        });
        
        let mut client = self.client.lock().await;
        client
            .rmdir(req)
            .await
            .map(|_| ())
            .map_err(Self::map_error)
    }

    async fn rename(&self, parent: Inode, name: &str, new_parent: Inode, new_name: &str) -> FsResult<()> {
        let req = Request::new(RenameRequest {
            parent,
            name: name.to_string(),
            new_parent,
            new_name: new_name.to_string(),
        });
        
        let mut client = self.client.lock().await;
        client
            .rename(req)
            .await
            .map(|_| ())
            .map_err(Self::map_error)
    }

    async fn setattr(&self, inode: Inode, attr: FileAttr) -> FsResult<FileAttr> {
        let proto_attr = Self::to_proto_file_attr(attr);
        
        let req = Request::new(SetattrRequest {
            inode,
            attr: Some(proto_attr),
        });
        
        let mut client = self.client.lock().await;
        client
            .setattr(req)
            .await
            .map(|r| Self::from_proto_file_attr(r.into_inner()))
            .map_err(Self::map_error)
    }

    async fn statfs(&self, inode: Inode) -> FsResult<StatFs> {
        let req = Request::new(StatfsRequest { inode });
        
        let mut client = self.client.lock().await;
        client
            .statfs(req)
            .await
            .map(|r| Self::from_proto_statfs(r.into_inner()))
            .map_err(Self::map_error)
    }

    async fn flush(&self, inode: Inode) -> FsResult<()> {
        let req = Request::new(FlushRequest { inode });
        
        let mut client = self.client.lock().await;
        client
            .flush(req)
            .await
            .map(|_| ())
            .map_err(Self::map_error)
    }

    async fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()> {
        let req = Request::new(FsyncRequest { inode, datasync });
        
        let mut client = self.client.lock().await;
        client
            .fsync(req)
            .await
            .map(|_| ())
            .map_err(Self::map_error)
    }
}
