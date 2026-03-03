//! End-to-end integration tests for the RucksFS stack.
//!
//! These tests exercise the full MetadataServer + DataServer + EmbeddedClient
//! pipeline using RocksDB storage for isolated testing and persistence
//! verification.

use std::sync::Arc;

use rucksfs_client::EmbeddedClient;
use rucksfs_core::{DataLocation, DataOps, MetadataOps, VfsOps};
use rucksfs_dataserver::DataServer;
use rucksfs_server::MetadataServer;
use rucksfs_storage::{RawDiskDataStore, RocksDeltaStore, RocksDirectoryIndex, RocksMetadataStore, RocksStorageBundle, open_rocks_db};

/// Root inode constant.
const ROOT: u64 = 1;

/// Helper: build a RocksDB-backed server+client stack.
fn new_client() -> (tempfile::TempDir, EmbeddedClient) {
    let tmp = tempfile::tempdir().unwrap();
    let db = open_rocks_db(tmp.path().join("meta.db")).unwrap();
    let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
    let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
    let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
    let data_store = RawDiskDataStore::open(&tmp.path().join("data.raw"), 64 * 1024 * 1024).unwrap();

    let data_server: Arc<dyn DataOps> = Arc::new(DataServer::new(data_store));
    let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));
    let metadata_server: Arc<dyn MetadataOps> = Arc::new(MetadataServer::new(
        metadata,
        index,
        delta_store,
        Arc::clone(&data_server),
        DataLocation {
            address: "embedded".to_string(),
        },
        storage_bundle,
    ));

    (tmp, EmbeddedClient::new(metadata_server, data_server))
}

// ===========================================================================
// Auto-demo scenario (mirrors run_auto_demo)
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn auto_demo_full_flow() {
    let (_tmp, client) = new_client();

    // 1. mkdir /mydir
    let dir_attr = client.mkdir(ROOT, "mydir", 0o755).await.unwrap();
    assert_eq!(dir_attr.mode & 0o040000, 0o040000);
    let mydir_inode = dir_attr.inode;

    // 2. create /mydir/hello.txt
    let file_attr = client.create(mydir_inode, "hello.txt", 0o644).await.unwrap();
    assert_eq!(file_attr.mode & 0o100000, 0o100000);
    let file_inode = file_attr.inode;

    // 3. write content
    let content = b"Hello, RucksFS!\n";
    let written = client.write(file_inode, 0, content, 0).await.unwrap();
    assert_eq!(written as usize, content.len());

    // 4. read content
    let data = client.read(file_inode, 0, 4096).await.unwrap();
    assert!(data.starts_with(content));

    // 5. readdir
    let entries = client.readdir(mydir_inode).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "hello.txt");
    assert_eq!(entries[0].inode, file_inode);

    // 6. rename
    client
        .rename(mydir_inode, "hello.txt", mydir_inode, "greeting.txt")
        .await
        .unwrap();

    // 7. getattr after rename
    let attr = client.lookup(mydir_inode, "greeting.txt").await.unwrap();
    assert_eq!(attr.inode, file_inode);
    assert_eq!(attr.size, content.len() as u64);

    // 8. unlink
    client.unlink(mydir_inode, "greeting.txt").await.unwrap();

    // 9. rmdir
    client.rmdir(ROOT, "mydir").await.unwrap();

    // 10. statfs
    let st = client.statfs(ROOT).await.unwrap();
    assert!(st.blocks > 0);
    assert!(st.bsize > 0);
}

// ===========================================================================
// Detailed operation tests
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn mkdir_creates_directory() {
    let (_tmp, client) = new_client();
    let attr = client.mkdir(ROOT, "testdir", 0o755).await.unwrap();
    assert_ne!(attr.inode, ROOT);
    assert_eq!(attr.mode & 0o040000, 0o040000);
}

