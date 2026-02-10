//! Raw-disk data store — uses a single flat file as a block device.
//!
//! Each inode is allocated a fixed-size region within the file:
//!
//! ```text
//! offset = inode * max_file_size + file_offset
//! ```
//!
//! This is intentionally simple and suitable for demonstration / testing.
//! For production, consider using a proper block allocator.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

use async_trait::async_trait;
use rucksfs_core::{FsError, FsResult, Inode};

use crate::DataStore;

/// A data store backed by a single flat file, treating it as raw disk.
///
/// Thread safety is ensured by a `Mutex<File>`.
pub struct RawDiskDataStore {
    file: Mutex<File>,
    /// Maximum bytes per inode.  Each inode's data lives in
    /// `[inode * max_file_size .. (inode + 1) * max_file_size)`.
    max_file_size: u64,
}

impl RawDiskDataStore {
    /// Open (or create) the backing file at `path`.
    ///
    /// * `max_file_size` — maximum number of bytes a single inode may store.
    ///   A value of 64 MiB (67_108_864) is a reasonable default.
    pub fn open(path: &std::path::Path, max_file_size: u64) -> FsResult<Self> {
        if max_file_size == 0 {
            return Err(FsError::InvalidInput(
                "max_file_size must be > 0".to_string(),
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| FsError::Io(e.to_string()))?;

        Ok(Self {
            file: Mutex::new(file),
            max_file_size,
        })
    }

    /// Compute the absolute byte offset inside the backing file.
    fn absolute_offset(&self, inode: Inode, offset: u64) -> FsResult<u64> {
        if offset >= self.max_file_size {
            return Err(FsError::InvalidInput(format!(
                "offset {} exceeds max_file_size {}",
                offset, self.max_file_size
            )));
        }
        inode
            .checked_mul(self.max_file_size)
            .and_then(|base| base.checked_add(offset))
            .ok_or_else(|| {
                FsError::InvalidInput("offset overflow in absolute_offset".to_string())
            })
    }
}

#[async_trait]
impl DataStore for RawDiskDataStore {
    async fn read_at(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        let abs = self.absolute_offset(inode, offset)?;

        // Clamp size to max_file_size boundary
        let remaining = self.max_file_size.saturating_sub(offset);
        let actual_size = (size as u64).min(remaining) as usize;

        let mut file = self
            .file
            .lock()
            .map_err(|e| FsError::Io(format!("file lock poisoned: {}", e)))?;

        file.seek(SeekFrom::Start(abs))
            .map_err(|e| FsError::Io(e.to_string()))?;

        let mut buf = vec![0u8; actual_size];
        let n = file
            .read(&mut buf)
            .map_err(|e| FsError::Io(e.to_string()))?;

        // If we read fewer bytes than requested, pad with zeros (sparse semantics)
        if n < actual_size {
            // Already zeroed by vec![0u8; ...], just truncate read part is fine
            // Actually buf is already correct — read fills front, rest is zero
        }

        Ok(buf)
    }

    async fn write_at(&self, inode: Inode, offset: u64, data: &[u8]) -> FsResult<u32> {
        let abs = self.absolute_offset(inode, offset)?;

        // Clamp to max_file_size boundary
        let remaining = self.max_file_size.saturating_sub(offset) as usize;
        let actual_len = data.len().min(remaining);

        let mut file = self
            .file
            .lock()
            .map_err(|e| FsError::Io(format!("file lock poisoned: {}", e)))?;

        file.seek(SeekFrom::Start(abs))
            .map_err(|e| FsError::Io(e.to_string()))?;

        file.write_all(&data[..actual_len])
            .map_err(|e| FsError::Io(e.to_string()))?;

        Ok(actual_len as u32)
    }

    async fn truncate(&self, inode: Inode, size: u64) -> FsResult<()> {
        if size > self.max_file_size {
            return Err(FsError::InvalidInput(format!(
                "truncate size {} exceeds max_file_size {}",
                size, self.max_file_size
            )));
        }

        // Zero-fill from `size` to `max_file_size` to simulate truncation.
        let abs_start = self.absolute_offset(inode, size)?;
        let zero_len = (self.max_file_size - size) as usize;

        if zero_len == 0 {
            return Ok(());
        }

        let mut file = self
            .file
            .lock()
            .map_err(|e| FsError::Io(format!("file lock poisoned: {}", e)))?;

        file.seek(SeekFrom::Start(abs_start))
            .map_err(|e| FsError::Io(e.to_string()))?;

        // Write zeros in 4 KiB chunks
        let zeros = [0u8; 4096];
        let mut remaining = zero_len;
        while remaining > 0 {
            let chunk = remaining.min(zeros.len());
            file.write_all(&zeros[..chunk])
                .map_err(|e| FsError::Io(e.to_string()))?;
            remaining -= chunk;
        }

        Ok(())
    }

    async fn flush(&self, _inode: Inode) -> FsResult<()> {
        let file = self
            .file
            .lock()
            .map_err(|e| FsError::Io(format!("file lock poisoned: {}", e)))?;

        file.sync_data()
            .map_err(|e| FsError::Io(e.to_string()))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Helper: create a temporary file and return (store, path).
    fn make_store(max_file_size: u64) -> (RawDiskDataStore, PathBuf) {
        let dir = std::env::temp_dir().join("rucksfs_rawdisk_test");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!(
            "test_{}.dat",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = RawDiskDataStore::open(&path, max_file_size).unwrap();
        (store, path)
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn open_zero_max_file_size() {
        let dir = std::env::temp_dir().join("rucksfs_rawdisk_test");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("zero_max.dat");
        let result = RawDiskDataStore::open(&path, 0);
        assert!(result.is_err());
    }

    #[test]
    fn basic_write_read() {
        let (store, path) = make_store(1024);
        rt().block_on(async {
            let written = store.write_at(1, 0, b"hello raw").await.unwrap();
            assert_eq!(written, 9);

            let data = store.read_at(1, 0, 9).await.unwrap();
            assert_eq!(&data, b"hello raw");
        });
        fs::remove_file(path).ok();
    }

    #[test]
    fn write_at_offset() {
        let (store, path) = make_store(1024);
        rt().block_on(async {
            store.write_at(1, 100, b"offset").await.unwrap();
            let data = store.read_at(1, 100, 6).await.unwrap();
            assert_eq!(&data, b"offset");

            // Reading before offset should return zeros
            let zeros = store.read_at(1, 0, 10).await.unwrap();
            assert_eq!(zeros, vec![0u8; 10]);
        });
        fs::remove_file(path).ok();
    }

    #[test]
    fn cross_inode_isolation() {
        let (store, path) = make_store(1024);
        rt().block_on(async {
            store.write_at(1, 0, b"inode_1").await.unwrap();
            store.write_at(2, 0, b"inode_2").await.unwrap();

            let d1 = store.read_at(1, 0, 7).await.unwrap();
            let d2 = store.read_at(2, 0, 7).await.unwrap();
            assert_eq!(&d1, b"inode_1");
            assert_eq!(&d2, b"inode_2");
        });
        fs::remove_file(path).ok();
    }

    #[test]
    fn offset_boundary_check() {
        let (store, path) = make_store(1024);
        rt().block_on(async {
            // Offset at max_file_size should fail
            let result = store.write_at(1, 1024, b"x").await;
            assert!(result.is_err());

            // Offset just within bounds should succeed
            let written = store.write_at(1, 1023, b"x").await.unwrap();
            assert_eq!(written, 1);
        });
        fs::remove_file(path).ok();
    }

    #[test]
    fn truncate_zeros_tail() {
        let (store, path) = make_store(256);
        rt().block_on(async {
            store.write_at(1, 0, &[0xFFu8; 256]).await.unwrap();
            store.truncate(1, 10).await.unwrap();

            // Data after offset 10 should be zeroed
            let data = store.read_at(1, 10, 100).await.unwrap();
            assert!(data.iter().all(|&b| b == 0));

            // Data before offset 10 should be preserved
            let head = store.read_at(1, 0, 10).await.unwrap();
            assert!(head.iter().all(|&b| b == 0xFF));
        });
        fs::remove_file(path).ok();
    }

    #[test]
    fn truncate_exceeds_max() {
        let (store, path) = make_store(256);
        rt().block_on(async {
            let result = store.truncate(1, 300).await;
            assert!(result.is_err());
        });
        fs::remove_file(path).ok();
    }

    #[test]
    fn flush_succeeds() {
        let (store, path) = make_store(256);
        rt().block_on(async {
            store.write_at(1, 0, b"data").await.unwrap();
            store.flush(1).await.unwrap();
        });
        fs::remove_file(path).ok();
    }

    #[test]
    fn write_clamp_to_boundary() {
        let (store, path) = make_store(10);
        rt().block_on(async {
            // Write 20 bytes at offset 5 — only 5 bytes fit
            let written = store.write_at(1, 5, &[0xAA; 20]).await.unwrap();
            assert_eq!(written, 5);

            let data = store.read_at(1, 5, 5).await.unwrap();
            assert!(data.iter().all(|&b| b == 0xAA));
        });
        fs::remove_file(path).ok();
    }
}
