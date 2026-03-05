#[cfg(target_os = "linux")]
use fuser::{
    FileAttr as FuseAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request,
};
#[cfg(target_os = "linux")]
use libc::{EAGAIN, EINVAL, EIO, EISDIR, ENOENT, ENOTDIR, ENOTEMPTY, EEXIST, EOPNOTSUPP, EPERM};
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
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.readdir(ino).await });
        match result {
            Ok(entries) => {
                // Skip entries before offset (FUSE pagination).
                for (i, entry) in entries.into_iter().enumerate().skip(offset as usize) {
                    let kind = dir_entry_kind_to_file_type(entry.kind);
                    // reply.add returns true when the buffer is full.
                    if reply.add(entry.inode, (i + 1) as i64, kind, entry.name) {
                        break;
                    }
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
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let name = name.to_string_lossy().to_string();
        let uid = req.uid();
        let gid = req.gid();
        // Apply umask: strip bits that umask disallows, keep file-type bits.
        let effective_mode = mode & !umask & 0o7777;
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            client.create(parent, &name, effective_mode, uid, gid).await
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

    fn mknod(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        // Only support regular files. Other types (block/char devices,
        // sockets, FIFOs) are not implemented.
        let file_type = mode & libc::S_IFMT;
        if file_type != libc::S_IFREG && file_type != 0 {
            reply.error(EOPNOTSUPP);
            return;
        }

        let name = name.to_string_lossy().to_string();
        let uid = req.uid();
        let gid = req.gid();
        let effective_mode = mode & !umask & 0o7777;
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            client.create(parent, &name, effective_mode, uid, gid).await
        });
        match result {
            Ok(attr) => reply.entry(&Duration::from_secs(1), &to_fuse_attr(attr), 0),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn fallocate(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        length: i64,
        mode: i32,
        reply: ReplyEmpty,
    ) {
        let client = self.client.clone();

        // FALLOC_FL_KEEP_SIZE = 0x01, FALLOC_FL_PUNCH_HOLE = 0x02
        let keep_size = mode & 0x01 != 0;
        let punch_hole = mode & 0x02 != 0;
        let unsupported_flags = mode & !(0x01 | 0x02);

        if unsupported_flags != 0 {
            reply.error(EOPNOTSUPP);
            return;
        }

        if punch_hole {
            // Punch hole: no-op for our sparse-like RawDisk backend.
            reply.ok();
            return;
        }

        if keep_size {
            // Preallocate without changing size: no-op for our backend.
            reply.ok();
            return;
        }

        // Mode 0: preallocate space. Extend file size if needed.
        let new_end = (offset + length) as u64;
        let result = futures::executor::block_on(async move {
            let attr = client.getattr(ino).await?;
            if new_end > attr.size {
                let req = rucksfs_core::SetAttrRequest {
                    size: Some(new_end),
                    ..Default::default()
                };
                client.setattr(ino, req).await?;
            }
            Ok::<(), rucksfs_core::FsError>(())
        });
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn access(&mut self, _req: &Request<'_>, ino: u64, _mask: i32, reply: ReplyEmpty) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            // With default_permissions, the kernel already checks permissions.
            // We just verify the inode exists.
            client.getattr(ino).await
        });
        match result {
            Ok(_) => reply.ok(),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn mkdir(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        let name = name.to_string_lossy().to_string();
        let uid = req.uid();
        let gid = req.gid();
        // Apply umask: strip bits that umask disallows, keep file-type bits.
        let effective_mode = mode & !umask & 0o7777;
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            client.mkdir(parent, &name, effective_mode, uid, gid).await
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

    fn link(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        newparent: u64,
        newname: &OsStr,
        reply: ReplyEntry,
    ) {
        let newname = newname.to_string_lossy().to_string();
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            client.link(newparent, &newname, ino).await
        });
        match result {
            Ok(attr) => reply.entry(&Duration::from_secs(1), &to_fuse_attr(attr), 0),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn symlink(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        link_name: &OsStr,
        target: &std::path::Path,
        reply: ReplyEntry,
    ) {
        let link_name = link_name.to_string_lossy().to_string();
        let target = target.to_string_lossy().to_string();
        let uid = req.uid();
        let gid = req.gid();
        let client = self.client.clone();
        let result = futures::executor::block_on(async move {
            client.symlink(parent, &link_name, &target, uid, gid).await
        });
        match result {
            Ok(attr) => reply.entry(&Duration::from_secs(1), &to_fuse_attr(attr), 0),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: fuser::ReplyData) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.readlink(ino).await });
        match result {
            Ok(target) => reply.data(target.as_bytes()),
            Err(e) => reply.error(fs_error_to_errno(e)),
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let client = self.client.clone();
        let result = futures::executor::block_on(async move { client.release(ino).await });
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
    let mt = kind & libc::S_IFMT;
    if mt == libc::S_IFDIR {
        FileType::Directory
    } else if mt == libc::S_IFLNK {
        FileType::Symlink
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
    let options = vec![
        MountOption::FSName("rucksfs".to_string()),
        MountOption::AutoUnmount,
        MountOption::DefaultPermissions,
        MountOption::AllowOther,
    ];
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
        PermissionDenied => EPERM,
        InvalidInput(_) => EINVAL,
        Io(_) => EIO,
        Other(_) => EIO,
        TransactionConflict => EAGAIN,
    }
}
