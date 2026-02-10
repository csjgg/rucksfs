//! Integration tests for MetadataServer with in-memory storage backends.
//!
//! These tests exercise the full POSIX operation stack through the
//! `MetadataServer<MemoryMetadataStore, MemoryDataStore, MemoryDirectoryIndex>`
//! concrete type.

use std::sync::Arc;

use rucksfs_core::{FsError, PosixOps};
use rucksfs_server::MetadataServer;
use rucksfs_storage::{MemoryDataStore, MemoryDirectoryIndex, MemoryMetadataStore};

/// Root inode constant.
const ROOT: u64 = 1;

/// Permission mode for a regular file.
const FILE_MODE: u32 = 0o644;

/// Permission mode for a directory.
const DIR_MODE: u32 = 0o755;

type TestServer = MetadataServer<MemoryMetadataStore, MemoryDataStore, MemoryDirectoryIndex>;

/// Helper to build a fresh server for each test.
fn new_server() -> TestServer {
    MetadataServer::new(
        Arc::new(MemoryMetadataStore::new()),
        Arc::new(MemoryDataStore::new()),
        Arc::new(MemoryDirectoryIndex::new()),
    )
}

// ===========================================================================
// Root directory
// ===========================================================================

#[test]
fn root_directory_exists_after_init() {
    let server = new_server();
    let attr = server.getattr(ROOT).unwrap();
    assert_eq!(attr.inode, ROOT);
    // Should be a directory (S_IFDIR = 0o040000)
    assert_ne!(attr.mode & 0o040000, 0);
    assert_eq!(attr.nlink, 2); // "." and ".."
}

#[test]
fn root_readdir_empty() {
    let server = new_server();
    let entries = server.readdir(ROOT).unwrap();
    assert!(entries.is_empty());
}

// ===========================================================================
// File lifecycle: create → write → read → getattr → unlink → NotFound
// ===========================================================================

#[test]
fn file_lifecycle() {
    let server = new_server();

    // Create
    let attr = server.create(ROOT, "hello.txt", FILE_MODE).unwrap();
    assert_eq!(attr.nlink, 1);
    assert_eq!(attr.size, 0);
    assert_ne!(attr.mode & 0o100000, 0); // S_IFREG

    let inode = attr.inode;

    // Open
    let fh = server.open(inode, 0).unwrap();
    assert_eq!(fh, 0);

    // Write
    let data = b"Hello, RucksFS!";
    let written = server.write(inode, 0, data, 0).unwrap();
    assert_eq!(written, data.len() as u32);

    // Read back
    let buf = server.read(inode, 0, data.len() as u32).unwrap();
    assert_eq!(&buf, data);

    // Getattr should reflect new size
    let attr2 = server.getattr(inode).unwrap();
    assert_eq!(attr2.size, data.len() as u64);

    // Unlink
    server.unlink(ROOT, "hello.txt").unwrap();

    // Lookup should fail
    let err = server.lookup(ROOT, "hello.txt").unwrap_err();
    assert!(matches!(err, FsError::NotFound));
}

// ===========================================================================
// Directory operations
// ===========================================================================

#[test]
fn mkdir_and_readdir() {
    let server = new_server();

    let dir_attr = server.mkdir(ROOT, "subdir", DIR_MODE).unwrap();
    assert_ne!(dir_attr.mode & 0o040000, 0); // S_IFDIR
    assert_eq!(dir_attr.nlink, 2);

    // Root nlink should be 3 now (. + .. + subdir's "..")
    let root_attr = server.getattr(ROOT).unwrap();
    assert_eq!(root_attr.nlink, 3);

    // Readdir root should list "subdir"
    let entries = server.readdir(ROOT).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "subdir");
}

#[test]
fn rmdir_empty_directory() {
    let server = new_server();
    server.mkdir(ROOT, "empty_dir", DIR_MODE).unwrap();

    server.rmdir(ROOT, "empty_dir").unwrap();

    let entries = server.readdir(ROOT).unwrap();
    assert!(entries.is_empty());

    // Root nlink back to 2
    let root_attr = server.getattr(ROOT).unwrap();
    assert_eq!(root_attr.nlink, 2);
}

