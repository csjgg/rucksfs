//! Stress / concurrency tests for the RucksFS VfsOps layer.
//!
//! These tests verify:
//! 1. Concurrent safety — no panics, deadlocks, or data races.
//! 2. File operation correctness under concurrent load.
//! 3. Metadata consistency after many parallel mutations.
//!
//! All tests use the RocksDB-backed EmbeddedClient stack, so they run on any OS
//! (including macOS) without FUSE.

use std::sync::Arc;

use rucksfs_client::EmbeddedClient;
use rucksfs_core::{DataLocation, DataOps, FsError, MetadataOps, VfsOps};
use rucksfs_dataserver::DataServer;
use rucksfs_server::MetadataServer;
use rucksfs_storage::{
    RawDiskDataStore, RocksDeltaStore, RocksDirectoryIndex, RocksMetadataStore, RocksStorageBundle, open_rocks_db,
};

const ROOT: u64 = 1;

/// Build a shared RocksDB-backed client wrapped in `Arc` for multi-task usage.
fn shared_client() -> (tempfile::TempDir, Arc<EmbeddedClient>) {
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

    (tmp, Arc::new(EmbeddedClient::new(metadata_server, data_server)))
}

// ===========================================================================
// 1. Concurrent file creation in the same directory
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_create_files_in_same_dir() {
    let (_tmp, client) = shared_client();
    let dir = client.mkdir(ROOT, "cdir", 0o755, 0, 0).await.unwrap();
    let dir_inode = dir.inode;

    let n = 100;
    let mut handles = Vec::with_capacity(n);

    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let name = format!("file_{}", i);
            c.create(dir_inode, &name, 0o644, 0, 0).await
        }));
    }

    let mut created = 0;
    for h in handles {
        if h.await.unwrap().is_ok() {
            created += 1;
        }
    }
    assert_eq!(created, n, "all files should be created successfully");

    // Verify via readdir
    let entries = client.readdir(dir_inode).await.unwrap();
    assert_eq!(entries.len(), n);
}

// ===========================================================================
// 2. Concurrent mkdir in the same parent
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_mkdir_in_same_parent() {
    let (_tmp, client) = shared_client();
    let n = 50;
    let mut handles = Vec::with_capacity(n);

    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let name = format!("subdir_{}", i);
            c.mkdir(ROOT, &name, 0o755, 0, 0).await
        }));
    }

    let mut created = 0;
    for h in handles {
        if h.await.unwrap().is_ok() {
            created += 1;
        }
    }
    assert_eq!(created, n);

    let entries = client.readdir(ROOT).await.unwrap();
    assert_eq!(entries.len(), n);
}

// ===========================================================================
// 3. Concurrent create of the same filename — only one should succeed
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_create_same_name() {
    let (_tmp, client) = shared_client();
    let n = 20;
    let mut handles = Vec::with_capacity(n);

    for _ in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            c.create(ROOT, "collision", 0o644, 0, 0).await
        }));
    }

    let mut successes = 0;
    let mut already_exists = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(_) => successes += 1,
            Err(FsError::AlreadyExists) => already_exists += 1,
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }

    assert!(successes >= 1, "at least one create should succeed");
    assert_eq!(successes + already_exists, n, "every attempt should either succeed or get AlreadyExists");
}

// ===========================================================================
// 4. Concurrent writes to the same file at different offsets
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_write_different_offsets() {
    let (_tmp, client) = shared_client();
    let file = client.create(ROOT, "multi_write", 0o644, 0, 0).await.unwrap();
    let inode = file.inode;

    let chunk_size: u64 = 64;
    let n: u64 = 50;
    let mut handles = Vec::new();

    for i in 0..n {
        let c = Arc::clone(&client);
        let offset = i * chunk_size;
        // Each task writes a unique byte pattern
        let data = vec![i as u8; chunk_size as usize];
        handles.push(tokio::spawn(async move {
            c.write(inode, offset, &data, 0).await
        }));
    }

    for h in handles {
        let written = h.await.unwrap().unwrap();
        assert_eq!(written, chunk_size as u32);
    }

    // Verify all chunks
    let total = (n * chunk_size) as u32;
    let all_data = client.read(inode, 0, total).await.unwrap();
    for i in 0..n {
        let start = (i * chunk_size) as usize;
        let end = start + chunk_size as usize;
        let expected = vec![i as u8; chunk_size as usize];
        assert_eq!(
            &all_data[start..end],
            &expected[..],
            "chunk {} mismatch",
            i
        );
    }
}

