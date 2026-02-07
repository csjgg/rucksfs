use async_trait::async_trait;
use rucksfs_core::{ClientOps, DirEntry, FileAttr, FsError, FsResult, Inode, StatFs};
use tokio::net::TcpStream;

use crate::{framing::recv_frame, framing::send_frame, message::Request, message::Response};

/// RPC client that implements ClientOps over TCP.
pub struct RpcClientOps {
    stream: tokio::sync::Mutex<TcpStream>,
}

impl RpcClientOps {
    pub async fn connect(addr: &str) -> FsResult<Self> {
        let stream = TcpStream::connect(addr).await.map_err(|e| FsError::Io(e.to_string()))?;
        Ok(Self {
            stream: tokio::sync::Mutex::new(stream),
        })
    }

    async fn roundtrip(&self, req: Request) -> FsResult<Response> {
        let mut stream = self.stream.lock().await;
        send_frame(&mut stream, &req).await?;
        let resp: Response = recv_frame(&mut stream).await?;
        if let Response::Err(e) = resp {
            return Err(e);
        }
        Ok(resp)
    }
}

#[async_trait]
impl ClientOps for RpcClientOps {
    async fn lookup(&self, parent: Inode, name: &str) -> FsResult<FileAttr> {
        match self.roundtrip(Request::Lookup { parent, name: name.to_string() }).await? {
            Response::OkFileAttr(a) => Ok(a),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn getattr(&self, inode: Inode) -> FsResult<FileAttr> {
        match self.roundtrip(Request::Getattr { inode }).await? {
            Response::OkFileAttr(a) => Ok(a),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn readdir(&self, inode: Inode) -> FsResult<Vec<DirEntry>> {
        match self.roundtrip(Request::Readdir { inode }).await? {
            Response::OkDirEntries(e) => Ok(e),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn open(&self, inode: Inode, flags: u32) -> FsResult<u64> {
        match self.roundtrip(Request::Open { inode, flags }).await? {
            Response::OkOpen(h) => Ok(h),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn read(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        match self.roundtrip(Request::Read { inode, offset, size }).await? {
            Response::OkRead(d) => Ok(d),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn write(&self, inode: Inode, offset: u64, data: &[u8], flags: u32) -> FsResult<u32> {
        match self.roundtrip(Request::Write { inode, offset, data: data.to_vec(), flags }).await? {
            Response::OkWrite(n) => Ok(n),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        match self.roundtrip(Request::Create { parent, name: name.to_string(), mode }).await? {
            Response::OkFileAttr(a) => Ok(a),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn mkdir(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
        match self.roundtrip(Request::Mkdir { parent, name: name.to_string(), mode }).await? {
            Response::OkFileAttr(a) => Ok(a),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn unlink(&self, parent: Inode, name: &str) -> FsResult<()> {
        match self.roundtrip(Request::Unlink { parent, name: name.to_string() }).await? {
            Response::OkUnit => Ok(()),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn rmdir(&self, parent: Inode, name: &str) -> FsResult<()> {
        match self.roundtrip(Request::Rmdir { parent, name: name.to_string() }).await? {
            Response::OkUnit => Ok(()),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn rename(&self, parent: Inode, name: &str, new_parent: Inode, new_name: &str) -> FsResult<()> {
        match self.roundtrip(Request::Rename {
            parent,
            name: name.to_string(),
            new_parent,
            new_name: new_name.to_string(),
        }).await? {
            Response::OkUnit => Ok(()),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn setattr(&self, inode: Inode, attr: FileAttr) -> FsResult<FileAttr> {
        match self.roundtrip(Request::Setattr { inode, attr }).await? {
            Response::OkFileAttr(a) => Ok(a),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn statfs(&self, inode: Inode) -> FsResult<StatFs> {
        match self.roundtrip(Request::Statfs { inode }).await? {
            Response::OkStatFs(s) => Ok(s),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn flush(&self, inode: Inode) -> FsResult<()> {
        match self.roundtrip(Request::Flush { inode }).await? {
            Response::OkUnit => Ok(()),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }

    async fn fsync(&self, inode: Inode, datasync: bool) -> FsResult<()> {
        match self.roundtrip(Request::Fsync { inode, datasync }).await? {
            Response::OkUnit => Ok(()),
            _ => Err(FsError::Other("invalid response".into())),
        }
    }
}
