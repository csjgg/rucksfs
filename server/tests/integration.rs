//! Integration tests for the full MetadataServer + DataServer stack.
//!
//! These tests exercise the POSIX operation stack through `EmbeddedClient`,
//! which routes metadata requests to `MetadataServer` and data requests to
//! `DataServer`.

use std::sync::Arc;

use rucksfs_client::EmbeddedClient;
use rucksfs_core::{DataLocation, DataOps, FsError, MetadataOps, VfsOps};
use rucksfs_dataserver::DataServer;
use rucksfs_server::MetadataServer;
use rucksfs_storage::{DeltaStore, RawDiskDataStore, RocksDeltaStore, RocksDirectoryIndex, RocksMetadataStore, RocksStorageBundle, open_rocks_db};

/// Root inode constant.
const ROOT: u64 = 1;

/// Permission mode for a regular file.
const FILE_MODE: u32 = 0o644;

/// Permission mode for a directory.
const DIR_MODE: u32 = 0o755;

/// Build a fresh EmbeddedClient backed by RocksDB stores for each test.
/// Returns `(TempDir, EmbeddedClient)` — the TempDir must be kept alive for
/// the duration of the test so the underlying database is not deleted.
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

/// Build raw MetadataServer + DataServer for tests that need direct access to
/// compaction workers or delta stores.
fn new_server_and_data() -> (
    tempfile::TempDir,
    Arc<MetadataServer<RocksMetadataStore, RocksDirectoryIndex, RocksDeltaStore>>,
    Arc<dyn DataOps>,
) {
    let tmp = tempfile::tempdir().unwrap();
    let db = open_rocks_db(tmp.path().join("meta.db")).unwrap();
    let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
    let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
    let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
    let data_store = RawDiskDataStore::open(&tmp.path().join("data.raw"), 64 * 1024 * 1024).unwrap();

    let data_server: Arc<dyn DataOps> = Arc::new(DataServer::new(data_store));
    let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));
    let server = Arc::new(MetadataServer::new(
        metadata,
        index,
        delta_store,
        Arc::clone(&data_server),
        DataLocation {
            address: "embedded".to_string(),
        },
        storage_bundle,
    ));

    (tmp, server, data_server)
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

// ===========================================================================
// Root directory
// ===========================================================================

#[test]
fn root_directory_exists_after_init() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let attr = client.getattr(ROOT).await.unwrap();
        assert_eq!(attr.inode, ROOT);
        assert_ne!(attr.mode & 0o040000, 0); // S_IFDIR
        assert_eq!(attr.nlink, 2);
    });
}

#[test]
fn root_readdir_empty() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let entries = client.readdir(ROOT).await.unwrap();
        assert!(entries.is_empty());
    });
}

// ===========================================================================
// File lifecycle: create → write → read → getattr → unlink → NotFound
// ===========================================================================

#[test]
fn file_lifecycle() {
    rt().block_on(async {
        let (_tmp, client) = new_client();

        // Create
        let attr = client.create(ROOT, "hello.txt", FILE_MODE, 0, 0).await.unwrap();
        assert_eq!(attr.nlink, 1);
        assert_eq!(attr.size, 0);
        assert_ne!(attr.mode & 0o100000, 0);

        let inode = attr.inode;

        // Open
        let fh = client.open(inode, 0).await.unwrap();
        assert_eq!(fh, 0);

        // Write
        let data = b"Hello, RucksFS!";
        let written = client.write(inode, 0, data, 0).await.unwrap();
        assert_eq!(written, data.len() as u32);

        // Read back
        let buf = client.read(inode, 0, data.len() as u32).await.unwrap();
        assert_eq!(&buf, data);

        // Getattr should reflect new size
        let attr2 = client.getattr(inode).await.unwrap();
        assert_eq!(attr2.size, data.len() as u64);

        // Unlink
        client.unlink(ROOT, "hello.txt").await.unwrap();

        // Lookup should fail
        let err = client.lookup(ROOT, "hello.txt").await.unwrap_err();
        assert!(matches!(err, FsError::NotFound));
    });
}

// ===========================================================================
// Directory operations
// ===========================================================================