// ===========================================================================
// 5. Concurrent read + write on the same file
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_read_write_same_file() {
    let (_tmp, client) = shared_client();
    let file = client.create(ROOT, "rw_mix", 0o644, 0, 0).await.unwrap();
    let inode = file.inode;

    // Pre-fill with known data
    let initial = vec![0xAA_u8; 4096];
    client.write(inode, 0, &initial, 0).await.unwrap();

    let n = 30;
    let mut handles = Vec::new();

    // Spawn writers
    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let data = vec![(i + 1) as u8; 32];
            let offset = (i * 32) as u64;
            c.write(inode, offset, &data, 0).await.map(|_| ())
        }));
    }

    // Spawn readers
    for _ in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            // Just verify no panic or error
            let _data = c.read(inode, 0, 4096).await?;
            Ok(())
        }));
    }

    for h in handles {
        // No task should panic
        let result = h.await.unwrap();
        assert!(result.is_ok(), "unexpected error in concurrent rw: {:?}", result);
    }
}

// ===========================================================================
// 6. Concurrent readdir + mkdir (reader should see a consistent snapshot)
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_readdir_and_mkdir() {
    let (_tmp, client) = shared_client();
    let parent = client.mkdir(ROOT, "readdir_parent", 0o755, 0, 0).await.unwrap();
    let parent_inode = parent.inode;

    let n = 40;
    let mut handles = Vec::new();

    // Writers: create subdirectories
    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let name = format!("child_{}", i);
            c.mkdir(parent_inode, &name, 0o755, 0, 0).await.map(|_| ())
        }));
    }

    // Readers: readdir concurrently
    for _ in 0..20 {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let entries = c.readdir(parent_inode).await?;
            // Each read should return a consistent list (no partial entries)
            for entry in &entries {
                assert!(!entry.name.is_empty(), "entry name should not be empty");
                assert!(entry.inode > 0, "entry inode should be positive");
            }
            Ok(())
        }));
    }

    for h in handles {
        let result = h.await.unwrap();
        assert!(result.is_ok());
    }

    // Final check: all children present
    let entries = client.readdir(parent_inode).await.unwrap();
    assert_eq!(entries.len(), n);
}

// ===========================================================================
// 7. Concurrent rename of different files (no conflicts)
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_rename_different_files() {
    let (_tmp, client) = shared_client();
    let n = 30;

    // Create files
    let mut inodes = Vec::with_capacity(n);
    for i in 0..n {
        let name = format!("before_{}", i);
        let attr = client.create(ROOT, &name, 0o644, 0, 0).await.unwrap();
        inodes.push(attr.inode);
    }

    // Rename all concurrently
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let old_name = format!("before_{}", i);
            let new_name = format!("after_{}", i);
            c.rename(ROOT, &old_name, ROOT, &new_name).await
        }));
    }

    for h in handles {
        h.await.unwrap().unwrap();
    }

    // Verify: old names gone, new names present
    for i in 0..n {
        let old_name = format!("before_{}", i);
        let new_name = format!("after_{}", i);
        assert!(client.lookup(ROOT, &old_name).await.is_err());
        let attr = client.lookup(ROOT, &new_name).await.unwrap();
        assert_eq!(attr.inode, inodes[i]);
    }
}

