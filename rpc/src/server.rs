use rucksfs_core::{ClientOps, FsError, FsResult};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

use crate::{framing::recv_frame, framing::send_frame, message::Request, message::Response};

/// Handle a single client connection.
async fn handle_stream(backend: Arc<dyn ClientOps>, mut stream: TcpStream) -> FsResult<()> {
    loop {
        let req: Request = match recv_frame(&mut stream).await {
            Ok(r) => r,
            Err(e) => {
                let _ = send_frame(&mut stream, &Response::Err(rucksfs_core::FsError::Io(e.to_string()))).await;
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

/// Start an RPC server listening on the given address.
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