#[test]
fn mkdir_and_readdir() {
    rt().block_on(async {
        let (_tmp, client) = new_client();

        let dir_attr = client.mkdir(ROOT, "subdir", DIR_MODE, 0, 0).await.unwrap();
        assert_ne!(dir_attr.mode & 0o040000, 0);
        assert_eq!(dir_attr.nlink, 2);

        let root_attr = client.getattr(ROOT).await.unwrap();
        assert_eq!(root_attr.nlink, 3);

        let entries = client.readdir(ROOT).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "subdir");
    });
}

#[test]
fn rmdir_empty_directory() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        client.mkdir(ROOT, "empty_dir", DIR_MODE, 0, 0).await.unwrap();
        client.rmdir(ROOT, "empty_dir").await.unwrap();

        let entries = client.readdir(ROOT).await.unwrap();
        assert!(entries.is_empty());

        let root_attr = client.getattr(ROOT).await.unwrap();
        assert_eq!(root_attr.nlink, 2);
    });
}

#[test]
fn rmdir_non_empty_fails() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let dir_attr = client.mkdir(ROOT, "mydir", DIR_MODE, 0, 0).await.unwrap();
        client
            .create(dir_attr.inode, "file.txt", FILE_MODE, 0, 0)
            .await
            .unwrap();

        let err = client.rmdir(ROOT, "mydir").await.unwrap_err();
        assert!(matches!(err, FsError::DirectoryNotEmpty));
    });
}

#[test]
fn rmdir_non_directory_fails() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        client.create(ROOT, "file.txt", FILE_MODE, 0, 0).await.unwrap();
        let err = client.rmdir(ROOT, "file.txt").await.unwrap_err();
        assert!(matches!(err, FsError::NotADirectory));
    });
}

// ===========================================================================
// Duplicate name detection
// ===========================================================================

#[test]
fn create_duplicate_name_fails() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        client.create(ROOT, "dup.txt", FILE_MODE, 0, 0).await.unwrap();
        let err = client.create(ROOT, "dup.txt", FILE_MODE, 0, 0).await.unwrap_err();
        assert!(matches!(err, FsError::AlreadyExists));
    });
}

#[test]
fn mkdir_duplicate_name_fails() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        client.mkdir(ROOT, "dup_dir", DIR_MODE, 0, 0).await.unwrap();
        let err = client.mkdir(ROOT, "dup_dir", DIR_MODE, 0, 0).await.unwrap_err();
        assert!(matches!(err, FsError::AlreadyExists));
    });
}

// ===========================================================================
// Unlink a directory should fail
// ===========================================================================

#[test]
fn unlink_directory_fails() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        client.mkdir(ROOT, "dir", DIR_MODE, 0, 0).await.unwrap();
        let err = client.unlink(ROOT, "dir").await.unwrap_err();
        assert!(matches!(err, FsError::IsADirectory));
    });
}

// ===========================================================================
// Lookup
// ===========================================================================

#[test]
fn lookup_nonexistent_returns_not_found() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let err = client.lookup(ROOT, "ghost").await.unwrap_err();
        assert!(matches!(err, FsError::NotFound));
    });
}

#[test]
fn lookup_after_create() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let created = client.create(ROOT, "found.txt", FILE_MODE, 0, 0).await.unwrap();
        let looked_up = client.lookup(ROOT, "found.txt").await.unwrap();
        assert_eq!(created.inode, looked_up.inode);
    });
}

// ===========================================================================
// Rename operations
// ===========================================================================

#[test]
fn rename_same_directory() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let attr = client.create(ROOT, "old.txt", FILE_MODE, 0, 0).await.unwrap();
        let inode = attr.inode;

        client
            .rename(ROOT, "old.txt", ROOT, "new.txt")
            .await
            .unwrap();

        assert!(client.lookup(ROOT, "old.txt").await.is_err());
        let new_attr = client.lookup(ROOT, "new.txt").await.unwrap();
        assert_eq!(new_attr.inode, inode);
    });
}