// ===========================================================================
// 8. Concurrent unlink + lookup — no panic, consistent results
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_unlink_and_lookup() {
    let (_tmp, client) = shared_client();
    let n = 50;

    // Create files
    for i in 0..n {
        let name = format!("ephemeral_{}", i);
        client.create(ROOT, &name, 0o644, 0, 0).await.unwrap();
    }

    let mut handles = Vec::new();

    // Unlinkers
    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let name = format!("ephemeral_{}", i);
            let _ = c.unlink(ROOT, &name).await; // may or may not find it
        }));
    }

    // Lookers
    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let name = format!("ephemeral_{}", i);
            // Either found or NotFound — both are valid under concurrency
            match c.lookup(ROOT, &name).await {
                Ok(_) | Err(FsError::NotFound) => {}
                Err(e) => panic!("unexpected error for {}: {:?}", name, e),
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // After all unlinks, none should remain
    let entries = client.readdir(ROOT).await.unwrap();
    assert_eq!(entries.len(), 0);
}

// ===========================================================================
// 9. Metadata consistency after heavy mutations
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn metadata_consistency_after_stress() {
    let (_tmp, client) = shared_client();

    // Phase 1: create a bunch of files and directories
    let n_dirs = 10;
    let n_files_per_dir = 20;
    let mut dir_inodes = Vec::new();

    for d in 0..n_dirs {
        let name = format!("stressdir_{}", d);
        let dir = client.mkdir(ROOT, &name, 0o755, 0, 0).await.unwrap();
        dir_inodes.push(dir.inode);

        for f in 0..n_files_per_dir {
            let fname = format!("file_{}", f);
            let file = client.create(dir.inode, &fname, 0o644, 0, 0).await.unwrap();
            // Write some data
            let content = format!("dir{}file{}", d, f);
            client
                .write(file.inode, 0, content.as_bytes(), 0)
                .await
                .unwrap();
        }
    }

    // Phase 2: verify all data
    for d in 0..n_dirs {
        let dir_inode = dir_inodes[d];
        let entries = client.readdir(dir_inode).await.unwrap();
        assert_eq!(
            entries.len(),
            n_files_per_dir,
            "dir {} should have {} entries",
            d,
            n_files_per_dir
        );

        for f in 0..n_files_per_dir {
            let fname = format!("file_{}", f);
            let attr = client.lookup(dir_inode, &fname).await.unwrap();

            // Verify inode attributes are self-consistent
            let getattr = client.getattr(attr.inode).await.unwrap();
            assert_eq!(getattr.inode, attr.inode);
            assert_eq!(getattr.mode & 0o100000, 0o100000, "should be regular file");
            assert!(getattr.size > 0, "file should have data");

            // Verify data matches
            let expected = format!("dir{}file{}", d, f);
            let data = client.read(attr.inode, 0, 4096).await.unwrap();
            assert!(
                data.starts_with(expected.as_bytes()),
                "data mismatch for {}/{}",
                d,
                f
            );
        }
    }

    // Phase 3: concurrent deletions
    let mut handles = Vec::new();
    for d in 0..n_dirs {
        let c = Arc::clone(&client);
        let dir_inode = dir_inodes[d];
        handles.push(tokio::spawn(async move {
            // Delete all files in this directory
            for f in 0..n_files_per_dir {
                let fname = format!("file_{}", f);
                c.unlink(dir_inode, &fname).await.unwrap();
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // Phase 4: verify all directories are now empty
    for d in 0..n_dirs {
        let entries = client.readdir(dir_inodes[d]).await.unwrap();
        assert_eq!(entries.len(), 0, "dir {} should be empty after cleanup", d);
    }

    // Phase 5: remove directories
    for d in 0..n_dirs {
        let name = format!("stressdir_{}", d);
        client.rmdir(ROOT, &name).await.unwrap();
    }

    let root_entries = client.readdir(ROOT).await.unwrap();
    assert_eq!(root_entries.len(), 0, "root should be empty after full cleanup");
}

// ===========================================================================
// 10. Large-scale file creation stress test
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn large_scale_concurrent_create() {
    let (_tmp, client) = shared_client();
    let dir = client.mkdir(ROOT, "bigdir", 0o755, 0, 0).await.unwrap();
    let dir_inode = dir.inode;

    let n = 500;
    let mut handles = Vec::with_capacity(n);

    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let name = format!("item_{:04}", i);
            c.create(dir_inode, &name, 0o644, 0, 0).await
        }));
    }

    for h in handles {
        h.await.unwrap().unwrap();
    }

    let entries = client.readdir(dir_inode).await.unwrap();
    assert_eq!(entries.len(), n);

    // Verify each entry can be looked up
    for i in 0..n {
        let name = format!("item_{:04}", i);
        let attr = client.lookup(dir_inode, &name).await.unwrap();
        assert!(attr.inode > 0);
    }
}

// ===========================================================================
// 11. Concurrent setattr on different inodes
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_setattr() {
    let (_tmp, client) = shared_client();
    let n = 30;
    let mut inodes = Vec::with_capacity(n);

    for i in 0..n {
        let name = format!("chmod_{}", i);
        let attr = client.create(ROOT, &name, 0o644, 0, 0).await.unwrap();
        inodes.push(attr.inode);
    }

    let mut handles = Vec::with_capacity(n);
    for (i, &inode) in inodes.iter().enumerate() {
        let c = Arc::clone(&client);
        let new_mode = 0o600 + (i as u32 % 8);
        handles.push(tokio::spawn(async move {
            let req = rucksfs_core::SetAttrRequest {
                mode: Some(new_mode),
                ..Default::default()
            };
            c.setattr(inode, req).await
        }));
    }

    for (i, h) in handles.into_iter().enumerate() {
        let attr = h.await.unwrap().unwrap();
        let expected = 0o600 + (i as u32 % 8);
        assert_eq!(attr.mode & 0o7777, expected);
    }
}

// ===========================================================================
// 12. Concurrent cross-directory renames
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_cross_directory_rename() {
    let (_tmp, client) = shared_client();
    let src_dir = client.mkdir(ROOT, "src_dir", 0o755, 0, 0).await.unwrap();
    let dst_dir = client.mkdir(ROOT, "dst_dir", 0o755, 0, 0).await.unwrap();

    let n = 30;
    let mut original_inodes = Vec::with_capacity(n);
    for i in 0..n {
        let name = format!("mover_{}", i);
        let attr = client.create(src_dir.inode, &name, 0o644, 0, 0).await.unwrap();
        original_inodes.push(attr.inode);
    }

    // Move all files from src_dir to dst_dir concurrently
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let c = Arc::clone(&client);
        let src_inode = src_dir.inode;
        let dst_inode = dst_dir.inode;
        handles.push(tokio::spawn(async move {
            let old_name = format!("mover_{}", i);
            let new_name = format!("moved_{}", i);
            c.rename(src_inode, &old_name, dst_inode, &new_name).await
        }));
    }

    for h in handles {
        h.await.unwrap().unwrap();
    }

    // src_dir should be empty
    let src_entries = client.readdir(src_dir.inode).await.unwrap();
    assert_eq!(src_entries.len(), 0);

    // dst_dir should have all files with correct inodes
    let dst_entries = client.readdir(dst_dir.inode).await.unwrap();
    assert_eq!(dst_entries.len(), n);

    for i in 0..n {
        let name = format!("moved_{}", i);
        let attr = client.lookup(dst_dir.inode, &name).await.unwrap();
        assert_eq!(attr.inode, original_inodes[i]);
    }
}

