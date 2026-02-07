use async_trait::async_trait;
use rucksfs_core::{
    ClientOps, DirEntry, FileAttr, FsError, FsResult, Inode, StatFs,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ---- Request / Response (one-to-one with ClientOps) ----

#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    Lookup { parent: Inode, name: String },
    Getattr { inode: Inode },
    Readdir { inode: Inode },
    Open { inode: Inode, flags: u32 },
    Read { inode: Inode, offset: u64, size: u32 },
    Write {
        inode: Inode,
        offset: u64,
        data: Vec<u8>,
        flags: u32,
    },
    Create { parent: Inode, name: String, mode: u32 },
    Mkdir { parent: Inode, name: String, mode: u32 },
    Unlink { parent: Inode, name: String },
    Rmdir { parent: Inode, name: String },
    Rename {
        parent: Inode,
        name: String,
        new_parent: Inode,
        new_name: String,
    },
    Setattr { inode: Inode, attr: FileAttr },
    Statfs { inode: Inode },
    Flush { inode: Inode },
    Fsync { inode: Inode, datasync: bool },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    OkFileAttr(FileAttr),
    OkDirEntries(Vec<DirEntry>),
    OkOpen(u64),
    OkRead(Vec<u8>),
    OkWrite(u32),
    OkStatFs(StatFs),
    OkUnit,
    Err(FsError),
}

// ---- Framing: length-prefixed bincode ----

const LEN_PREFIX: usize = 4;

async fn send_frame(stream: &mut TcpStream, msg: &impl Serialize) -> FsResult<()> {
    let bytes = bincode::serialize(msg).map_err(|e| FsError::Io(e.to_string()))?;
    let len = bytes.len() as u32;
    stream.write_all(&len.to_be_bytes()).await.map_err(|e| FsError::Io(e.to_string()))?;
    stream.write_all(&bytes).await.map_err(|e| FsError::Io(e.to_string()))?;
    Ok(())
}

async fn recv_frame<T: for<'de> Deserialize<'de>>(stream: &mut TcpStream) -> FsResult<T> {
    let mut len_buf = [0u8; LEN_PREFIX];
    stream.read_exact(&mut len_buf).await.map_err(|e| FsError::Io(e.to_string()))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.map_err(|e| FsError::Io(e.to_string()))?;
    bincode::deserialize(&buf).map_err(|e| FsError::Io(e.to_string()))
}

// ---- RpcClientOps: implements ClientOps over TCP ----

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
        send_frame(&mut *stream, &req).await?;
        let resp: Response = recv_frame(&mut *stream).await?;
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

// ---- RpcServer: TCP listener, spawn per connection, dispatch to ClientOps ----

async fn handle_stream(backend: Arc<dyn ClientOps>, mut stream: TcpStream) -> FsResult<()> {
    loop {
        let req: Request = match recv_frame(&mut stream).await {
            Ok(r) => r,
            Err(e) => {
                let _ = send_frame(&mut stream, &Response::Err(FsError::Io(e.to_string()))).await;
                break;
            }
        };
        let resp = match req {
            Request::Lookup { parent, name } => {
                match backend.lookup(parent, &name).await {
                    Ok(a) => Response::OkFileAttr(a),
                    Err(e) => Response::Err(e),
                }
            }
            Request::Getattr { inode } => {
                match backend.getattr(inode).await {
                    Ok(a) => Response::OkFileAttr(a),
                    Err(e) => Response::Err(e),
                }
            }
            Request::Readdir { inode } => {
                match backend.readdir(inode).await {
                    Ok(e) => Response::OkDirEntries(e),
                    Err(e) => Response::Err(e),
                }
            }
            Request::Open { inode, flags } => {
                match backend.open(inode, flags).await {
                    Ok(h) => Response::OkOpen(h),
                    Err(e) => Response::Err(e),
                }
            }
            Request::Read { inode, offset, size } => {
                match backend.read(inode, offset, size).await {
                    Ok(d) => Response::OkRead(d),
                    Err(e) => Response::Err(e),
                }
            }
            Request::Write { inode, offset, data, flags } => {
                match backend.write(inode, offset, &data, flags).await {
                    Ok(n) => Response::OkWrite(n),
                    Err(e) => Response::Err(e),
                }
            }
            Request::Create { parent, name, mode } => {
                match backend.create(parent, &name, mode).await {
                    Ok(a) => Response::OkFileAttr(a),
                    Err(e) => Response::Err(e),
                }
            }
            Request::Mkdir { parent, name, mode } => {
                match backend.mkdir(parent, &name, mode).await {
                    Ok(a) => Response::OkFileAttr(a),
                    Err(e) => Response::Err(e),
                }
            }
            Request::Unlink { parent, name } => {
                match backend.unlink(parent, &name).await {
                    Ok(()) => Response::OkUnit,
                    Err(e) => Response::Err(e),
                }
            }
            Request::Rmdir { parent, name } => {
                match backend.rmdir(parent, &name).await {
                    Ok(()) => Response::OkUnit,
                    Err(e) => Response::Err(e),
                }
            }
            Request::Rename { parent, name, new_parent, new_name } => {
                match backend.rename(parent, &name, new_parent, &new_name).await {
                    Ok(()) => Response::OkUnit,
                    Err(e) => Response::Err(e),
                }
            }
            Request::Setattr { inode, attr } => {
                match backend.setattr(inode, attr).await {
                    Ok(a) => Response::OkFileAttr(a),
                    Err(e) => Response::Err(e),
                }
            }
            Request::Statfs { inode } => {
                match backend.statfs(inode).await {
                    Ok(s) => Response::OkStatFs(s),
                    Err(e) => Response::Err(e),
                }
            }
            Request::Flush { inode } => {
                match backend.flush(inode).await {
                    Ok(()) => Response::OkUnit,
                    Err(e) => Response::Err(e),
                }
            }
            Request::Fsync { inode, datasync } => {
                match backend.fsync(inode, datasync).await {
                    Ok(()) => Response::OkUnit,
                    Err(e) => Response::Err(e),
                }
            }
        };
        if let Err(_e) = send_frame(&mut stream, &resp).await {
            break;
        }
    }
    Ok(())
}

pub async fn serve(addr: &str, backend: Arc<dyn ClientOps>) -> FsResult<()> {
    let listener = TcpListener::bind(addr).await.map_err(|e| FsError::Io(e.to_string()))?;
    loop {
        let (stream, _) = listener.accept().await.map_err(|e| FsError::Io(e.to_string()))?;
        let backend = backend.clone();
        tokio::spawn(async move {
            let _ = handle_stream(backend, stream).await;
        });
    }
}

// ---- Legacy trait aliases (for compatibility) ----

#[async_trait]
pub trait RpcClient: ClientOps {}

#[async_trait]
pub trait RpcServer: Send + Sync {
    async fn serve(&self) -> FsResult<()>;
}

#[derive(Debug)]
pub struct RpcPlaceholder;

#[async_trait]
impl RpcServer for RpcPlaceholder {
    async fn serve(&self) -> FsResult<()> {
        Err(FsError::NotImplemented)
    }
}
