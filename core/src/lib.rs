use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type FileId = u64;
pub type Inode = u64;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAttr {
    pub inode: Inode,
    pub size: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StatFs {
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub bsize: u32,
    pub namelen: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub inode: Inode,
    pub kind: u32,
}

/// Request for `setattr`. Each field is `Option<T>` to avoid the ambiguity of
/// using 0 to mean "no change" (e.g. `mode = 0` could be a valid value).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SetAttrRequest {
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub size: Option<u64>,
    pub atime: Option<u64>,
    pub mtime: Option<u64>,
}

/// DataServer endpoint information returned by MetadataServer on `open`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DataLocation {
    /// Network address of the DataServer, e.g. "127.0.0.1:9001".
    pub address: String,
}

/// Response from `MetadataOps::open`, containing a file handle and the
/// DataServer location for subsequent read/write operations.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OpenResponse {
    pub handle: u64,
    pub data_location: DataLocation,
}

#[derive(Error, Debug, Serialize, Deserialize)]
pub enum FsError {
    #[error("not implemented")]
    NotImplemented,
    #[error("not found")]
    NotFound,
    #[error("already exists")]
    AlreadyExists,
    #[error("not a directory")]
    NotADirectory,
    #[error("is a directory")]
    IsADirectory,
    #[error("directory not empty")]
    DirectoryNotEmpty,
    #[error("io error: {0}")]
    Io(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("permission denied")]
    PermissionDenied,
    #[error("transaction conflict")]
    TransactionConflict,
    #[error("other: {0}")]
    Other(String),
}

pub type FsResult<T> = Result<T, FsError>;

/// Pure metadata operations. Implemented by MetadataServer.
///
/// Does NOT include data I/O methods (read/write/flush/fsync).
/// When a client needs to read/write file data, it calls `open` first to get
/// a `DataLocation`, then talks to the DataServer directly.
#[async_trait]
pub trait MetadataOps: Send + Sync {
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr>;
    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr>;
    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>>;
    async fn create(&self, parent: Inode, name: &str, mode: u32, uid: u32, gid: u32) -> FsResult<FileAttr>;
    async fn mkdir(&self, parent: Inode, name: &str, mode: u32, uid: u32, gid: u32) -> FsResult<FileAttr>;
    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()>;
    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()>;
    async fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<()>;
    async fn setattr(&self, inode: Inode, req: SetAttrRequest) -> FsResult<FileAttr>;
    async fn statfs(&self, inode: Inode) -> FsResult<StatFs>;
    /// Open a file and return a handle + DataServer location.
    async fn open(&self, inode: Inode, flags: u32) -> FsResult<OpenResponse>;
    /// Called by the client after a successful write to the DataServer,
    /// so that MetadataServer can update size and mtime.
    async fn report_write(
        &self,
        inode: Inode,
        new_size: u64,
        mtime: u64,
    ) -> FsResult<()>;
    /// Create a hard link: add a directory entry `name` under `parent` that
    /// points to the existing `target_inode`. Returns the target's FileAttr.
    async fn link(&self, parent: Inode, name: &str, target_inode: Inode) -> FsResult<FileAttr>;
    /// Create a symbolic link: add a new S_IFLNK inode under `parent` with
    /// directory entry `name`, storing `link_target` as the symlink target.
    async fn symlink(
        &self,
        parent: Inode,
        name: &str,
        link_target: &str,
        uid: u32,
        gid: u32,
    ) -> FsResult<FileAttr>;
    /// Read the target path of a symbolic link.
    async fn readlink(&self, inode: Inode) -> FsResult<String>;
    /// Notify that a file handle has been closed. Decrements open handle
    /// count and triggers deferred deletion if nlink=0 and no handles remain.
    async fn release(&self, inode: Inode) -> FsResult<()>;
}

/// Pure data I/O operations. Implemented by DataServer.
#[async_trait]
pub trait DataOps: Send + Sync {
    async fn read_data(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>>;
    async fn write_data(&self, inode: Inode, offset: u64, data: &[u8]) -> FsResult<u32>;
    async fn truncate(&self, inode: Inode, size: u64) -> FsResult<()>;
    async fn flush(&self, inode: Inode) -> FsResult<()>;
    async fn delete_data(&self, inode: Inode) -> FsResult<()>;
}

/// Full POSIX VFS interface. Implemented by the fat client (EmbeddedClient /
/// RucksClient) which routes metadata ops to MetadataServer and data ops to
/// DataServer.
#[async_trait]
pub trait VfsOps: Send + Sync {
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr>;
    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr>;
    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>>;
    async fn create(&self, parent: Inode, name: &str, mode: u32, uid: u32, gid: u32) -> FsResult<FileAttr>;
    async fn mkdir(&self, parent: Inode, name: &str, mode: u32, uid: u32, gid: u32) -> FsResult<FileAttr>;
    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()>;
    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()>;
    async fn rename(
        &self,
        parent: Inode,
        name: &str,
        new_parent: Inode,
        new_name: &str,
    ) -> FsResult<()>;
    async fn setattr(&self, inode: Inode, req: SetAttrRequest) -> FsResult<FileAttr>;
    async fn statfs(&self, inode: Inode) -> FsResult<StatFs>;
    async fn open(&self, inode: Inode, flags: u32) -> FsResult<u64>;
    async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>>;
    async fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32>;
    async fn flush(&self, inode: Inode) -> FsResult<()>;
    async fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()>;
    async fn link(&self, parent: Inode, name: &str, target_inode: Inode) -> FsResult<FileAttr>;
    async fn symlink(
        &self,
        parent: Inode,
        name: &str,
        link_target: &str,
        uid: u32,
        gid: u32,
    ) -> FsResult<FileAttr>;
    async fn readlink(&self, inode: Inode) -> FsResult<String>;
    async fn release(&self, inode: Inode) -> FsResult<()>;
}
