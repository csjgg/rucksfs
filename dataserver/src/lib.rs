//! DataServer — an independent data storage service.
//!
//! The `DataServer` wraps a [`DataStore`] backend and exposes
//! [`DataOps`] for file data I/O. It does NOT handle metadata;
//! that is the responsibility of MetadataServer.

use async_trait::async_trait;
use rucksfs_core::{DataOps, FsResult, Inode};
use rucksfs_storage::DataStore;

/// A data server backed by a concrete [`DataStore`] implementation.
///
/// The `DataServer` translates [`DataOps`] requests into calls on the
/// underlying storage backend.
pub struct DataServer<D: DataStore> {
    store: D,
}

impl<D: DataStore> DataServer<D> {
    /// Create a new `DataServer` wrapping the given store.
    pub fn new(store: D) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<D: DataStore> DataOps for DataServer<D> {
    async fn read_data(&self, inode: Inode, offset: u64, size: u32) -> FsResult<Vec<u8>> {
        self.store.read_at(inode, offset, size).await
    }

    async fn write_data(&self, inode: Inode, offset: u64, data: &[u8]) -> FsResult<u32> {
        self.store.write_at(inode, offset, data).await
    }

    async fn truncate(&self, inode: Inode, size: u64) -> FsResult<()> {
        self.store.truncate(inode, size).await
    }

    async fn flush(&self, inode: Inode) -> FsResult<()> {
        self.store.flush(inode).await
    }

    async fn delete_data(&self, inode: Inode) -> FsResult<()> {
        self.store.delete(inode).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rucksfs_storage::MemoryDataStore;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn write_then_read() {
        rt().block_on(async {
            let ds = DataServer::new(MemoryDataStore::new());
            let written = ds.write_data(1, 0, b"hello dataserver").await.unwrap();
            assert_eq!(written, 16);
            let data = ds.read_data(1, 0, 16).await.unwrap();
            assert_eq!(data, b"hello dataserver");
        });
    }

    #[test]
    fn read_unwritten_returns_zeros() {
        rt().block_on(async {
            let ds = DataServer::new(MemoryDataStore::new());
            let data = ds.read_data(1, 0, 10).await.unwrap();
            assert_eq!(data, vec![0u8; 10]);
        });
    }

    #[test]
    fn write_at_offset() {
        rt().block_on(async {
            let ds = DataServer::new(MemoryDataStore::new());
            ds.write_data(1, 5, b"world").await.unwrap();
            let data = ds.read_data(1, 0, 10).await.unwrap();
            assert_eq!(&data[..5], &[0, 0, 0, 0, 0]);
            assert_eq!(&data[5..10], b"world");
        });
    }

    #[test]
    fn truncate_shrink() {
        rt().block_on(async {
            let ds = DataServer::new(MemoryDataStore::new());
            ds.write_data(1, 0, b"hello world").await.unwrap();
            ds.truncate(1, 5).await.unwrap();
            let data = ds.read_data(1, 0, 11).await.unwrap();
            assert_eq!(&data[..5], b"hello");
            assert_eq!(&data[5..], &[0; 6]);
        });
    }

    #[test]
    fn truncate_expand() {
        rt().block_on(async {
            let ds = DataServer::new(MemoryDataStore::new());
            ds.write_data(1, 0, b"hi").await.unwrap();
            ds.truncate(1, 10).await.unwrap();
            let data = ds.read_data(1, 0, 10).await.unwrap();
            assert_eq!(&data[..2], b"hi");
            assert_eq!(&data[2..], &[0; 8]);
        });
    }

    #[test]
    fn delete_data_removes_inode() {
        rt().block_on(async {
            let ds = DataServer::new(MemoryDataStore::new());
            ds.write_data(1, 0, b"some data").await.unwrap();
            ds.delete_data(1).await.unwrap();
            // After delete, reading should return zeros
            let data = ds.read_data(1, 0, 9).await.unwrap();
            assert_eq!(data, vec![0u8; 9]);
        });
    }

    #[test]
    fn flush_is_noop_for_memory() {
        rt().block_on(async {
            let ds = DataServer::new(MemoryDataStore::new());
            ds.flush(1).await.unwrap();
        });
    }

    #[test]
    fn cross_inode_isolation() {
        rt().block_on(async {
            let ds = DataServer::new(MemoryDataStore::new());
            ds.write_data(1, 0, b"inode1").await.unwrap();
            ds.write_data(2, 0, b"inode2").await.unwrap();
            assert_eq!(ds.read_data(1, 0, 6).await.unwrap(), b"inode1");
            assert_eq!(ds.read_data(2, 0, 6).await.unwrap(), b"inode2");

            // Delete inode 1 should not affect inode 2
            ds.delete_data(1).await.unwrap();
            assert_eq!(ds.read_data(2, 0, 6).await.unwrap(), b"inode2");
        });
    }
}