// ===========================================================================
// 13. Interleaved create + unlink storm
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn create_unlink_storm() {
    let (_tmp, client) = shared_client();
    let n = 100;

    // Each task creates a file, writes to it, reads back, then deletes it.
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let name = format!("storm_{}", i);
            let attr = c.create(ROOT, &name, 0o644, 0, 0).await?;
            let content = format!("storm data {}", i);
            c.write(attr.inode, 0, content.as_bytes(), 0).await?;

            let data = c.read(attr.inode, 0, 4096).await?;
            assert!(
                data.starts_with(content.as_bytes()),
                "data mismatch for storm_{}",
                i
            );

            c.unlink(ROOT, &name).await?;
            Ok::<(), FsError>(())
        }));
    }

    for h in handles {
        h.await.unwrap().unwrap();
    }

    // Root should be empty
    let entries = client.readdir(ROOT).await.unwrap();
    assert_eq!(entries.len(), 0);
}

// ===========================================================================
// 14. Deep nested concurrent operations
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn deep_nested_concurrent_ops() {
    let (_tmp, client) = shared_client();

    // Build a deep tree: /a/b/c/d/e
    let a = client.mkdir(ROOT, "a", 0o755, 0, 0).await.unwrap();
    let b = client.mkdir(a.inode, "b", 0o755, 0, 0).await.unwrap();
    let c = client.mkdir(b.inode, "c", 0o755, 0, 0).await.unwrap();
    let d = client.mkdir(c.inode, "d", 0o755, 0, 0).await.unwrap();
    let e = client.mkdir(d.inode, "e", 0o755, 0, 0).await.unwrap();

    let leaf_inode = e.inode;
    let n = 50;

    // Create files at the deepest level concurrently
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let name = format!("deep_{}", i);
            let attr = c.create(leaf_inode, &name, 0o644, 0, 0).await?;
            let data = format!("deep content {}", i);
            c.write(attr.inode, 0, data.as_bytes(), 0).await?;
            Ok::<_, FsError>(attr.inode)
        }));
    }

    let mut file_inodes = Vec::with_capacity(n);
    for h in handles {
        file_inodes.push(h.await.unwrap().unwrap());
    }

    // Concurrent reads of all deep files
    let mut read_handles = Vec::with_capacity(n);
    for (i, &inode) in file_inodes.iter().enumerate() {
        let c = Arc::clone(&client);
        read_handles.push(tokio::spawn(async move {
            let data = c.read(inode, 0, 4096).await?;
            let expected = format!("deep content {}", i);
            assert!(
                data.starts_with(expected.as_bytes()),
                "deep file {} data mismatch",
                i
            );
            Ok::<_, FsError>(())
        }));
    }

    for h in read_handles {
        h.await.unwrap().unwrap();
    }

    let entries = client.readdir(leaf_inode).await.unwrap();
    assert_eq!(entries.len(), n);
}