#[tokio::test(flavor = "multi_thread")]
async fn mkdir_duplicate_fails() {
    let (_tmp, client) = new_client();
    client.mkdir(ROOT, "dup", 0o755).await.unwrap();
    let result = client.mkdir(ROOT, "dup", 0o755).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn create_file() {
    let (_tmp, client) = new_client();
    let attr = client.create(ROOT, "myfile.txt", 0o644).await.unwrap();
    assert_eq!(attr.mode & 0o100000, 0o100000);
    assert_eq!(attr.size, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_duplicate_fails() {
    let (_tmp, client) = new_client();
    client.create(ROOT, "dup.txt", 0o644).await.unwrap();
    let result = client.create(ROOT, "dup.txt", 0o644).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn write_and_read() {
    let (_tmp, client) = new_client();
    let attr = client.create(ROOT, "data.bin", 0o644).await.unwrap();
    let inode = attr.inode;

    let data = b"test data 12345";
    let written = client.write(inode, 0, data, 0).await.unwrap();
    assert_eq!(written as usize, data.len());

    let read_back = client.read(inode, 0, 4096).await.unwrap();
    assert!(read_back.starts_with(data));
}

#[tokio::test(flavor = "multi_thread")]
async fn write_at_offset() {
    let (_tmp, client) = new_client();
    let attr = client.create(ROOT, "offset.bin", 0o644).await.unwrap();
    let inode = attr.inode;

    client.write(inode, 0, b"AAAA", 0).await.unwrap();
    client.write(inode, 4, b"BBBB", 0).await.unwrap();

    let data = client.read(inode, 0, 8).await.unwrap();
    assert_eq!(&data[..8], b"AAAABBBB");
}

#[tokio::test(flavor = "multi_thread")]
async fn read_empty_file() {
    let (_tmp, client) = new_client();
    let attr = client.create(ROOT, "empty", 0o644).await.unwrap();
    let data = client.read(attr.inode, 0, 100).await.unwrap();
    assert!(data.iter().all(|&b| b == 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn lookup_existing() {
    let (_tmp, client) = new_client();
    let created = client.create(ROOT, "findme", 0o644).await.unwrap();
    let found = client.lookup(ROOT, "findme").await.unwrap();
    assert_eq!(found.inode, created.inode);
}

#[tokio::test(flavor = "multi_thread")]
async fn lookup_nonexistent() {
    let (_tmp, client) = new_client();
    let result = client.lookup(ROOT, "nope").await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn readdir_root_empty() {
    let (_tmp, client) = new_client();
    let entries = client.readdir(ROOT).await.unwrap();
    assert!(entries.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn readdir_multiple_entries() {
    let (_tmp, client) = new_client();
    client.mkdir(ROOT, "d1", 0o755).await.unwrap();
    client.mkdir(ROOT, "d2", 0o755).await.unwrap();
    client.create(ROOT, "f1", 0o644).await.unwrap();

    let entries = client.readdir(ROOT).await.unwrap();
    assert_eq!(entries.len(), 3);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"d1"));
    assert!(names.contains(&"d2"));
    assert!(names.contains(&"f1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unlink_file() {
    let (_tmp, client) = new_client();
    client.create(ROOT, "todelete", 0o644).await.unwrap();
    client.unlink(ROOT, "todelete").await.unwrap();
    assert!(client.lookup(ROOT, "todelete").await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn unlink_nonexistent_fails() {
    let (_tmp, client) = new_client();
    let result = client.unlink(ROOT, "ghost").await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn rmdir_empty() {
    let (_tmp, client) = new_client();
    client.mkdir(ROOT, "emptydir", 0o755).await.unwrap();
    client.rmdir(ROOT, "emptydir").await.unwrap();
    assert!(client.lookup(ROOT, "emptydir").await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn rmdir_nonempty_fails() {
    let (_tmp, client) = new_client();
    let dir = client.mkdir(ROOT, "full", 0o755).await.unwrap();
    client.create(dir.inode, "child", 0o644).await.unwrap();
    let result = client.rmdir(ROOT, "full").await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_same_parent() {
    let (_tmp, client) = new_client();
    let attr = client.create(ROOT, "old", 0o644).await.unwrap();
    client.rename(ROOT, "old", ROOT, "new").await.unwrap();

    assert!(client.lookup(ROOT, "old").await.is_err());
    let renamed = client.lookup(ROOT, "new").await.unwrap();
    assert_eq!(renamed.inode, attr.inode);
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_cross_parent() {
    let (_tmp, client) = new_client();
    let d1 = client.mkdir(ROOT, "src", 0o755).await.unwrap();
    let d2 = client.mkdir(ROOT, "dst", 0o755).await.unwrap();
    let f = client.create(d1.inode, "moveme", 0o644).await.unwrap();

    client
        .rename(d1.inode, "moveme", d2.inode, "moved")
        .await
        .unwrap();

    assert!(client.lookup(d1.inode, "moveme").await.is_err());
    let found = client.lookup(d2.inode, "moved").await.unwrap();
    assert_eq!(found.inode, f.inode);
}

#[tokio::test(flavor = "multi_thread")]
async fn getattr_root() {
    let (_tmp, client) = new_client();
    let attr = client.getattr(ROOT).await.unwrap();
    assert_eq!(attr.inode, ROOT);
    assert_eq!(attr.mode & 0o040000, 0o040000);
}

#[tokio::test(flavor = "multi_thread")]
async fn setattr_changes_mode() {
    let (_tmp, client) = new_client();
    let attr = client.create(ROOT, "chmod_me", 0o644).await.unwrap();
    let req = rucksfs_core::SetAttrRequest {
        mode: Some(0o755),
        ..Default::default()
    };
    let result = client.setattr(attr.inode, req).await.unwrap();
    assert_eq!(result.mode & 0o7777, 0o755);
}

#[tokio::test(flavor = "multi_thread")]
async fn statfs_returns_valid_data() {
    let (_tmp, client) = new_client();
    let st = client.statfs(ROOT).await.unwrap();
    assert!(st.blocks > 0);
    assert!(st.bsize > 0);
    assert!(st.namelen > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn deep_directory_tree() {
    let (_tmp, client) = new_client();

    let a = client.mkdir(ROOT, "a", 0o755).await.unwrap();
    let b = client.mkdir(a.inode, "b", 0o755).await.unwrap();
    let c = client.mkdir(b.inode, "c", 0o755).await.unwrap();
    let d = client.mkdir(c.inode, "d", 0o755).await.unwrap();

    let f = client.create(d.inode, "deep.txt", 0o644).await.unwrap();
    client.write(f.inode, 0, b"deep content", 0).await.unwrap();

    let data = client.read(f.inode, 0, 100).await.unwrap();
    assert!(data.starts_with(b"deep content"));

    let a2 = client.lookup(ROOT, "a").await.unwrap();
    assert_eq!(a2.inode, a.inode);
    let b2 = client.lookup(a2.inode, "b").await.unwrap();
    assert_eq!(b2.inode, b.inode);
    let c2 = client.lookup(b2.inode, "c").await.unwrap();
    assert_eq!(c2.inode, c.inode);
    let d2 = client.lookup(c2.inode, "d").await.unwrap();
    assert_eq!(d2.inode, d.inode);
    let f2 = client.lookup(d2.inode, "deep.txt").await.unwrap();
    assert_eq!(f2.inode, f.inode);
}

// ===========================================================================
// RocksDB persistence tests
// ===========================================================================

mod rocksdb_tests {
    use super::*;
    use rucksfs_storage::{open_rocks_db, RawDiskDataStore, RocksDeltaStore, RocksDirectoryIndex, RocksMetadataStore, RocksStorageBundle};

    /// Build a persistent server+client stack at the given directory.
    fn persistent_client(dir: &std::path::Path) -> EmbeddedClient {
        let db_path = dir.join("metadata.db");
        let data_path = dir.join("data.raw");

        let db = open_rocks_db(&db_path).expect("open RocksDB");
        let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
        let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
        let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
        let data_store = RawDiskDataStore::open(&data_path, 64 * 1024 * 1024)
            .expect("open RawDisk");

        let data_server: Arc<dyn DataOps> = Arc::new(DataServer::new(data_store));
        let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));
        let metadata_server: Arc<dyn MetadataOps> = Arc::new(MetadataServer::new(
            metadata,
            index,
            delta_store,
            Arc::clone(&data_server),
            DataLocation {
                address: "embedded".to_string(),
            },
            storage_bundle,
        ));

        EmbeddedClient::new(metadata_server, data_server)
    }

    #[tokio::test]
    async fn persist_data_across_restart() {
        let tmp = tempfile::tempdir().unwrap();

        // Session 1: create files and write data
        {
            let client = persistent_client(tmp.path());
            client.mkdir(ROOT, "persist_dir", 0o755).await.unwrap();
            let dir_attr = client.lookup(ROOT, "persist_dir").await.unwrap();
            let file_attr = client
                .create(dir_attr.inode, "hello.txt", 0o644)
                .await
                .unwrap();
            client
                .write(file_attr.inode, 0, b"persistent data", 0)
                .await
                .unwrap();
        }

        // Session 2: verify data survives restart
        {
            let client = persistent_client(tmp.path());
            let dir_attr = client.lookup(ROOT, "persist_dir").await.unwrap();
            let file_attr = client
                .lookup(dir_attr.inode, "hello.txt")
                .await
                .unwrap();
            let data = client.read(file_attr.inode, 0, 4096).await.unwrap();
            assert!(data.starts_with(b"persistent data"));
        }
    }

    #[tokio::test]
    async fn persist_directory_structure() {
        let tmp = tempfile::tempdir().unwrap();

        // Session 1
        {
            let client = persistent_client(tmp.path());
            let a = client.mkdir(ROOT, "alpha", 0o755).await.unwrap();
            client.mkdir(a.inode, "beta", 0o755).await.unwrap();
            client.create(ROOT, "root_file.txt", 0o644).await.unwrap();
        }

        // Session 2
        {
            let client = persistent_client(tmp.path());
            let entries = client.readdir(ROOT).await.unwrap();
            assert_eq!(entries.len(), 2);
            let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
            assert!(names.contains(&"alpha"));
            assert!(names.contains(&"root_file.txt"));

            let alpha = client.lookup(ROOT, "alpha").await.unwrap();
            let sub_entries = client.readdir(alpha.inode).await.unwrap();
            assert_eq!(sub_entries.len(), 1);
            assert_eq!(sub_entries[0].name, "beta");
        }
    }
}