#[test]
fn rmdir_non_empty_fails() {
    let server = new_server();
    let dir_attr = server.mkdir(ROOT, "mydir", DIR_MODE).unwrap();
    let dir_inode = dir_attr.inode;

    // Create a file inside
    server.create(dir_inode, "file.txt", FILE_MODE).unwrap();

    // rmdir should fail
    let err = server.rmdir(ROOT, "mydir").unwrap_err();
    assert!(matches!(err, FsError::DirectoryNotEmpty));
}

#[test]
fn rmdir_non_directory_fails() {
    let server = new_server();
    server.create(ROOT, "file.txt", FILE_MODE).unwrap();

    let err = server.rmdir(ROOT, "file.txt").unwrap_err();
    assert!(matches!(err, FsError::NotADirectory));
}

// ===========================================================================
// Duplicate name detection
// ===========================================================================

#[test]
fn create_duplicate_name_fails() {
    let server = new_server();
    server.create(ROOT, "dup.txt", FILE_MODE).unwrap();

    let err = server.create(ROOT, "dup.txt", FILE_MODE).unwrap_err();
    assert!(matches!(err, FsError::AlreadyExists));
}

#[test]
fn mkdir_duplicate_name_fails() {
    let server = new_server();
    server.mkdir(ROOT, "dup_dir", DIR_MODE).unwrap();

    let err = server.mkdir(ROOT, "dup_dir", DIR_MODE).unwrap_err();
    assert!(matches!(err, FsError::AlreadyExists));
}

// ===========================================================================
// Unlink a directory should fail
// ===========================================================================

#[test]
fn unlink_directory_fails() {
    let server = new_server();
    server.mkdir(ROOT, "dir", DIR_MODE).unwrap();

    let err = server.unlink(ROOT, "dir").unwrap_err();
    assert!(matches!(err, FsError::IsADirectory));
}

// ===========================================================================
// Lookup
// ===========================================================================

#[test]
fn lookup_nonexistent_returns_not_found() {
    let server = new_server();
    let err = server.lookup(ROOT, "ghost").unwrap_err();
    assert!(matches!(err, FsError::NotFound));
}

#[test]
fn lookup_after_create() {
    let server = new_server();
    let created = server.create(ROOT, "found.txt", FILE_MODE).unwrap();
    let looked_up = server.lookup(ROOT, "found.txt").unwrap();
    assert_eq!(created.inode, looked_up.inode);
}

// ===========================================================================
// Rename operations
// ===========================================================================

#[test]
fn rename_same_directory() {
    let server = new_server();
    let attr = server.create(ROOT, "old.txt", FILE_MODE).unwrap();
    let inode = attr.inode;

    server.rename(ROOT, "old.txt", ROOT, "new.txt").unwrap();

    // Old name should not resolve
    assert!(server.lookup(ROOT, "old.txt").is_err());

    // New name should resolve to same inode
    let new_attr = server.lookup(ROOT, "new.txt").unwrap();
    assert_eq!(new_attr.inode, inode);
}

#[test]
fn rename_cross_directory() {
    let server = new_server();
    let dir_a = server.mkdir(ROOT, "dir_a", DIR_MODE).unwrap();
    let dir_b = server.mkdir(ROOT, "dir_b", DIR_MODE).unwrap();

    let file = server.create(dir_a.inode, "file.txt", FILE_MODE).unwrap();

    server
        .rename(dir_a.inode, "file.txt", dir_b.inode, "moved.txt")
        .unwrap();

    // Should not be in dir_a
    assert!(server.lookup(dir_a.inode, "file.txt").is_err());

    // Should be in dir_b
    let moved = server.lookup(dir_b.inode, "moved.txt").unwrap();
    assert_eq!(moved.inode, file.inode);
}

#[test]
fn rename_overwrite_file() {
    let server = new_server();
    server.create(ROOT, "src.txt", FILE_MODE).unwrap();
    let src = server.lookup(ROOT, "src.txt").unwrap();

    server.create(ROOT, "dst.txt", FILE_MODE).unwrap();

    server.rename(ROOT, "src.txt", ROOT, "dst.txt").unwrap();

    // dst.txt should now be the old src
    let dst = server.lookup(ROOT, "dst.txt").unwrap();
    assert_eq!(dst.inode, src.inode);

    // src.txt should be gone
    assert!(server.lookup(ROOT, "src.txt").is_err());

    // Only one entry in root
    let entries = server.readdir(ROOT).unwrap();
    assert_eq!(entries.len(), 1);
}

