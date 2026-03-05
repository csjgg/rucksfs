//! Tests for the fsck consistency checker.

use std::sync::Arc;

use rucksfs_core::{DataLocation, DataOps, MetadataOps, VfsOps};
use rucksfs_client::EmbeddedClient;
use rucksfs_dataserver::DataServer;
use rucksfs_server::MetadataServer;
use rucksfs_server::fsck::{self, FsckIssueKind};
use rucksfs_storage::{
    MetadataStore, RawDiskDataStore, RocksDeltaStore,
    RocksDirectoryIndex, RocksMetadataStore, RocksStorageBundle, open_rocks_db,
    encoding::{encode_inode_key, InodeValue},
};

const ROOT: u64 = 1;

/// Build a fresh stack and return individual components so fsck can access
/// the raw MetadataStore and DirectoryIndex.
fn new_stack() -> (
    tempfile::TempDir,
    EmbeddedClient,
    Arc<RocksMetadataStore>,
    Arc<RocksDirectoryIndex>,
) {
    let tmp = tempfile::tempdir().unwrap();
    let db = open_rocks_db(tmp.path().join("meta.db")).unwrap();
    let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
    let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
    let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
    let data_store =
        RawDiskDataStore::open(&tmp.path().join("data.raw"), 64 * 1024 * 1024).unwrap();

    let data_server: Arc<dyn DataOps> = Arc::new(DataServer::new(data_store));
    let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));
    let metadata_server: Arc<dyn MetadataOps> = Arc::new(MetadataServer::new(
        Arc::clone(&metadata),
        Arc::clone(&index),
        delta_store,
        Arc::clone(&data_server),
        DataLocation {
            address: "embedded".to_string(),
        },
        storage_bundle,
    ));

    let client = EmbeddedClient::new(metadata_server, data_server);
    (tmp, client, metadata, index)
}

// ===========================================================================
// Clean filesystem
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn fsck_clean_filesystem_with_root_only() {
    let (_tmp, _client, metadata, index) = new_stack();
    let report = fsck::check(metadata.as_ref(), index.as_ref()).unwrap();
    assert!(report.is_clean());
    assert_eq!(report.total_inodes, 1); // root only
}

#[tokio::test(flavor = "multi_thread")]
async fn fsck_clean_filesystem_after_creates() {
    let (_tmp, client, metadata, index) = new_stack();

    // Create some files and directories.
    client.create(ROOT, "file1", 0o644, 0, 0).await.unwrap();
    client.create(ROOT, "file2", 0o644, 0, 0).await.unwrap();
    client.mkdir(ROOT, "dir1", 0o755, 0, 0).await.unwrap();

    let report = fsck::check(metadata.as_ref(), index.as_ref()).unwrap();
    assert!(report.is_clean(), "issues: {:?}", report.issues);
    assert_eq!(report.total_inodes, 4); // root + 3
    assert_eq!(report.total_dir_entries, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn fsck_clean_with_hard_links() {
    let (_tmp, client, metadata, index) = new_stack();

    let file = client.create(ROOT, "original", 0o644, 0, 0).await.unwrap();
    client.link(ROOT, "hardlink", file.inode).await.unwrap();

    let report = fsck::check(metadata.as_ref(), index.as_ref()).unwrap();
    assert!(report.is_clean(), "issues: {:?}", report.issues);
    // 2 dir entries (original + hardlink) pointing to same inode.
    assert_eq!(report.total_dir_entries, 2);
}

// ===========================================================================
// Orphan inode detection
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn fsck_detects_orphan_inode() {
    let (_tmp, _client, metadata, index) = new_stack();

    // Inject an orphan inode directly into the metadata store.
    let orphan_ino: u64 = 9999;
    let orphan_iv = InodeValue {
        version: 1,
        inode: orphan_ino,
        size: 0,
        mode: 0o100644, // regular file
        nlink: 1,
        uid: 0,
        gid: 0,
        atime: 0,
        mtime: 0,
        ctime: 0,
    };
    let key = encode_inode_key(orphan_ino);
    metadata.put(&key, &orphan_iv.serialize()).unwrap();

    let report = fsck::check(metadata.as_ref(), index.as_ref()).unwrap();
    assert!(!report.is_clean());

    let orphan_issues: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.kind == FsckIssueKind::OrphanInode && i.inode == orphan_ino)
        .collect();
    assert_eq!(orphan_issues.len(), 1);
}

// ===========================================================================
// nlink mismatch detection
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn fsck_detects_nlink_mismatch() {
    let (_tmp, client, metadata, index) = new_stack();

    // Create a file (nlink=1 in metadata, 1 dir entry reference).
    let file = client.create(ROOT, "file1", 0o644, 0, 0).await.unwrap();

    // Corrupt the nlink to 5 — does not match the 1 directory reference.
    let key = encode_inode_key(file.inode);
    let raw = metadata.get(&key).unwrap().unwrap();
    let mut iv = InodeValue::deserialize(&raw).unwrap();
    iv.nlink = 5;
    metadata.put(&key, &iv.serialize()).unwrap();

    let report = fsck::check(metadata.as_ref(), index.as_ref()).unwrap();
    assert!(!report.is_clean());

    let nlink_issues: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.kind == FsckIssueKind::NlinkMismatch && i.inode == file.inode)
        .collect();
    assert_eq!(nlink_issues.len(), 1);
    assert!(nlink_issues[0].detail.contains("nlink=5"));
    assert!(nlink_issues[0].detail.contains("1 directory references"));
}

