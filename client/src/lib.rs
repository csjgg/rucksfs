use async_trait::async_trait;
use rucksfs_core::{ClientOps, DirEntry, FileAttr, FsError, FsResult, Inode, StatFs};
use std::sync::Arc;

#[cfg(target_os = "linux")]
use fuser::{
    FileAttr as FuseAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory,
    ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request,
};
#[cfg(target_os = "linux")]
use libc::{EACCES, EINVAL, EIO, ENOENT, EOPNOTSUPP};
#[cfg(target_os = "linux")]
use std::ffi::OsStr;
#[cfg(target_os = "linux")]
use std::time::{Duration, SystemTime};

#[async_trait]
pub trait Client: Send + Sync {
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

pub struct InProcessClient<S>
where
    S: ClientOps,
{
    server: Arc<S>,
}

impl<S> InProcessClient<S>
where
    S: ClientOps,
{
    pub fn new(server: Arc<S>) -> Self {
        Self { server }
    }
}

#[async_trait]
impl<S> Client for InProcessClient<S>
where
    S: ClientOps,
{
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
        self.server.lookup(parent, name).await
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        self.server.getattr(inode).await
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        self.server.readdir(inode).await
    }

    async fn open(&self, inode: Inode, flags: u32) -> FsResult<u64> {
        self.server.open(inode, flags).await
    }

    async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        self.server.read(inode, offset, size).await
    }

    async fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32> {
        self.server.write(inode, offset, data, flags).await
    }

    async fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        self.server.create(parent, name, mode).await
    }

    async fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        self.server.mkdir(parent, name, mode).await
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
        self.server.unlink(parent, name).await
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        self.server.rmdir(parent, name).await
    }

    async fn rename(&self, parent: Inode, name: &str, new_parent: Inode, new_name: &str) -> FsResult<()> {
        self.server.rename(parent, name, new_parent, new_name).await
    }

    async fn setattr(&self, inode: Inode, attr: FileAttr) -> FsResult<FileAttr> {
        self.server.setattr(inode, attr).await
    }

    async fn statfs(&self, inode: Inode) -> FsResult<StatFs> {
        self.server.statfs(inode).await
    }

    async fn flush(&self, inode: Inode) -> FsResult<()> {
        self.server.flush(inode).await
    }

    async fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()> {
        self.server.fsync(inode, datasync).await
    }
}

#[cfg(target_os = "linux")]
pub struct FuseClient<C> {
    client: Arc<C>,
}

#[cfg(target_os = "linux")]
impl<C> FuseClient<C> {
    pub fn new(client: Arc<C>) -> Self {
        Self { client }
    }
}

#[cfg(target_os = "linux")]
impl<C> Filesystem for FuseClient<C>
where
    C: Client + 'static,
{
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name = name.to_string_lossy().to_string();
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.lookup(parent, &name).await });
        match result {
            Ok(attr) => reply.entry(&Duration::from_secs(1), &to_fuse_attr(attr), 0),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.getattr(ino).await });
        match result {
            Ok(attr) => reply.attr(&Duration::from_secs(1), &to_fuse_attr(attr)),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.readdir(ino).await });
        match result {
            Ok(entries) => {
                for (i, entry) in entries.into_iter().enumerate() {
                    let kind = dir_entry_kind_to_file_type(entry.kind);
                    let _ = reply.add(entry.inode, (i + 1) as i64, kind, entry.name);
                }
                reply.ok();
            }
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.open(ino, flags as u32).await });
        match result {
            Ok(handle) => reply.opened(handle, 0),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn read(&mut self, _req: &Request<'_>, ino: u64, _fh: u64, offset: i64, size: u32, reply: ReplyData) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            client.read(ino, offset as u64, size).await
        });
        match result {
            Ok(data) => reply.data(&data),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let client = self.client.clone();
        let data = data.to_vec();
        let result = futures::executor::block_on(async move {
            client.write(ino, offset as u64, &data, flags as u32).await
        });
        match result {
            Ok(written) => reply.written(written),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name = name.to_string_lossy().to_string();
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            client.create(parent, &name, libc::S_IFREG as u32 | 0o644).await
        });
        match result {
            Ok(attr) => reply.entry(&Duration::from_secs(1), &to_fuse_attr(attr), 0),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name = name.to_string_lossy().to_string();
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            client.mkdir(parent, &name, libc::S_IFDIR as u32 | 0o755).await
        });
        match result {
            Ok(attr) => reply.entry(&Duration::from_secs(1), &to_fuse_attr(attr), 0),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let name = name.to_string_lossy().to_string();
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.unlink(parent, &name).await });
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let name = name.to_string_lossy().to_string();
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.rmdir(parent, &name).await });
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.statfs(1).await });
        match result {
            Ok(st) => reply.statfs(
                st.blocks,
                st.bfree,
                st.bavail,
                st.files,
                st.ffree,
                st.bsize,
                st.namelen,
                st.bsize,
            ),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }
}

#[cfg(target_os = "linux")]
fn dir_entry_kind_to_file_type(kind: u32) -> FileType {
    use libc::{S_IFDIR, S_IFREG};
    let mt = kind & libc::S_IFMT;
    if mt == S_IFDIR {
        FileType::Directory
    } else {
        FileType::RegularFile
    }
}

#[cfg(target_os = "linux")]
fn to_fuse_attr(attr: FileAttr) -> FuseAttr {
    let kind = dir_entry_kind_to_file_type(attr.mode & libc::S_IFMT);
    FuseAttr {
        ino: attr.inode,
        size: attr.size,
        blocks: 0,
        atime: SystemTime::UNIX_EPOCH + Duration::from_secs(attr.atime),
        mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(attr.mtime),
        ctime: SystemTime::UNIX_EPOCH + Duration::from_secs(attr.ctime),
        crtime: SystemTime::UNIX_EPOCH,
        kind,
        perm: (attr.mode & 0o7777) as u16,
        nlink: 1,
        uid: attr.uid,
        gid: attr.gid,
        rdev: 0,
        flags: 0,
        blksize: 512,
    }
}

#[cfg(target_os = "linux")]
pub fn mount_fuse<C: Client + 'static>(mountpoint: &str, client: Arc<C>) -> FsResult<()> {
    let fs = FuseClient::new(client);
    let options = vec![MountOption::RO, MountOption::FSName("rucksfs".to_string())];
    fuser::mount2(fs, mountpoint, &options).map_err(|e| FsError::Io(e.to_string()))
}

#[cfg(not(target_os = "linux"))]
pub fn mount_fuse<C: Client + 'static>(_mountpoint: &str, _client: Arc<C>) -> FsResult<()> {
    Err(FsError::NotImplemented)
}

/// Build a Client from any ClientOps (in-process or RPC). Demo can use this with Arc::new(metadata_server).
pub fn build_client<S: ClientOps>(ops: Arc<S>) -> InProcessClient<S> {
    InProcessClient::new(ops)
}

#[cfg(target_os = "linux")]
pub fn fs_error_to_errno(e: FsError) -> i32 {
    use FsError::*;
    match e {
        NotImplemented => EOPNOTSUPP,
        NotFound => ENOENT,
        PermissionDenied => EACCES,
        InvalidInput(_) => EINVAL,
        Io(_) => EIO,
        Other(_) => EIO,
    }
}
