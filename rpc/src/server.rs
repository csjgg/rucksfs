use std::sync::Arc;
use tonic::{Request, Response, Status};
use tonic::transport::Server as TonicServer;

use rucksfs_core::{ClientOps, FsError};
use crate::rucksfs::*;

/// gRPC server implementation
#[derive(Clone)]
pub struct FileSystemService {
    backend: Arc<dyn ClientOps>,
}

impl FileSystemService {
    pub fn new(backend: Arc<dyn ClientOps>) -> Self {
        Self { backend }
    }

    /// Convert FsError to tonic Status
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

    /// Convert core FileAttr to protobuf FileAttr
    fn to_proto_file_attr(attr: rucksfs_core::FileAttr) -> FileAttr {
        FileAttr {
            inode: attr.inode,
            size: attr.size,
            mode: attr.mode,
            uid: attr.uid,
            gid: attr.gid,
            atime: attr.atime,
            mtime: attr.mtime,
            ctime: attr.ctime,
            // nlink is not in protobuf schema yet; will be added later
        }
    }

    /// Convert proto FileAttr to core FileAttr
    fn from_proto_file_attr(attr: FileAttr) -> rucksfs_core::FileAttr {
        rucksfs_core::FileAttr {
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

    /// Convert proto StatFs to core StatFs
    #[allow(dead_code)]
    fn from_proto_statfs(statfs: StatFs) -> rucksfs_core::StatFs {
        rucksfs_core::StatFs {
            blocks: statfs.blocks,
            bfree: statfs.bfree,
            bavail: statfs.bavail,
            files: statfs.files,
            ffree: statfs.ffree,
            bsize: statfs.bsize,
            namelen: statfs.namelen,
        }
    }

    /// Convert core StatFs to proto StatFs
    fn to_proto_statfs(statfs: rucksfs_core::StatFs) -> StatFs {
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
}

#[tonic::async_trait]
impl file_system_service_server::FileSystemService for FileSystemService {
    async fn lookup(&self, request: Request<LookupRequest>) -> Result<Response<FileAttr>, Status> {
        let req = request.into_inner();
        self.backend
            .lookup(req.parent, &req.name)
            .await
            .map(|attr| Response::new(Self::to_proto_file_attr(attr)))
            .map_err(Self::map_error)
    }

    async fn getattr(&self, request: Request<GetattrRequest>) -> Result<Response<FileAttr>, Status> {
        let req = request.into_inner();
        self.backend
            .getattr(req.inode)
            .await
            .map(|attr| Response::new(Self::to_proto_file_attr(attr)))
            .map_err(Self::map_error)
    }

    async fn readdir(&self, request: Request<ReaddirRequest>) -> Result<Response<ReaddirResponse>, Status> {
        let req = request.into_inner();
        let entries = self.backend
            .readdir(req.inode)
            .await
            .map_err(Self::map_error)?;
        
        let proto_entries = entries
            .into_iter()
            .map(|e| DirEntry {
                name: e.name,
                inode: e.inode,
                kind: e.kind,
            })
            .collect();

        Ok(Response::new(ReaddirResponse { entries: proto_entries }))
    }

    async fn open(&self, request: Request<OpenRequest>) -> Result<Response<OpenResponse>, Status> {
        let req = request.into_inner();
        let handle = self.backend
            .open(req.inode, req.flags)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(OpenResponse { handle }))
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<Response<ReadResponse>, Status> {
        let req = request.into_inner();
        let data = self.backend
            .read(req.inode, req.offset, req.size)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(ReadResponse { data }))
    }

    async fn write(&self, request: Request<WriteRequest>) -> Result<Response<WriteResponse>, Status> {
        let req = request.into_inner();
        let bytes_written = self.backend
            .write(req.inode, req.offset, &req.data, req.flags)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(WriteResponse { bytes_written }))
    }

    async fn create(&self, request: Request<CreateRequest>) -> Result<Response<FileAttr>, Status> {
        let req = request.into_inner();
        let attr = self.backend
            .create(req.parent, &req.name, req.mode)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(Self::to_proto_file_attr(attr)))
    }

    async fn mkdir(&self, request: Request<MkdirRequest>) -> Result<Response<FileAttr>, Status> {
        let req = request.into_inner();
        let attr = self.backend
            .mkdir(req.parent, &req.name, req.mode)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(Self::to_proto_file_attr(attr)))
    }

    async fn unlink(&self, request: Request<UnlinkRequest>) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .unlink(req.parent, &req.name)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(EmptyResponse {}))
    }

    async fn rmdir(&self, request: Request<RmdirRequest>) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .rmdir(req.parent, &req.name)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(EmptyResponse {}))
    }

    async fn rename(&self, request: Request<RenameRequest>) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .rename(req.parent, &req.name, req.new_parent, &req.new_name)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(EmptyResponse {}))
    }

    async fn setattr(&self, request: Request<SetattrRequest>) -> Result<Response<FileAttr>, Status> {
        let req = request.into_inner();
        let attr = Self::from_proto_file_attr(req.attr.unwrap_or_default());
        let new_attr = self.backend
            .setattr(req.inode, attr)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(Self::to_proto_file_attr(new_attr)))
    }

    async fn statfs(&self, request: Request<StatfsRequest>) -> Result<Response<StatFs>, Status> {
        let req = request.into_inner();
        let statfs = self.backend
            .statfs(req.inode)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(Self::to_proto_statfs(statfs)))
    }

    async fn flush(&self, request: Request<FlushRequest>) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .flush(req.inode)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(EmptyResponse {}))
    }

    async fn fsync(&self, request: Request<FsyncRequest>) -> Result<Response<EmptyResponse>, Status> {
        let req = request.into_inner();
        self.backend
            .fsync(req.inode, req.datasync)
            .await
            .map_err(Self::map_error)?;
        
        Ok(Response::new(EmptyResponse {}))
    }
}

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub auth_token: Option<String>,
    pub tls: Option<crate::tls::TlsConfig>,
    pub max_connections: usize,
    pub max_frame_size: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:50051".to_string(),
            auth_token: None,
            tls: None,
            max_connections: 100,
            max_frame_size: 16 * 1024 * 1024, // 16MB
        }
    }
}

/// Start the gRPC server
pub async fn serve(
    backend: Arc<dyn ClientOps>,
    config: ServerConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Starting gRPC server on {}", config.bind_addr);

    let service = FileSystemService::new(backend);
    let svc = file_system_service_server::FileSystemServiceServer::new(service);
    
    // Create server builder
    let mut builder = TonicServer::builder();

    // Configure TLS if provided
    if let Some(tls_config) = &config.tls {
        let tls = tls_config.create_server_tls_config()?;
        builder = builder.tls_config(tls)?;
        tracing::info!("TLS enabled");
    }

    // Note: Authentication interceptor support requires additional setup
    // For now, we add the service without authentication
    if config.auth_token.is_some() {
        tracing::warn!("Authentication token configured but interceptor support needs additional implementation");
    }
    
    tracing::warn!("Authentication disabled - server is not secure!");
    let addr = config.bind_addr.parse()?;
    builder
        .add_service(svc)
        .serve(addr)
        .await?;

    Ok(())
}