#[test]
fn rename_cross_directory() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let dir_a = client.mkdir(ROOT, "dir_a", DIR_MODE, 0, 0).await.unwrap();
        let dir_b = client.mkdir(ROOT, "dir_b", DIR_MODE, 0, 0).await.unwrap();

        let file = client
            .create(dir_a.inode, "file.txt", FILE_MODE, 0, 0)
            .await
            .unwrap();
        client
            .rename(dir_a.inode, "file.txt", dir_b.inode, "moved.txt")
            .await
            .unwrap();

        assert!(client.lookup(dir_a.inode, "file.txt").await.is_err());
        let moved = client.lookup(dir_b.inode, "moved.txt").await.unwrap();
        assert_eq!(moved.inode, file.inode);
    });
}

#[test]
fn rename_overwrite_file() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        client.create(ROOT, "src.txt", FILE_MODE, 0, 0).await.unwrap();
        let src = client.lookup(ROOT, "src.txt").await.unwrap();

        client.create(ROOT, "dst.txt", FILE_MODE, 0, 0).await.unwrap();
        client
            .rename(ROOT, "src.txt", ROOT, "dst.txt")
            .await
            .unwrap();

        let dst = client.lookup(ROOT, "dst.txt").await.unwrap();
        assert_eq!(dst.inode, src.inode);
        assert!(client.lookup(ROOT, "src.txt").await.is_err());

        let entries = client.readdir(ROOT).await.unwrap();
        assert_eq!(entries.len(), 1);
    });
}

#[test]
fn rename_dir_cross_directory() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let parent_a = client.mkdir(ROOT, "a", DIR_MODE, 0, 0).await.unwrap();
        let parent_b = client.mkdir(ROOT, "b", DIR_MODE, 0, 0).await.unwrap();
        let child = client
            .mkdir(parent_a.inode, "child", DIR_MODE, 0, 0)
            .await
            .unwrap();

        client
            .rename(parent_a.inode, "child", parent_b.inode, "child_moved")
            .await
            .unwrap();

        let moved = client.lookup(parent_b.inode, "child_moved").await.unwrap();
        assert_eq!(moved.inode, child.inode);

        let a = client.getattr(parent_a.inode).await.unwrap();
        assert_eq!(a.nlink, 2);

        let b = client.getattr(parent_b.inode).await.unwrap();
        assert_eq!(b.nlink, 3);
    });
}

// ===========================================================================
// Statfs
// ===========================================================================

#[test]
fn statfs_returns_reasonable_values() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let st = client.statfs(ROOT).await.unwrap();
        assert!(st.blocks > 0);
        assert!(st.bsize > 0);
        assert!(st.namelen > 0);
    });
}

// ===========================================================================
// Data integrity
// ===========================================================================

#[test]
fn write_read_large_block() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let attr = client.create(ROOT, "big.bin", FILE_MODE, 0, 0).await.unwrap();
        let inode = attr.inode;

        let pattern: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
        let written = client.write(inode, 0, &pattern, 0).await.unwrap();
        assert_eq!(written, 65536);

        let data = client.read(inode, 0, 65536).await.unwrap();
        assert_eq!(data, pattern);
    });
}

#[test]
fn write_at_offset_preserves_earlier_data() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let attr = client.create(ROOT, "sparse.bin", FILE_MODE, 0, 0).await.unwrap();
        let inode = attr.inode;

        client.write(inode, 0, b"AAAA", 0).await.unwrap();
        client.write(inode, 100, b"BBBB", 0).await.unwrap();

        let head = client.read(inode, 0, 4).await.unwrap();
        assert_eq!(&head, b"AAAA");

        let tail = client.read(inode, 100, 4).await.unwrap();
        assert_eq!(&tail, b"BBBB");

        let gap = client.read(inode, 4, 10).await.unwrap();
        assert!(gap.iter().all(|&b| b == 0));
    });
}

#[test]
fn flush_and_fsync() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let attr = client.create(ROOT, "sync.txt", FILE_MODE, 0, 0).await.unwrap();
        let inode = attr.inode;

        client.write(inode, 0, b"data", 0).await.unwrap();
        client.flush(inode).await.unwrap();
        client.fsync(inode, false).await.unwrap();
        client.fsync(inode, true).await.unwrap();
    });
}