#[test]
fn rename_dir_cross_directory() {
    let server = new_server();
    let parent_a = server.mkdir(ROOT, "a", DIR_MODE).unwrap();
    let parent_b = server.mkdir(ROOT, "b", DIR_MODE).unwrap();
    let child = server.mkdir(parent_a.inode, "child", DIR_MODE).unwrap();

    server
        .rename(parent_a.inode, "child", parent_b.inode, "child_moved")
        .unwrap();

    // child should be in parent_b now
    let moved = server.lookup(parent_b.inode, "child_moved").unwrap();
    assert_eq!(moved.inode, child.inode);

    // parent_a nlink decreased (lost a ".." from child)
    let a = server.getattr(parent_a.inode).unwrap();
    assert_eq!(a.nlink, 2); // just "." and parent's link

    // parent_b nlink increased (gained a ".." from child)
    let b = server.getattr(parent_b.inode).unwrap();
    assert_eq!(b.nlink, 3);
}

// ===========================================================================
// Setattr
// ===========================================================================

#[test]
fn setattr_changes_mode() {
    let server = new_server();
    let attr = server.create(ROOT, "f.txt", FILE_MODE).unwrap();
    let inode = attr.inode;

    let mut new_attr = server.getattr(inode).unwrap();
    new_attr.mode = 0o100755;
    let updated = server.setattr(inode, new_attr).unwrap();
    assert_eq!(updated.mode, 0o100755);
}

// ===========================================================================
// Statfs
// ===========================================================================

#[test]
fn statfs_returns_reasonable_values() {
    let server = new_server();
    let st = server.statfs(ROOT).unwrap();
    assert!(st.blocks > 0);
    assert!(st.bsize > 0);
    assert!(st.namelen > 0);
}

// ===========================================================================
// Data integrity
// ===========================================================================

#[test]
fn write_read_large_block() {
    let server = new_server();
    let attr = server.create(ROOT, "big.bin", FILE_MODE).unwrap();
    let inode = attr.inode;

    // Write 64 KiB of pattern data
    let pattern: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
    let written = server.write(inode, 0, &pattern, 0).unwrap();
    assert_eq!(written, 65536);

    // Read back and verify
    let data = server.read(inode, 0, 65536).unwrap();
    assert_eq!(data, pattern);
}

#[test]
fn write_at_offset_preserves_earlier_data() {
    let server = new_server();
    let attr = server.create(ROOT, "sparse.bin", FILE_MODE).unwrap();
    let inode = attr.inode;

    server.write(inode, 0, b"AAAA", 0).unwrap();
    server.write(inode, 100, b"BBBB", 0).unwrap();

    let head = server.read(inode, 0, 4).unwrap();
    assert_eq!(&head, b"AAAA");

    let tail = server.read(inode, 100, 4).unwrap();
    assert_eq!(&tail, b"BBBB");

    // Between should be zeros
    let gap = server.read(inode, 4, 10).unwrap();
    assert!(gap.iter().all(|&b| b == 0));
}

#[test]
fn read_past_eof_returns_empty() {
    let server = new_server();
    let attr = server.create(ROOT, "tiny.txt", FILE_MODE).unwrap();
    let inode = attr.inode;

    server.write(inode, 0, b"hi", 0).unwrap();

    // Read starting past EOF
    let data = server.read(inode, 100, 10).unwrap();
    assert!(data.is_empty());
}

#[test]
fn flush_and_fsync() {
    let server = new_server();
    let attr = server.create(ROOT, "sync.txt", FILE_MODE).unwrap();
    let inode = attr.inode;

    server.write(inode, 0, b"data", 0).unwrap();
    server.flush(inode).unwrap();
    server.fsync(inode, false).unwrap();
    server.fsync(inode, true).unwrap();
}

// ===========================================================================
// Open checks
// ===========================================================================

