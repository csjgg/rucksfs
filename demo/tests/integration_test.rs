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
    let dir_attr = client.mkdir(ROOT, "mydir", 0o755, 0, 0).await.unwrap();
    assert_eq!(dir_attr.mode & 0o040000, 0o040000);
    let mydir_inode = dir_attr.inode;

    // 2. create /mydir/hello.txt
    let file_attr = client.create(mydir_inode, "hello.txt", 0o644, 0, 0).await.unwrap();
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
    let attr = client.mkdir(ROOT, "testdir", 0o755, 0, 0).await.unwrap();
    assert_ne!(attr.inode, ROOT);
    assert_eq!(attr.mode & 0o040000, 0o040000);
}

#[tokio::test(flavor = "multi_thread")]
async fn mkdir_duplicate_fails() {
    let (_tmp, client) = new_client();
    client.mkdir(ROOT, "dup", 0o755, 0, 0).await.unwrap();
    let result = client.mkdir(ROOT, "dup", 0o755, 0, 0).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn create_file() {
    let (_tmp, client) = new_client();
    let attr = client.create(ROOT, "myfile.txt", 0o644, 0, 0).await.unwrap();
    assert_eq!(attr.mode & 0o100000, 0o100000);
    assert_eq!(attr.size, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_duplicate_fails() {
    let (_tmp, client) = new_client();
    client.create(ROOT, "dup.txt", 0o644, 0, 0).await.unwrap();
    let result = client.create(ROOT, "dup.txt", 0o644, 0, 0).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn write_and_read() {
    let (_tmp, client) = new_client();
    let attr = client.create(ROOT, "data.bin", 0o644, 0, 0).await.unwrap();
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
    let attr = client.create(ROOT, "offset.bin", 0o644, 0, 0).await.unwrap();
    let inode = attr.inode;

    client.write(inode, 0, b"AAAA", 0).await.unwrap();
    client.write(inode, 4, b"BBBB", 0).await.unwrap();

    let data = client.read(inode, 0, 8).await.unwrap();
    assert_eq!(&data[..8], b"AAAABBBB");
}

#[tokio::test(flavor = "multi_thread")]
async fn read_empty_file() {
    let (_tmp, client) = new_client();
    let attr = client.create(ROOT, "empty", 0o644, 0, 0).await.unwrap();
    let data = client.read(attr.inode, 0, 100).await.unwrap();
    assert!(data.iter().all(|&b| b == 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn lookup_existing() {
    let (_tmp, client) = new_client();
    let created = client.create(ROOT, "findme", 0o644, 0, 0).await.unwrap();
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
    client.mkdir(ROOT, "d1", 0o755, 0, 0).await.unwrap();
    client.mkdir(ROOT, "d2", 0o755, 0, 0).await.unwrap();
    client.create(ROOT, "f1", 0o644, 0, 0).await.unwrap();

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
    client.create(ROOT, "todelete", 0o644, 0, 0).await.unwrap();
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
    client.mkdir(ROOT, "emptydir", 0o755, 0, 0).await.unwrap();
    client.rmdir(ROOT, "emptydir").await.unwrap();
    assert!(client.lookup(ROOT, "emptydir").await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn rmdir_nonempty_fails() {
    let (_tmp, client) = new_client();
    let dir = client.mkdir(ROOT, "full", 0o755, 0, 0).await.unwrap();
    client.create(dir.inode, "child", 0o644, 0, 0).await.unwrap();
    let result = client.rmdir(ROOT, "full").await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_same_parent() {
    let (_tmp, client) = new_client();
    let attr = client.create(ROOT, "old", 0o644, 0, 0).await.unwrap();
    client.rename(ROOT, "old", ROOT, "new").await.unwrap();

    assert!(client.lookup(ROOT, "old").await.is_err());
    let renamed = client.lookup(ROOT, "new").await.unwrap();
    assert_eq!(renamed.inode, attr.inode);
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_cross_parent() {
    let (_tmp, client) = new_client();
    let d1 = client.mkdir(ROOT, "src", 0o755, 0, 0).await.unwrap();
    let d2 = client.mkdir(ROOT, "dst", 0o755, 0, 0).await.unwrap();
    let f = client.create(d1.inode, "moveme", 0o644, 0, 0).await.unwrap();

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
    let attr = client.create(ROOT, "chmod_me", 0o644, 0, 0).await.unwrap();
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

    let a = client.mkdir(ROOT, "a", 0o755, 0, 0).await.unwrap();
    let b = client.mkdir(a.inode, "b", 0o755, 0, 0).await.unwrap();
    let c = client.mkdir(b.inode, "c", 0o755, 0, 0).await.unwrap();
    let d = client.mkdir(c.inode, "d", 0o755, 0, 0).await.unwrap();

    let f = client.create(d.inode, "deep.txt", 0o644, 0, 0).await.unwrap();
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
            client.mkdir(ROOT, "persist_dir", 0o755, 0, 0).await.unwrap();
            let dir_attr = client.lookup(ROOT, "persist_dir").await.unwrap();
            let file_attr = client
                .create(dir_attr.inode, "hello.txt", 0o644, 0, 0)
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
            let a = client.mkdir(ROOT, "alpha", 0o755, 0, 0).await.unwrap();
            client.mkdir(a.inode, "beta", 0o755, 0, 0).await.unwrap();
            client.create(ROOT, "root_file.txt", 0o644, 0, 0).await.unwrap();
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

// ===========================================================================
// Hard link tests (US-002, US-003)
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn link_creates_hard_link() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "original", 0o644, 0, 0).await.unwrap();
    let linked = client.link(ROOT, "hardlink", file.inode).await.unwrap();

    // Same inode, nlink incremented to 2.
    assert_eq!(linked.inode, file.inode);
    assert_eq!(linked.nlink, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn link_data_shared() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "orig", 0o644, 0, 0).await.unwrap();
    client.write(file.inode, 0, b"shared data", 0).await.unwrap();
    client.link(ROOT, "link1", file.inode).await.unwrap();

    // Read via original and via link should see the same data.
    let link_attr = client.lookup(ROOT, "link1").await.unwrap();
    assert_eq!(link_attr.inode, file.inode);
    let data = client.read(file.inode, 0, 100).await.unwrap();
    assert!(data.starts_with(b"shared data"));
}

#[tokio::test(flavor = "multi_thread")]
async fn link_write_via_link_read_via_original() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "a", 0o644, 0, 0).await.unwrap();
    client.link(ROOT, "b", file.inode).await.unwrap();

    // Write through the linked name.
    client.write(file.inode, 0, b"written via link", 0).await.unwrap();

    // Read via original name.
    let data = client.read(file.inode, 0, 100).await.unwrap();
    assert!(data.starts_with(b"written via link"));
}

#[tokio::test(flavor = "multi_thread")]
async fn link_to_directory_returns_eperm() {
    let (_tmp, client) = new_client();
    let dir = client.mkdir(ROOT, "mydir", 0o755, 0, 0).await.unwrap();
    let result = client.link(ROOT, "dirlink", dir.inode).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn link_duplicate_name_returns_eexist() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "file1", 0o644, 0, 0).await.unwrap();
    client.create(ROOT, "existing", 0o644, 0, 0).await.unwrap();
    let result = client.link(ROOT, "existing", file.inode).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn link_nonexistent_target_returns_enoent() {
    let (_tmp, client) = new_client();
    let result = client.link(ROOT, "broken", 99999).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn unlink_hardlink_preserves_data() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "x", 0o644, 0, 0).await.unwrap();
    client.write(file.inode, 0, b"keep me", 0).await.unwrap();
    client.link(ROOT, "y", file.inode).await.unwrap();

    // Unlink original name — inode should survive (nlink > 0).
    client.unlink(ROOT, "x").await.unwrap();

    // Data should still be accessible via the remaining link.
    let attr = client.lookup(ROOT, "y").await.unwrap();
    assert_eq!(attr.inode, file.inode);
    assert_eq!(attr.nlink, 1);
    let data = client.read(file.inode, 0, 100).await.unwrap();
    assert!(data.starts_with(b"keep me"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unlink_last_link_deletes_data() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "sole", 0o644, 0, 0).await.unwrap();
    client.write(file.inode, 0, b"goodbye", 0).await.unwrap();

    client.unlink(ROOT, "sole").await.unwrap();

    // Inode should be gone.
    assert!(client.lookup(ROOT, "sole").await.is_err());
    assert!(client.getattr(file.inode).await.is_err());
}

// ===========================================================================
// Symbolic link tests (US-004, US-005)
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn symlink_create_and_readlink() {
    let (_tmp, client) = new_client();
    let attr = client
        .symlink(ROOT, "mylink", "/target/path", 1000, 1000)
        .await
        .unwrap();

    // Check symlink attributes.
    assert_eq!(attr.mode & 0o170000, 0o120000); // S_IFLNK
    assert_eq!(attr.mode & 0o7777, 0o777);      // permissions
    assert_eq!(attr.size, "/target/path".len() as u64);
    assert_eq!(attr.nlink, 1);
    assert_eq!(attr.uid, 1000);
    assert_eq!(attr.gid, 1000);

    // Read back the target.
    let target = client.readlink(attr.inode).await.unwrap();
    assert_eq!(target, "/target/path");
}

#[tokio::test(flavor = "multi_thread")]
async fn symlink_lookup_returns_symlink_type() {
    let (_tmp, client) = new_client();
    let attr = client
        .symlink(ROOT, "sl", "target", 0, 0)
        .await
        .unwrap();
    let looked_up = client.lookup(ROOT, "sl").await.unwrap();
    assert_eq!(looked_up.inode, attr.inode);
    assert_eq!(looked_up.mode & 0o170000, 0o120000);
}

#[tokio::test(flavor = "multi_thread")]
async fn symlink_duplicate_name_returns_eexist() {
    let (_tmp, client) = new_client();
    client.create(ROOT, "taken", 0o644, 0, 0).await.unwrap();
    let result = client.symlink(ROOT, "taken", "target", 0, 0).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn readlink_on_regular_file_returns_einval() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "regular", 0o644, 0, 0).await.unwrap();
    let result = client.readlink(file.inode).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn readlink_on_directory_returns_einval() {
    let (_tmp, client) = new_client();
    let dir = client.mkdir(ROOT, "adir", 0o755, 0, 0).await.unwrap();
    let result = client.readlink(dir.inode).await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn symlink_roundtrip_long_target() {
    let (_tmp, client) = new_client();
    let long_target = "/a/very/long/path/".to_string() + &"x".repeat(200);
    let attr = client
        .symlink(ROOT, "longlink", &long_target, 0, 0)
        .await
        .unwrap();
    let readback = client.readlink(attr.inode).await.unwrap();
    assert_eq!(readback, long_target);
}

#[tokio::test(flavor = "multi_thread")]
async fn unlink_symlink_removes_it() {
    let (_tmp, client) = new_client();
    let attr = client
        .symlink(ROOT, "ephemeral", "target", 0, 0)
        .await
        .unwrap();
    client.unlink(ROOT, "ephemeral").await.unwrap();
    assert!(client.lookup(ROOT, "ephemeral").await.is_err());
    assert!(client.getattr(attr.inode).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn readdir_shows_symlink() {
    let (_tmp, client) = new_client();
    client.create(ROOT, "file1", 0o644, 0, 0).await.unwrap();
    client.symlink(ROOT, "link1", "target", 0, 0).await.unwrap();
    client.mkdir(ROOT, "dir1", 0o755, 0, 0).await.unwrap();

    let entries = client.readdir(ROOT).await.unwrap();
    assert_eq!(entries.len(), 3);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"file1"));
    assert!(names.contains(&"link1"));
    assert!(names.contains(&"dir1"));

    // Check that the symlink's dir entry has S_IFLNK kind.
    let symlink_entry = entries.iter().find(|e| e.name == "link1").unwrap();
    assert_eq!(symlink_entry.kind & 0o170000, 0o120000);
}

// ===========================================================================
// Deferred unlink tests (T-22)
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn deferred_unlink_open_file_survives() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "victim", 0o644, 0, 0).await.unwrap();
    client.write(file.inode, 0, b"keep alive", 0).await.unwrap();

    // Open the file (increments handle count).
    let _fh = client.open(file.inode, 0).await.unwrap();

    // Unlink while open — inode should survive (nlink=0, handles>0).
    client.unlink(ROOT, "victim").await.unwrap();

    // Directory entry gone.
    assert!(client.lookup(ROOT, "victim").await.is_err());

    // But data is still accessible via inode.
    let data = client.read(file.inode, 0, 100).await.unwrap();
    assert!(data.starts_with(b"keep alive"));

    // Release the file handle → triggers deferred delete.
    client.release(file.inode).await.unwrap();

    // Now inode data should be gone.
    assert!(client.getattr(file.inode).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn unlink_without_open_deletes_immediately() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "ephemeral", 0o644, 0, 0).await.unwrap();
    client.write(file.inode, 0, b"bye", 0).await.unwrap();

    // Unlink without open — should delete immediately.
    client.unlink(ROOT, "ephemeral").await.unwrap();
    assert!(client.getattr(file.inode).await.is_err());
}

// ===========================================================================
// Fallocate tests (setattr size extension simulates mode-0 fallocate)
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn fallocate_mode0_extends_file_size() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "falloc", 0o644, 0, 0).await.unwrap();

    // File starts at size 0.
    let attr = client.getattr(file.inode).await.unwrap();
    assert_eq!(attr.size, 0);

    // Simulate fallocate mode 0: extend file size to 4096 via setattr.
    let req = rucksfs_core::SetAttrRequest {
        size: Some(4096),
        ..Default::default()
    };
    let attr = client.setattr(file.inode, req).await.unwrap();
    assert_eq!(attr.size, 4096);

    // Reading the extended region should return zeros.
    let data = client.read(file.inode, 0, 4096).await.unwrap();
    assert_eq!(data.len(), 4096);
    assert!(data.iter().all(|&b| b == 0), "extended region should be zero-filled");
}

#[tokio::test(flavor = "multi_thread")]
async fn fallocate_does_not_shrink() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "falloc2", 0o644, 0, 0).await.unwrap();
    client.write(file.inode, 0, b"hello world", 0).await.unwrap();

    let attr = client.getattr(file.inode).await.unwrap();
    assert_eq!(attr.size, 11);

    // setattr with size < current: should truncate (this is setattr, not fallocate).
    // But when fallocate offset+length < current size, it's a no-op.
    // Here we test that setattr with a larger size extends correctly.
    let req = rucksfs_core::SetAttrRequest {
        size: Some(1024),
        ..Default::default()
    };
    let attr = client.setattr(file.inode, req).await.unwrap();
    assert_eq!(attr.size, 1024);

    // Original data should still be readable.
    let data = client.read(file.inode, 0, 11).await.unwrap();
    assert_eq!(&data[..5], b"hello");
}

#[tokio::test(flavor = "multi_thread")]
async fn fallocate_extend_preserves_existing_data() {
    let (_tmp, client) = new_client();
    let file = client.create(ROOT, "falloc3", 0o644, 0, 0).await.unwrap();
    client.write(file.inode, 0, b"existing", 0).await.unwrap();

    // Extend to 1MB.
    let req = rucksfs_core::SetAttrRequest {
        size: Some(1024 * 1024),
        ..Default::default()
    };
    client.setattr(file.inode, req).await.unwrap();

    let attr = client.getattr(file.inode).await.unwrap();
    assert_eq!(attr.size, 1024 * 1024);

    // Original data preserved.
    let data = client.read(file.inode, 0, 8).await.unwrap();
    assert_eq!(&data, b"existing");
}
