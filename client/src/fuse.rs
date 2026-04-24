#[cfg(target_os = "linux")]
use std::ffi::OsStr;
#[cfg(target_os = "linux")]
use std::num::NonZeroU32;
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "linux")]
use bytes::Bytes;
#[cfg(target_os = "linux")]
use fuse3::raw::prelude::*;
#[cfg(target_os = "linux")]
use fuse3::{Errno, MountOptions, Result as Fuse3Result};
#[cfg(target_os = "linux")]
use rucksfs_core::{FsError, FsResult, SetAttrRequest, VfsOps};

#[cfg(target_os = "linux")]
const TTL: Duration = Duration::from_secs(1);

/// Async FUSE filesystem implementation backed by fuse3.
///
/// Because fuse3's Filesystem trait is async, we can directly await
/// VfsOps methods without block_on or spawn hacks. fuse3 uses tokio
/// internally and dispatches requests to multiple concurrent tasks,
/// enabling true parallel FUSE request handling.
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
fn secs_to_systime(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

#[cfg(target_os = "linux")]
fn to_fuse3_attr(attr: rucksfs_core::FileAttr) -> FileAttr {
    let kind = if (attr.mode & libc::S_IFMT) == libc::S_IFDIR {
        FileType::Directory
    } else if (attr.mode & libc::S_IFMT) == libc::S_IFLNK {
        FileType::Symlink
    } else {
        FileType::RegularFile
    };
    FileAttr {
        ino: attr.inode,
        size: attr.size,
        blocks: 0,
        atime: secs_to_systime(attr.atime).into(),
        mtime: secs_to_systime(attr.mtime).into(),
        ctime: secs_to_systime(attr.ctime).into(),
        kind,
        perm: (attr.mode & 0o7777) as u16,
        nlink: attr.nlink,
        uid: attr.uid,
        gid: attr.gid,
        rdev: 0,
        blksize: 512,
    }
}

#[cfg(target_os = "linux")]
pub fn fs_error_to_errno(e: FsError) -> Errno {
    use FsError::*;
    let code = match e {
        NotImplemented => libc::EOPNOTSUPP,
        NotFound => libc::ENOENT,
        AlreadyExists => libc::EEXIST,
        NotADirectory => libc::ENOTDIR,
        IsADirectory => libc::EISDIR,
        DirectoryNotEmpty => libc::ENOTEMPTY,
        NameTooLong => libc::ENAMETOOLONG,
        PermissionDenied => libc::EPERM,
        InvalidInput(_) => libc::EINVAL,
        Io(_) => libc::EIO,
        Other(_) => libc::EIO,
        TransactionConflict => libc::EAGAIN,
    };
    Errno::from(code)
}

#[cfg(target_os = "linux")]
impl<C: VfsOps + 'static> Filesystem for FuseClient<C> {
    async fn init(&self, _req: Request) -> Fuse3Result<ReplyInit> {
        Ok(ReplyInit {
            max_write: NonZeroU32::new(128 * 1024).unwrap(),
        })
    }

    async fn destroy(&self, _req: Request) {}

    async fn lookup(
        &self,
        _req: Request,
        parent: u64,
        name: &OsStr,
    ) -> Fuse3Result<ReplyEntry> {
        let name = name.to_string_lossy().to_string();
        let attr = self.client.lookup(parent, &name).await.map_err(fs_error_to_errno)?;
        Ok(ReplyEntry {
            ttl: TTL,
            attr: to_fuse3_attr(attr),
            generation: 0,
        })
    }

    async fn getattr(
        &self,
        _req: Request,
        inode: u64,
        _fh: Option<u64>,
        _flags: u32,
    ) -> Fuse3Result<ReplyAttr> {
        let attr = self.client.getattr(inode).await.map_err(fs_error_to_errno)?;
        Ok(ReplyAttr {
            ttl: TTL,
            attr: to_fuse3_attr(attr),
        })
    }

    async fn setattr(
        &self,
        _req: Request,
        inode: u64,
        _fh: Option<u64>,
        set_attr: SetAttr,
    ) -> Fuse3Result<ReplyAttr> {
        let req = SetAttrRequest {
            mode: set_attr.mode,
            uid: set_attr.uid,
            gid: set_attr.gid,
            size: set_attr.size,
            atime: set_attr.atime.map(|t| t.sec as u64),
            mtime: set_attr.mtime.map(|t| t.sec as u64),
        };
        let attr = self.client.setattr(inode, req).await.map_err(fs_error_to_errno)?;
        Ok(ReplyAttr {
            ttl: TTL,
            attr: to_fuse3_attr(attr),
        })
    }

    async fn mkdir(
        &self,
        req: Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
    ) -> Fuse3Result<ReplyEntry> {
        let name = name.to_string_lossy().to_string();
        let effective_mode = mode & !umask & 0o7777;
        let attr = self
            .client
            .mkdir(parent, &name, effective_mode, req.uid, req.gid)
            .await
            .map_err(fs_error_to_errno)?;
        Ok(ReplyEntry {
            ttl: TTL,
            attr: to_fuse3_attr(attr),
            generation: 0,
        })
    }

    async fn create(
        &self,
        req: Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        flags: u32,
    ) -> Fuse3Result<ReplyCreated> {
        let name = name.to_string_lossy().to_string();
        // fuse3 doesn't pass umask separately in create; use mode as-is.
        let effective_mode = mode & 0o7777;
        // Merged create+open: one RPC instead of two.  The server performs
        // both operations in a single transaction; we get back the new
        // file's attributes and its open handle together.
        let (attr, fh) = self
            .client
            .create_and_open(parent, &name, effective_mode, req.uid, req.gid, flags)
            .await
            .map_err(fs_error_to_errno)?;
        Ok(ReplyCreated {
            ttl: TTL,
            attr: to_fuse3_attr(attr),
            generation: 0,
            fh,
            flags: 0,
        })
    }

    async fn mknod(
        &self,
        req: Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _rdev: u32,
    ) -> Fuse3Result<ReplyEntry> {
        let file_type = mode & libc::S_IFMT;
        if file_type != libc::S_IFREG && file_type != 0 {
            return Err(libc::EOPNOTSUPP.into());
        }
        let name = name.to_string_lossy().to_string();
        let effective_mode = mode & 0o7777;
        let attr = self
            .client
            .create(parent, &name, effective_mode, req.uid, req.gid)
            .await
            .map_err(fs_error_to_errno)?;
        Ok(ReplyEntry {
            ttl: TTL,
            attr: to_fuse3_attr(attr),
            generation: 0,
        })
    }

    async fn unlink(
        &self,
        _req: Request,
        parent: u64,
        name: &OsStr,
    ) -> Fuse3Result<()> {
        let name = name.to_string_lossy().to_string();
        self.client.unlink(parent, &name).await.map_err(fs_error_to_errno)
    }

    async fn rmdir(
        &self,
        _req: Request,
        parent: u64,
        name: &OsStr,
    ) -> Fuse3Result<()> {
        let name = name.to_string_lossy().to_string();
        self.client.rmdir(parent, &name).await.map_err(fs_error_to_errno)
    }

    async fn rename(
        &self,
        _req: Request,
        parent: u64,
        name: &OsStr,
        new_parent: u64,
        new_name: &OsStr,
    ) -> Fuse3Result<()> {
        let name = name.to_string_lossy().to_string();
        let new_name = new_name.to_string_lossy().to_string();
        self.client
            .rename(parent, &name, new_parent, &new_name)
            .await
            .map_err(fs_error_to_errno)
    }

    async fn open(
        &self,
        _req: Request,
        inode: u64,
        flags: u32,
    ) -> Fuse3Result<ReplyOpen> {
        let fh = self.client.open(inode, flags).await.map_err(fs_error_to_errno)?;
        Ok(ReplyOpen { fh, flags: 0 })
    }

    async fn read(
        &self,
        _req: Request,
        inode: u64,
        _fh: u64,
        offset: u64,
        size: u32,
    ) -> Fuse3Result<ReplyData> {
        let data = self.client.read(inode, offset, size).await.map_err(fs_error_to_errno)?;
        Ok(ReplyData {
            data: Bytes::from(data),
        })
    }

    async fn write(
        &self,
        _req: Request,
        inode: u64,
        _fh: u64,
        offset: u64,
        data: &[u8],
        _write_flags: u32,
        flags: u32,
    ) -> Fuse3Result<ReplyWrite> {
        let written = self
            .client
            .write(inode, offset, data, flags)
            .await
            .map_err(fs_error_to_errno)?;
        Ok(ReplyWrite { written })
    }

    async fn release(
        &self,
        _req: Request,
        inode: u64,
        _fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
    ) -> Fuse3Result<()> {
        // POSIX close() does not require the filesystem to report errors
        // synchronously.  Fire-and-forget the release RPC so that FUSE
        // can immediately process the next operation on this thread.
        // This turns release from a sync 240us+ RPC into a free return,
        // critical for the create/close hot path in mdtest-style workloads.
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            if let Err(e) = client.release(inode).await {
                tracing::warn!("async release for inode {} failed: {}", inode, e);
            }
        });
        Ok(())
    }

    async fn flush(
        &self,
        _req: Request,
        inode: u64,
        _fh: u64,
        _lock_owner: u64,
    ) -> Fuse3Result<()> {
        self.client.flush(inode).await.map_err(fs_error_to_errno)
    }

    async fn fsync(
        &self,
        _req: Request,
        inode: u64,
        _fh: u64,
        datasync: bool,
    ) -> Fuse3Result<()> {
        self.client.fsync(inode, datasync).await.map_err(fs_error_to_errno)
    }

    async fn readdir(
        &self,
        _req: Request,
        _parent: u64,
        _fh: u64,
        _offset: i64,
    ) -> Fuse3Result<ReplyDirectory<Self::DirEntryStream<'_>>> {
        // Not used — we implement readdirplus instead.
        // Returning ENOSYS tells the kernel to use readdirplus.
        Err(libc::ENOSYS.into())
    }

    type DirEntryStream<'a> = futures_util::stream::Iter<std::vec::IntoIter<Fuse3Result<DirectoryEntry>>>;

    async fn opendir(
        &self,
        _req: Request,
        _inode: u64,
        _flags: u32,
    ) -> Fuse3Result<ReplyOpen> {
        // We don't track directory handles; just return a dummy.
        Ok(ReplyOpen { fh: 0, flags: 0 })
    }

    async fn releasedir(
        &self,
        _req: Request,
        _inode: u64,
        _fh: u64,
        _flags: u32,
    ) -> Fuse3Result<()> {
        Ok(())
    }

    type DirEntryPlusStream<'a> = futures_util::stream::Iter<std::vec::IntoIter<Fuse3Result<DirectoryEntryPlus>>>;

    async fn readdirplus(
        &self,
        _req: Request,
        parent: u64,
        _fh: u64,
        offset: u64,
        _lock_owner: u64,
    ) -> Fuse3Result<ReplyDirectoryPlus<Self::DirEntryPlusStream<'_>>> {
        let entries = self.client.readdir(parent).await.map_err(fs_error_to_errno)?;
        let plus_entries: Vec<Fuse3Result<DirectoryEntryPlus>> = entries
            .into_iter()
            .enumerate()
            .skip(offset as usize)
            .map(|(i, entry)| {
                let kind = if (entry.kind & libc::S_IFMT) == libc::S_IFDIR {
                    FileType::Directory
                } else if (entry.kind & libc::S_IFMT) == libc::S_IFLNK {
                    FileType::Symlink
                } else {
                    FileType::RegularFile
                };
                Ok(DirectoryEntryPlus {
                    inode: entry.inode,
                    generation: 0,
                    kind,
                    name: entry.name.into(),
                    offset: (i + 1) as i64,
                    attr: FileAttr {
                        ino: entry.inode,
                        size: 0,
                        blocks: 0,
                        atime: UNIX_EPOCH.into(),
                        mtime: UNIX_EPOCH.into(),
                        ctime: UNIX_EPOCH.into(),
                        kind,
                        perm: 0o755,
                        nlink: 1,
                        uid: 0,
                        gid: 0,
                        rdev: 0,
                        blksize: 512,
                    },
                    entry_ttl: TTL,
                    attr_ttl: TTL,
                })
            })
            .collect();
        Ok(ReplyDirectoryPlus {
            entries: futures_util::stream::iter(plus_entries),
        })
    }

    async fn access(
        &self,
        _req: Request,
        inode: u64,
        _mask: u32,
    ) -> Fuse3Result<()> {
        // With default_permissions, kernel checks permissions.
        // We just verify the inode exists.
        self.client.getattr(inode).await.map_err(fs_error_to_errno)?;
        Ok(())
    }

    async fn statfs(
        &self,
        _req: Request,
        inode: u64,
    ) -> Fuse3Result<ReplyStatFs> {
        let st = self.client.statfs(inode).await.map_err(fs_error_to_errno)?;
        Ok(ReplyStatFs {
            blocks: st.blocks,
            bfree: st.bfree,
            bavail: st.bavail,
            files: st.files,
            ffree: st.ffree,
            bsize: st.bsize,
            namelen: st.namelen,
            frsize: st.bsize,
        })
    }

    async fn fallocate(
        &self,
        _req: Request,
        inode: u64,
        _fh: u64,
        offset: u64,
        length: u64,
        mode: u32,
    ) -> Fuse3Result<()> {
        let keep_size = mode & 0x01 != 0;
        let punch_hole = mode & 0x02 != 0;
        let unsupported = mode & !(0x01 | 0x02);

        if unsupported != 0 {
            return Err(libc::EOPNOTSUPP.into());
        }
        if punch_hole || keep_size {
            return Ok(());
        }

        let new_end = offset + length;
        let attr = self.client.getattr(inode).await.map_err(fs_error_to_errno)?;
        if new_end > attr.size {
            let req = SetAttrRequest {
                size: Some(new_end),
                ..Default::default()
            };
            self.client.setattr(inode, req).await.map_err(fs_error_to_errno)?;
        }
        Ok(())
    }

    async fn link(
        &self,
        _req: Request,
        inode: u64,
        new_parent: u64,
        new_name: &OsStr,
    ) -> Fuse3Result<ReplyEntry> {
        let new_name = new_name.to_string_lossy().to_string();
        let attr = self
            .client
            .link(new_parent, &new_name, inode)
            .await
            .map_err(fs_error_to_errno)?;
        Ok(ReplyEntry {
            ttl: TTL,
            attr: to_fuse3_attr(attr),
            generation: 0,
        })
    }

    async fn symlink(
        &self,
        req: Request,
        parent: u64,
        name: &OsStr,
        link: &OsStr,
    ) -> Fuse3Result<ReplyEntry> {
        let name = name.to_string_lossy().to_string();
        let link = link.to_string_lossy().to_string();
        let attr = self
            .client
            .symlink(parent, &name, &link, req.uid, req.gid)
            .await
            .map_err(fs_error_to_errno)?;
        Ok(ReplyEntry {
            ttl: TTL,
            attr: to_fuse3_attr(attr),
            generation: 0,
        })
    }

    async fn readlink(
        &self,
        _req: Request,
        inode: u64,
    ) -> Fuse3Result<ReplyData> {
        let target = self.client.readlink(inode).await.map_err(fs_error_to_errno)?;
        Ok(ReplyData {
            data: Bytes::from(target.into_bytes()),
        })
    }
}

