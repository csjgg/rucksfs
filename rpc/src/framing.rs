use rucksfs_core::{FsError, FsResult};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Length prefix size for frame protocol.
const LEN_PREFIX: usize = 4;

/// Send a length-prefixed serialized message.
pub async fn send_frame(stream: &mut TcpStream, msg: &impl Serialize) -> FsResult<()> {
    let bytes = bincode::serialize(msg).map_err(|e| FsError::Io(e.to_string()))?;
    let len = bytes.len() as u32;
    stream.write_all(&len.to_be_bytes()).await.map_err(|e| FsError::Io(e.to_string()))?;
    stream.write_all(&bytes).await.map_err(|e| FsError::Io(e.to_string()))?;
    Ok(())
}

/// Receive a length-prefixed serialized message.
pub async fn recv_frame<T: for<'de> serde::Deserialize<'de>>(stream: &mut TcpStream) -> FsResult<T> {
    let mut len_buf = [0u8; LEN_PREFIX];
    stream.read_exact(&mut len_buf).await.map_err(|e| FsError::Io(e.to_string()))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.map_err(|e| FsError::Io(e.to_string()))?;
    bincode::deserialize(&buf).map_err(|e| FsError::Io(e.to_string()))
}