#[test]
fn open_directory_fails() {
    let server = new_server();
    let dir = server.mkdir(ROOT, "dir", DIR_MODE).unwrap();
    let err = server.open(dir.inode, 0).unwrap_err();
    assert!(matches!(err, FsError::IsADirectory));
}

#[test]
fn open_nonexistent_fails() {
    let server = new_server();
    let err = server.open(9999, 0).unwrap_err();
    assert!(matches!(err, FsError::NotFound));
}

// ===========================================================================
// Nested directory operations
// ===========================================================================

#[test]
fn nested_directories_and_files() {
    let server = new_server();

    let d1 = server.mkdir(ROOT, "level1", DIR_MODE).unwrap();
    let d2 = server.mkdir(d1.inode, "level2", DIR_MODE).unwrap();
    let f = server.create(d2.inode, "deep.txt", FILE_MODE).unwrap();

    server.write(f.inode, 0, b"deep content", 0).unwrap();
    let data = server.read(f.inode, 0, 20).unwrap();
    assert_eq!(&data, b"deep content");

    // Lookup chain
    let l1 = server.lookup(ROOT, "level1").unwrap();
    assert_eq!(l1.inode, d1.inode);
    let l2 = server.lookup(d1.inode, "level2").unwrap();
    assert_eq!(l2.inode, d2.inode);
    let lf = server.lookup(d2.inode, "deep.txt").unwrap();
    assert_eq!(lf.inode, f.inode);
}

// ===========================================================================
// Concurrent safety
// ===========================================================================

#[test]
fn concurrent_create_unlink() {
    use std::thread;

    let server = Arc::new(new_server());
    let n = 100;
    let mut handles = vec![];

    // Create N files concurrently
    for i in 0..n {
        let s = Arc::clone(&server);
        handles.push(thread::spawn(move || {
            let name = format!("file_{}", i);
            s.create(ROOT, &name, FILE_MODE).unwrap();
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Verify all N files exist
    let entries = server.readdir(ROOT).unwrap();
    assert_eq!(entries.len(), n);

    // Unlink all concurrently
    let mut handles = vec![];
    for i in 0..n {
        let s = Arc::clone(&server);
        handles.push(thread::spawn(move || {
            let name = format!("file_{}", i);
            s.unlink(ROOT, &name).unwrap();
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Verify root is empty
    let entries = server.readdir(ROOT).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn concurrent_write_read_different_inodes() {
    use std::thread;

    let server = Arc::new(new_server());

    // Create 10 files
    let mut inodes = vec![];
    for i in 0..10 {
        let name = format!("cfile_{}", i);
        let attr = server.create(ROOT, &name, FILE_MODE).unwrap();
        inodes.push(attr.inode);
    }

    // Write to each concurrently
    let mut handles = vec![];
    for (idx, &inode) in inodes.iter().enumerate() {
        let s = Arc::clone(&server);
        handles.push(thread::spawn(move || {
            let data = format!("content for inode {}", idx);
            s.write(inode, 0, data.as_bytes(), 0).unwrap();
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Read and verify
    for (idx, &inode) in inodes.iter().enumerate() {
        let expected = format!("content for inode {}", idx);
        let data = server.read(inode, 0, expected.len() as u32).unwrap();
        assert_eq!(data, expected.as_bytes());
    }
}

#[test]
fn concurrent_mkdir_rmdir() {
    use std::thread;

    let server = Arc::new(new_server());
    let n = 50;

    // Create N directories concurrently
    let mut handles = vec![];
    for i in 0..n {
        let s = Arc::clone(&server);
        handles.push(thread::spawn(move || {
            let name = format!("dir_{}", i);
            s.mkdir(ROOT, &name, DIR_MODE).unwrap();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let entries = server.readdir(ROOT).unwrap();
    assert_eq!(entries.len(), n);

    // Remove all concurrently
    let mut handles = vec![];
    for i in 0..n {
        let s = Arc::clone(&server);
        handles.push(thread::spawn(move || {
            let name = format!("dir_{}", i);
            s.rmdir(ROOT, &name).unwrap();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let entries = server.readdir(ROOT).unwrap();
    assert!(entries.is_empty());

    // Root nlink should be back to 2
    let root_attr = server.getattr(ROOT).unwrap();
    assert_eq!(root_attr.nlink, 2);
}