// ===========================================================================
// next_inode counter inconsistency
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn fsck_detects_counter_inconsistency() {
    let (_tmp, client, metadata, index) = new_stack();

    // Create a file so allocator advances.
    client.create(ROOT, "file1", 0o644, 0, 0).await.unwrap();

    // Corrupt the next_inode counter to 1 (less than the max inode).
    let counter_key = b"next_inode";
    let bad_counter: u64 = 1;
    metadata.put(counter_key, &bad_counter.to_be_bytes()).unwrap();

    let report = fsck::check(metadata.as_ref(), index.as_ref()).unwrap();
    assert!(!report.is_clean());

    let counter_issues: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.kind == FsckIssueKind::CounterInconsistency)
        .collect();
    assert_eq!(counter_issues.len(), 1);
}

// ===========================================================================
// Mixed issues
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn fsck_detects_multiple_issues() {
    let (_tmp, client, metadata, index) = new_stack();

    let file = client.create(ROOT, "file1", 0o644, 0, 0).await.unwrap();

    // Inject orphan inode.
    let orphan_ino: u64 = 8888;
    let orphan_iv = InodeValue {
        version: 1,
        inode: orphan_ino,
        size: 0,
        mode: 0o100644,
        nlink: 1,
        uid: 0,
        gid: 0,
        atime: 0,
        mtime: 0,
        ctime: 0,
    };
    metadata
        .put(&encode_inode_key(orphan_ino), &orphan_iv.serialize())
        .unwrap();

    // Corrupt nlink on existing file.
    let key = encode_inode_key(file.inode);
    let raw = metadata.get(&key).unwrap().unwrap();
    let mut iv = InodeValue::deserialize(&raw).unwrap();
    iv.nlink = 99;
    metadata.put(&key, &iv.serialize()).unwrap();

    let report = fsck::check(metadata.as_ref(), index.as_ref()).unwrap();
    assert!(!report.is_clean());
    assert!(report.issues.len() >= 2);

    let kinds: Vec<_> = report.issues.iter().map(|i| &i.kind).collect();
    assert!(kinds.contains(&&FsckIssueKind::OrphanInode));
    assert!(kinds.contains(&&FsckIssueKind::NlinkMismatch));
}

// ===========================================================================
// Report helpers
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn fsck_report_print_summary_does_not_panic() {
    let (_tmp, _client, metadata, index) = new_stack();
    let report = fsck::check(metadata.as_ref(), index.as_ref()).unwrap();
    // Should not panic.
    report.print_summary();
}