// ===========================================================================
// Open checks
// ===========================================================================

#[test]
fn open_directory_fails() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let dir = client.mkdir(ROOT, "dir", DIR_MODE, 0, 0).await.unwrap();
        let err = client.open(dir.inode, 0).await.unwrap_err();
        assert!(matches!(err, FsError::IsADirectory));
    });
}

#[test]
fn open_nonexistent_fails() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let err = client.open(9999, 0).await.unwrap_err();
        assert!(matches!(err, FsError::NotFound));
    });
}

// ===========================================================================
// Nested directory operations
// ===========================================================================

#[test]
fn nested_directories_and_files() {
    rt().block_on(async {
        let (_tmp, client) = new_client();

        let d1 = client.mkdir(ROOT, "level1", DIR_MODE, 0, 0).await.unwrap();
        let d2 = client.mkdir(d1.inode, "level2", DIR_MODE, 0, 0).await.unwrap();
        let f = client
            .create(d2.inode, "deep.txt", FILE_MODE, 0, 0)
            .await
            .unwrap();

        client.write(f.inode, 0, b"deep content", 0).await.unwrap();
        let data = client.read(f.inode, 0, 20).await.unwrap();
        assert_eq!(&data[..12], b"deep content");

        let l1 = client.lookup(ROOT, "level1").await.unwrap();
        assert_eq!(l1.inode, d1.inode);
        let l2 = client.lookup(d1.inode, "level2").await.unwrap();
        assert_eq!(l2.inode, d2.inode);
        let lf = client.lookup(d2.inode, "deep.txt").await.unwrap();
        assert_eq!(lf.inode, f.inode);
    });
}

// ===========================================================================
// Concurrent safety (using tokio tasks)
// ===========================================================================