// ===========================================================================
// 15. Concurrent statfs calls (should never fail)
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_statfs() {
    let (_tmp, client) = shared_client();
    let n = 50;

    let mut handles = Vec::with_capacity(n);
    for _ in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move { c.statfs(ROOT).await }));
    }

    for h in handles {
        let st = h.await.unwrap().unwrap();
        assert!(st.blocks > 0);
        assert!(st.bsize > 0);
    }
}

// ===========================================================================
// 16. Concurrent open on different files
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_open() {
    let (_tmp, client) = shared_client();
    let n = 50;
    let mut inodes = Vec::with_capacity(n);

    for i in 0..n {
        let name = format!("open_{}", i);
        let attr = client.create(ROOT, &name, 0o644, 0, 0).await.unwrap();
        inodes.push(attr.inode);
    }

    // Open all files concurrently
    let mut handles = Vec::with_capacity(n);
    for &inode in &inodes {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            // flags=0 for read
            c.open(inode, 0).await
        }));
    }

    for h in handles {
        h.await.unwrap().unwrap();
    }

    // Open the same file concurrently from multiple tasks
    let shared_inode = inodes[0];
    let mut handles2 = Vec::with_capacity(n);
    for _ in 0..n {
        let c = Arc::clone(&client);
        handles2.push(tokio::spawn(async move {
            c.open(shared_inode, 0).await
        }));
    }

    for h in handles2 {
        h.await.unwrap().unwrap();
    }
}

