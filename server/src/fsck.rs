//! Filesystem consistency checker (fsck).
//!
//! Scans RocksDB column families to detect:
//! - Orphan inodes (no directory entry references, except root)
//! - nlink mismatches (stored nlink != actual reference count)
//! - next_inode counter inconsistencies

use std::collections::HashMap;

use rucksfs_core::{FsResult, Inode};
use rucksfs_storage::allocator::ROOT_INODE;
use rucksfs_storage::encoding::InodeValue;
use rucksfs_storage::{DirectoryIndex, MetadataStore};

/// Prefix byte for inode metadata keys (must match encoding.rs).
const INODE_KEY_PREFIX: u8 = b'I';

/// Well-known key used to persist the next-inode counter (must match allocator.rs).
const NEXT_INODE_KEY: &[u8] = b"next_inode";

/// Result of a single consistency check.
#[derive(Debug, Clone)]
pub struct FsckIssue {
    pub inode: Inode,
    pub kind: FsckIssueKind,
    pub detail: String,
}

/// Categories of consistency issues.
#[derive(Debug, Clone, PartialEq)]
pub enum FsckIssueKind {
    OrphanInode,
    NlinkMismatch,
    CounterInconsistency,
}

/// Overall fsck report.
#[derive(Debug)]
pub struct FsckReport {
    pub issues: Vec<FsckIssue>,
    pub total_inodes: usize,
    pub total_dir_entries: usize,
}

impl FsckReport {
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn print_summary(&self) {
        println!(
            "fsck: scanned {} inodes, {} directory entries",
            self.total_inodes, self.total_dir_entries
        );
        if self.is_clean() {
            println!("fsck: filesystem is clean");
        } else {
            println!("fsck: found {} issues:", self.issues.len());
            for issue in &self.issues {
                println!(
                    "  inode {}: {:?} — {}",
                    issue.inode, issue.kind, issue.detail
                );
            }
        }
    }
}

/// Run fsck on the given metadata store and directory index.
///
/// This performs a read-only scan. No data is modified.
pub fn check<M: MetadataStore, I: DirectoryIndex>(
    metadata: &M,
    index: &I,
) -> FsResult<FsckReport> {
    let mut issues = Vec::new();
    let mut all_inodes: HashMap<Inode, InodeValue> = HashMap::new();
    let mut referenced_inodes: HashMap<Inode, u32> = HashMap::new();

    // Phase 1: Collect all inodes using scan_prefix on the inode key prefix.
    let mut max_inode: Inode = 0;
    let inode_prefix = [INODE_KEY_PREFIX];
    let entries = metadata.scan_prefix(&inode_prefix)?;
    for (key, value) in &entries {
        // Inode keys are 9 bytes: [b'I'][inode: u64 BE]
        if key.len() == 9 && key[0] == INODE_KEY_PREFIX {
            let ino = u64::from_be_bytes(key[1..9].try_into().unwrap());
            if let Ok(iv) = InodeValue::deserialize(value) {
                all_inodes.insert(ino, iv);
                if ino > max_inode {
                    max_inode = ino;
                }
            }
        }
    }

    // Phase 2: Collect directory entry references by listing children of
    // every directory inode.
    let dir_inodes: Vec<Inode> = all_inodes
        .iter()
        .filter(|(_, iv)| iv.mode & 0o170000 == 0o040000)
        .map(|(&ino, _)| ino)
        .collect();

    let mut total_dir_entries = 0;
    for &dir_ino in &dir_inodes {
        if let Ok(children) = index.list_dir(dir_ino) {
            for child in &children {
                total_dir_entries += 1;
                *referenced_inodes.entry(child.inode).or_insert(0) += 1;
            }
        }
    }

    // Phase 3: Check for orphan inodes (no directory entry points to them).
    // Root inode is exempt since nothing references it via a parent entry.
    for &ino in all_inodes.keys() {
        if ino == ROOT_INODE {
            continue;
        }
        if !referenced_inodes.contains_key(&ino) {
            issues.push(FsckIssue {
                inode: ino,
                kind: FsckIssueKind::OrphanInode,
                detail: format!("inode {} has no directory entry references", ino),
            });
        }
    }

    // Phase 4: Check nlink consistency for regular files.
    // Directories are skipped because their nlink is 2 + number of
    // subdirectories, which requires deeper analysis via deltas.
    for (&ino, iv) in &all_inodes {
        if iv.mode & 0o170000 == 0o040000 {
            // Skip directories.
            continue;
        }
        let expected_refs = referenced_inodes.get(&ino).copied().unwrap_or(0);
        if iv.nlink != expected_refs {
            issues.push(FsckIssue {
                inode: ino,
                kind: FsckIssueKind::NlinkMismatch,
                detail: format!(
                    "inode {} has nlink={} but {} directory references",
                    ino, iv.nlink, expected_refs
                ),
            });
        }
    }

    // Phase 5: Check next_inode counter.
    // The allocator persists its counter under the key b"next_inode"
    // in the same MetadataStore (see allocator.rs).
    if let Ok(Some(raw)) = metadata.get(NEXT_INODE_KEY) {
        if raw.len() == 8 {
            let stored_next = u64::from_be_bytes(raw[..8].try_into().unwrap());
            if stored_next <= max_inode {
                issues.push(FsckIssue {
                    inode: 0,
                    kind: FsckIssueKind::CounterInconsistency,
                    detail: format!(
                        "next_inode counter ({}) <= max existing inode ({})",
                        stored_next, max_inode
                    ),
                });
            }
        }
    }

    Ok(FsckReport {
        issues,
        total_inodes: all_inodes.len(),
        total_dir_entries,
    })
}
