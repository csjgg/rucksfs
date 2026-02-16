#[cfg(target_os = "linux")]
use fuser::{
    FileAttr as FuseAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request,
};
#[cfg(target_os = "linux")]
use libc::{EACCES, EINVAL, EIO, EISDIR, ENOENT, ENOTDIR, ENOTEMPTY, EEXIST, EOPNOTSUPP};
#[cfg(target_os = "linux")]
use rucksfs_core::{FileAttr, FsError, FsResult, SetAttrRequest, VfsOps};
#[cfg(target_os = "linux")]
use std::ffi::OsStr;
#[cfg(target_os = "linux")]
use std::time::{Duration, SystemTime};
#[cfg(target_os = "linux")]
use std::sync::Arc;

/// FUSE client wrapper that implements fuser::Filesystem.
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
    C: VfsOps + 'static,
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

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.getattr(ino).await });
        match result {
            Ok(attr) => reply.attr(&Duration::from_secs(1), &to_fuse_attr(attr)),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<fuser::TimeOrNow>,
        mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        let client = self.client.clone();
        let req = SetAttrRequest {
            mode,
            uid,
            gid,
            size,
            atime: atime.map(|t| match t {
                fuser::TimeOrNow::SpecificTime(st) => st
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                fuser::TimeOrNow::Now => SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            }),
            mtime: mtime.map(|t| match t {
                fuser::TimeOrNow::SpecificTime(st) => st
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                fuser::TimeOrNow::Now => SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            }),
        };
        let result =
            futures::executor::block_on(async move { client.setattr(ino, req).await });
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

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
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

    fn flush(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _lock_owner: u64,
        reply: ReplyEmpty,
    ) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.flush(ino).await });
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn fsync(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        let client = self.client.clone();
        let result =
            futures::executor::block_on(async move { client.fsync(ino, datasync).await });
        match result {
            Ok(()) => reply.ok(),
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
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let name = name.to_string_lossy().to_string();
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            client.create(parent, &name, libc::S_IFREG as u32 | 0o644).await
        });
        match result {
            Ok(attr) => {
                let ino = attr.inode;
                let fuse_attr = to_fuse_attr(attr);
                reply.created(&Duration::from_secs(1), &fuse_attr, 0, ino, 0);
            }
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

    fn rename(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        let name = name.to_string_lossy().to_string();
        let newname = newname.to_string_lossy().to_string();
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            client.rename(parent, &name, newparent, &newname).await
        });
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
        nlink: attr.nlink,
        uid: attr.uid,
        gid: attr.gid,
        rdev: 0,
        flags: 0,
        blksize: 512,
    }
}

#[cfg(target_os = "linux")]
pub fn mount_fuse<C: VfsOps + 'static>(mountpoint: &str, client: Arc<C>) -> FsResult<()> {
    let fs = FuseClient::new(client);
    let options = vec![MountOption::RO, MountOption::FSName("rucksfs".to_string())];
    fuser::mount2(fs, mountpoint, &options).map_err(|e| FsError::Io(e.to_string()))
}

#[cfg(not(target_os = "linux"))]
pub fn mount_fuse<C: VfsOps + 'static>(_mountpoint: &str, _client: Arc<C>) -> FsResult<()> {
    Err(FsError::NotImplemented)
}

#[cfg(target_os = "linux")]
pub fn fs_error_to_errno(e: FsError) -> i32 {
    use FsError::*;
    match e {
        NotImplemented => EOPNOTSUPP,
        NotFound => ENOENT,
        AlreadyExists => EEXIST,
        NotADirectory => ENOTDIR,
        IsADirectory => EISDIR,
        DirectoryNotEmpty => ENOTEMPTY,
        PermissionDenied => EACCES,
        InvalidInput(_) => EINVAL,
        Io(_) => EIO,
        Other(_) => EIO,
    }
}