/// Mount the filesystem using fuse3 with async support.
///
/// This uses fuse3's async mount which dispatches FUSE requests
/// to tokio tasks, enabling true concurrent request handling.
#[cfg(target_os = "linux")]
pub async fn mount_fuse<C: VfsOps + 'static>(
    mountpoint: &str,
    client: Arc<C>,
) -> FsResult<()> {
    let fs = FuseClient::new(client);
    let mut mount_options = MountOptions::default();
    mount_options
        .fs_name("rucksfs")
        .force_readdir_plus(true)
        .allow_other(true)
        .default_permissions(true);

    let mount_handle = fuse3::raw::Session::new(mount_options)
        .mount_with_unprivileged(fs, mountpoint)
        .await
        .map_err(|e| FsError::Io(format!("FUSE mount failed: {}", e)))?;

    // Wait for Ctrl+C, then unmount.
    tokio::signal::ctrl_c()
        .await
        .map_err(|e| FsError::Io(format!("signal handler failed: {}", e)))?;

    mount_handle
        .unmount()
        .await
        .map_err(|e| FsError::Io(format!("unmount failed: {}", e)))?;

    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub async fn mount_fuse<C: VfsOps + 'static>(
    _mountpoint: &str,
    _client: Arc<C>,
) -> FsResult<()> {
    Err(FsError::NotImplemented)
}

#[cfg(target_os = "linux")]
pub fn fs_error_to_libc_errno(e: FsError) -> i32 {
    use FsError::*;
    match e {
        NotImplemented => libc::EOPNOTSUPP,
        NotFound => libc::ENOENT,
        AlreadyExists => libc::EEXIST,
        NotADirectory => libc::ENOTDIR,
        IsADirectory => libc::EISDIR,
        DirectoryNotEmpty => libc::ENOTEMPTY,
        NameTooLong => libc::ENAMETOOLONG,
        PermissionDenied => libc::EPERM,
        InvalidInput(_) => libc::EINVAL,
        Io(_) => libc::EIO,
        Other(_) => libc::EIO,
        TransactionConflict => libc::EAGAIN,
    }
}