#[test]
fn concurrent_create_unlink() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let client = Arc::new(client);
        let n = 100usize;
        let mut handles = vec![];

        for i in 0..n {
            let c = Arc::clone(&client);
            handles.push(tokio::spawn(async move {
                let name = format!("file_{}", i);
                c.create(ROOT, &name, FILE_MODE, 0, 0).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let entries = client.readdir(ROOT).await.unwrap();
        assert_eq!(entries.len(), n);

        let mut handles = vec![];
        for i in 0..n {
            let c = Arc::clone(&client);
            handles.push(tokio::spawn(async move {
                let name = format!("file_{}", i);
                c.unlink(ROOT, &name).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let entries = client.readdir(ROOT).await.unwrap();
        assert!(entries.is_empty());
    });
}

#[test]
fn concurrent_write_read_different_inodes() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let client = Arc::new(client);

        let mut inodes = vec![];
        for i in 0..10 {
            let name = format!("cfile_{}", i);
            let attr = client.create(ROOT, &name, FILE_MODE, 0, 0).await.unwrap();
            inodes.push(attr.inode);
        }

        let mut handles = vec![];
        for (idx, &inode) in inodes.iter().enumerate() {
            let c = Arc::clone(&client);
            handles.push(tokio::spawn(async move {
                let data = format!("content for inode {}", idx);
                c.write(inode, 0, data.as_bytes(), 0).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        for (idx, &inode) in inodes.iter().enumerate() {
            let expected = format!("content for inode {}", idx);
            let data = client.read(inode, 0, expected.len() as u32).await.unwrap();
            assert_eq!(data, expected.as_bytes());
        }
    });
}

#[test]
fn concurrent_mkdir_rmdir() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let client = Arc::new(client);
        let n = 50usize;

        let mut handles = vec![];
        for i in 0..n {
            let c = Arc::clone(&client);
            handles.push(tokio::spawn(async move {
                let name = format!("dir_{}", i);
                c.mkdir(ROOT, &name, DIR_MODE, 0, 0).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let entries = client.readdir(ROOT).await.unwrap();
        assert_eq!(entries.len(), n);

        let mut handles = vec![];
        for i in 0..n {
            let c = Arc::clone(&client);
            handles.push(tokio::spawn(async move {
                let name = format!("dir_{}", i);
                c.rmdir(ROOT, &name).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let entries = client.readdir(ROOT).await.unwrap();
        assert!(entries.is_empty());

        let root_attr = client.getattr(ROOT).await.unwrap();
        assert_eq!(root_attr.nlink, 2);
    });
}

// ===========================================================================
// Delta entries — correctness & concurrency
// ===========================================================================

#[test]
fn delta_fold_many_creates() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let n = 100usize;
        for i in 0..n {
            let name = format!("f_{}", i);
            client.create(ROOT, &name, FILE_MODE, 0, 0).await.unwrap();
        }

        let root = client.getattr(ROOT).await.unwrap();
        assert_eq!(root.nlink, 2);
        let entries = client.readdir(ROOT).await.unwrap();
        assert_eq!(entries.len(), n);
    });
}

#[test]
fn delta_fold_many_mkdirs() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let n = 100usize;
        for i in 0..n {
            let name = format!("d_{}", i);
            client.mkdir(ROOT, &name, DIR_MODE, 0, 0).await.unwrap();
        }

        let root = client.getattr(ROOT).await.unwrap();
        assert_eq!(root.nlink, 2 + n as u32);
        let entries = client.readdir(ROOT).await.unwrap();
        assert_eq!(entries.len(), n);
    });
}

#[test]
fn delta_fold_mkdir_rmdir_nlink() {
    rt().block_on(async {
        let (_tmp, client) = new_client();

        for i in 0..20 {
            client
                .mkdir(ROOT, &format!("d_{}", i), DIR_MODE, 0, 0)
                .await
                .unwrap();
        }
        assert_eq!(client.getattr(ROOT).await.unwrap().nlink, 22);

        for i in 0..10 {
            client.rmdir(ROOT, &format!("d_{}", i)).await.unwrap();
        }
        assert_eq!(client.getattr(ROOT).await.unwrap().nlink, 12);

        for i in 10..20 {
            client.rmdir(ROOT, &format!("d_{}", i)).await.unwrap();
        }
        assert_eq!(client.getattr(ROOT).await.unwrap().nlink, 2);
    });
}

#[test]
fn concurrent_create_storm() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let client = Arc::new(client);
        let n = 200usize;

        let mut handles = vec![];
        for i in 0..n {
            let c = Arc::clone(&client);
            handles.push(tokio::spawn(async move {
                let name = format!("storm_{}", i);
                c.create(ROOT, &name, FILE_MODE, 0, 0).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let entries = client.readdir(ROOT).await.unwrap();
        assert_eq!(entries.len(), n);

        let root = client.getattr(ROOT).await.unwrap();
        assert!(root.mtime > 0);
    });
}

#[test]
fn concurrent_mkdir_storm_nlink() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let client = Arc::new(client);
        let n = 100usize;

        let mut handles = vec![];
        for i in 0..n {
            let c = Arc::clone(&client);
            handles.push(tokio::spawn(async move {
                let name = format!("sd_{}", i);
                c.mkdir(ROOT, &name, DIR_MODE, 0, 0).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let root = client.getattr(ROOT).await.unwrap();
        assert_eq!(root.nlink, 2 + n as u32);
    });
}

// ===========================================================================
// Compaction tests — need direct access to MetadataServer internals
// ===========================================================================

#[test]
fn compaction_flush_merges_deltas() {
    rt().block_on(async {
        let (_tmp, server, data_server) = new_server_and_data();

        for i in 0..50 {
            server
                .mkdir(ROOT, &format!("cd_{}", i), DIR_MODE, 0, 0)
                .await
                .unwrap();
        }

        assert_eq!(server.getattr(ROOT).await.unwrap().nlink, 52);

        let flushed = server.compaction.flush_all().unwrap();
        assert!(flushed > 0);

        assert_eq!(server.getattr(ROOT).await.unwrap().nlink, 52);

        let remaining = server.delta_store.scan_deltas(ROOT).unwrap();
        assert!(remaining.is_empty());

        let _ = data_server;
    });
}

#[test]
fn compaction_interleaved_with_writes() {
    rt().block_on(async {
        let (_tmp, server, data_server) = new_server_and_data();

        // Phase 1
        for i in 0..10 {
            server
                .mkdir(ROOT, &format!("p1_{}", i), DIR_MODE, 0, 0)
                .await
                .unwrap();
        }
        assert_eq!(server.getattr(ROOT).await.unwrap().nlink, 12);

        server.compaction.flush_all().unwrap();
        assert_eq!(server.getattr(ROOT).await.unwrap().nlink, 12);

        // Phase 2
        for i in 0..10 {
            server
                .mkdir(ROOT, &format!("p2_{}", i), DIR_MODE, 0, 0)
                .await
                .unwrap();
        }
        assert_eq!(server.getattr(ROOT).await.unwrap().nlink, 22);

        // Phase 3
        for i in 0..5 {
            server.rmdir(ROOT, &format!("p1_{}", i)).await.unwrap();
        }
        assert_eq!(server.getattr(ROOT).await.unwrap().nlink, 17);

        server.compaction.flush_all().unwrap();
        assert_eq!(server.getattr(ROOT).await.unwrap().nlink, 17);
        assert!(server.delta_store.scan_deltas(ROOT).unwrap().is_empty());

        let _ = data_server;
    });
}

// ===========================================================================
// MetadataServer-specific: unlink nlink=0 → data_client.delete_data
// ===========================================================================

#[test]
fn unlink_nlink_zero_deletes_data() {
    rt().block_on(async {
        let (_tmp, client) = new_client();

        let attr = client.create(ROOT, "will_delete.txt", FILE_MODE, 0, 0).await.unwrap();
        let inode = attr.inode;

        // Write data
        client.write(inode, 0, b"some data", 0).await.unwrap();
        let data = client.read(inode, 0, 9).await.unwrap();
        assert_eq!(&data, b"some data");

        // Unlink (nlink goes to 0)
        client.unlink(ROOT, "will_delete.txt").await.unwrap();

        // Data should have been cleaned up by DataServer
        // (reading the inode should return zeros since it's been deleted)
        let data = client.read(inode, 0, 9).await.unwrap();
        assert_eq!(data, vec![0u8; 9]);
    });
}

// ===========================================================================
// MetadataServer-specific: setattr size change → data_client.truncate
// ===========================================================================

#[test]
fn setattr_truncate_delegates_to_data_server() {
    rt().block_on(async {
        let (_tmp, client) = new_client();

        let attr = client.create(ROOT, "trunc.txt", FILE_MODE, 0, 0).await.unwrap();
        let inode = attr.inode;

        // Write 100 bytes
        let data = vec![0xAA; 100];
        client.write(inode, 0, &data, 0).await.unwrap();
        assert_eq!(client.getattr(inode).await.unwrap().size, 100);

        // Setattr to truncate to 50
        let req = rucksfs_core::SetAttrRequest {
            size: Some(50),
            ..Default::default()
        };
        let new_attr = client.setattr(inode, req).await.unwrap();
        assert_eq!(new_attr.size, 50);

        // Data beyond 50 should be zeros
        let read_data = client.read(inode, 0, 100).await.unwrap();
        assert_eq!(&read_data[..50], &[0xAA; 50]);
        assert_eq!(&read_data[50..], &[0u8; 50]);
    });
}

// ===========================================================================
// TOCTOU race: concurrent rmdir + create into the same directory
// ===========================================================================

/// Stress test for the rmdir / create TOCTOU race (T-12).
///
/// Scenario: one task repeatedly tries to rmdir a directory while another
/// task concurrently creates a file inside it.  Without a transactional
/// `is_dir_empty` check, the rmdir could observe an empty directory (via
/// non-transactional `list_dir`) and commit the delete even though the
/// create has already placed a new entry in the dir_entries CF.
///
/// After the fix (using `batch.is_dir_empty()` inside the PCC transaction),
/// the rmdir should either:
/// - Fail with `DirectoryNotEmpty` (create committed first), or
/// - Succeed, and the create should then fail with `NotFound` (rmdir
///   committed first, parent dir entry gone — or get a TransactionConflict).
///
/// The invariant we verify: at no point should the directory be deleted
/// while it still contains children.
#[test]
fn rmdir_create_race_no_orphan_entries() {
    use std::sync::atomic::{AtomicBool, Ordering};

    rt().block_on(async {
        let (_tmp, client) = new_client();
        let client = Arc::new(client);
        let iterations = 200;
        let orphan_detected = Arc::new(AtomicBool::new(false));

        for round in 0..iterations {
            let dir_name = format!("race_dir_{}", round);
            let dir_attr = client.mkdir(ROOT, &dir_name, DIR_MODE, 0, 0).await.unwrap();
            let dir_inode = dir_attr.inode;

            let c1 = Arc::clone(&client);
            let c2 = Arc::clone(&client);
            let dn = dir_name.clone();
            let orphan = Arc::clone(&orphan_detected);

            // Spawn: create a file inside the directory
            let create_handle = tokio::spawn(async move {
                c1.create(dir_inode, "child.txt", FILE_MODE, 0, 0).await
            });

            // Spawn: rmdir the directory
            let rmdir_handle = tokio::spawn(async move {
                c2.rmdir(ROOT, &dn).await
            });

            let (create_res, rmdir_res) = tokio::join!(create_handle, rmdir_handle);
            let create_res = create_res.unwrap();
            let rmdir_res = rmdir_res.unwrap();

            match (create_res.is_ok(), rmdir_res.is_ok()) {
                (true, true) => {
                    // Both succeeded — this is the TOCTOU bug!
                    // The directory was removed but a child entry was created.
                    orphan.store(true, Ordering::SeqCst);
                }
                (true, false) => {
                    // Create won, rmdir should have gotten DirectoryNotEmpty.
                    // Clean up: unlink the child, then rmdir.
                    let _ = client.unlink(dir_inode, "child.txt").await;
                    let _ = client.rmdir(ROOT, &dir_name).await;
                }
                (false, true) => {
                    // Rmdir won, create failed (NotFound or TransactionConflict).
                    // Directory is already gone — nothing to clean up.
                }
                (false, false) => {
                    // Both failed — possible under heavy contention. Clean up.
                    let _ = client.rmdir(ROOT, &dir_name).await;
                }
            }
        }

        assert!(
            !orphan_detected.load(Ordering::SeqCst),
            "TOCTOU race detected: rmdir succeeded while create also succeeded (orphan entry)"
        );
    });
}

/// Verify that rename onto a non-empty directory correctly fails with
/// `DirectoryNotEmpty`, even under concurrent create pressure.
#[test]
fn rename_onto_nonempty_dir_race() {
    rt().block_on(async {
        let (_tmp, client) = new_client();
        let client = Arc::new(client);
        let iterations = 100;

        for round in 0..iterations {
            let src_name = format!("rsrc_{}", round);
            let dst_name = format!("rdst_{}", round);

            // Create source dir and destination dir.
            let _src = client.mkdir(ROOT, &src_name, DIR_MODE, 0, 0).await.unwrap();
            let dst = client.mkdir(ROOT, &dst_name, DIR_MODE, 0, 0).await.unwrap();
            let dst_inode = dst.inode;

            let c1 = Arc::clone(&client);
            let c2 = Arc::clone(&client);
            let sn = src_name.clone();
            let dn = dst_name.clone();

            // Spawn: create a file inside the destination directory.
            let create_handle = tokio::spawn(async move {
                c1.create(dst_inode, "blocker.txt", FILE_MODE, 0, 0).await
            });

            // Spawn: rename source onto destination (should fail if dst non-empty).
            let rename_handle = tokio::spawn(async move {
                c2.rename(ROOT, &sn, ROOT, &dn).await
            });

            let (create_res, rename_res) = tokio::join!(create_handle, rename_handle);
            let create_ok = create_res.unwrap().is_ok();
            let rename_ok = rename_res.unwrap().is_ok();

            if create_ok && rename_ok {
                panic!(
                    "TOCTOU race in rename: both create and rename-over-dir succeeded in round {}",
                    round
                );
            }

            // Clean up for next round.
            if create_ok {
                let _ = client.unlink(dst_inode, "blocker.txt").await;
            }
            let _ = client.rmdir(ROOT, &src_name).await;
            let _ = client.rmdir(ROOT, &dst_name).await;
        }
    });
}
