use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type FileId = u64;
pub type Inode = u64;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
    #[error("other: {0}")]
    Other(String),
}

pub type FsResult<T> = Result<T, FsError>;

pub trait PosixOps: Send + Sync {
    fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr>;
    fn getattr(&self, inode: Inode) -> FsResult<FileAttr>;
    fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>>;
    fn open(&self, inode: Inode, flags: u32) -> FsResult<u64>;
    fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>>;
    fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32>;
    fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr>;
    fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr>;
    fn unlink(&self, parent: Inode, name: &str) -> FsResult<()>;
    fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()>;
    fn rename(&self, parent: Inode, name: &str, new_parent: Inode, new_name: &str) -> FsResult<()>;
    fn setattr(&self, inode: Inode, attr: FileAttr) -> FsResult<FileAttr>;
    fn statfs(&self, inode: Inode) -> FsResult<StatFs>;
    fn flush(&self, inode: Inode) -> FsResult<()>;
    fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()>;
}

#[async_trait]
pub trait ClientOps: Send + Sync {
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr>;
    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr>;
    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>>;
    async fn open(&self, inode: Inode, flags: u32) -> FsResult<u64>;
    async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>>;
    async fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32>;
    async fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr>;
    async fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr>;
    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()>;
    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()>;
    async fn rename(&self, parent: Inode, name: &str, new_parent: Inode, new_name: &str) -> FsResult<()>;
    async fn setattr(&self, inode: Inode, attr: FileAttr) -> FsResult<FileAttr>;
    async fn statfs(&self, inode: Inode) -> FsResult<StatFs>;
    async fn flush(&self, inode: Inode) -> FsResult<()>;
    async fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()>;
}