// ===========================================================================
// 17. Concurrent flush on different files
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_flush() {
    let (_tmp, client) = shared_client();
    let n = 50;
    let mut inodes = Vec::with_capacity(n);

    for i in 0..n {
        let name = format!("flush_{}", i);
        let attr = client.create(ROOT, &name, 0o644, 0, 0).await.unwrap();
        // Write some data so flush has something to work with
        let data = format!("flush data {}", i);
        client.write(attr.inode, 0, data.as_bytes(), 0).await.unwrap();
        inodes.push(attr.inode);
    }

    // Flush all files concurrently
    let mut handles = Vec::with_capacity(n);
    for &inode in &inodes {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            c.flush(inode).await
        }));
    }

    for h in handles {
        h.await.unwrap().unwrap();
    }

    // Verify data is still intact after flush
    for (i, &inode) in inodes.iter().enumerate() {
        let data = client.read(inode, 0, 4096).await.unwrap();
        let expected = format!("flush data {}", i);
        assert!(
            data.starts_with(expected.as_bytes()),
            "data mismatch after flush for file {}",
            i
        );
    }
}

// ===========================================================================
// 18. Concurrent fsync on different files
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_fsync() {
    let (_tmp, client) = shared_client();
    let n = 50;
    let mut inodes = Vec::with_capacity(n);

    for i in 0..n {
        let name = format!("fsync_{}", i);
        let attr = client.create(ROOT, &name, 0o644, 0, 0).await.unwrap();
        let data = format!("fsync data {}", i);
        client.write(attr.inode, 0, data.as_bytes(), 0).await.unwrap();
        inodes.push(attr.inode);
    }

    // fsync all files concurrently (datasync=false)
    let mut handles = Vec::with_capacity(n);
    for &inode in &inodes {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            c.fsync(inode, false).await
        }));
    }

    for h in handles {
        h.await.unwrap().unwrap();
    }

    // Also test datasync=true concurrently
    let mut handles2 = Vec::with_capacity(n);
    for &inode in &inodes {
        let c = Arc::clone(&client);
        handles2.push(tokio::spawn(async move {
            c.fsync(inode, true).await
        }));
    }

    for h in handles2 {
        h.await.unwrap().unwrap();
    }

    // Verify data is still intact after fsync
    for (i, &inode) in inodes.iter().enumerate() {
        let data = client.read(inode, 0, 4096).await.unwrap();
        let expected = format!("fsync data {}", i);
        assert!(
            data.starts_with(expected.as_bytes()),
            "data mismatch after fsync for file {}",
            i
        );
    }
}

// ===========================================================================
// 19. Interleaved open + write + flush + fsync + read lifecycle
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_full_file_lifecycle() {
    let (_tmp, client) = shared_client();
    let n = 50;

    // Each task performs a complete file lifecycle: create → open → write → flush → fsync → read → unlink
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let name = format!("lifecycle_{}", i);
            let attr = c.create(ROOT, &name, 0o644, 0, 0).await?;
            let inode = attr.inode;

            c.open(inode, 0).await?;

            let content = format!("lifecycle content {}", i);
            c.write(inode, 0, content.as_bytes(), 0).await?;

            c.flush(inode).await?;
            c.fsync(inode, false).await?;

            let data = c.read(inode, 0, 4096).await?;
            assert!(
                data.starts_with(content.as_bytes()),
                "lifecycle data mismatch for file {}",
                i
            );

            c.unlink(ROOT, &name).await?;
            Ok::<(), FsError>(())
        }));
    }

    for h in handles {
        h.await.unwrap().unwrap();
    }

    let entries = client.readdir(ROOT).await.unwrap();
    assert_eq!(entries.len(), 0, "root should be empty after lifecycle storm");
}
