use rucksfs_core::{DirEntry, FileAttr, FsError, Inode, StatFs};
use serde::{Deserialize, Serialize};

/// RPC request types (one-to-one with ClientOps).
#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    Lookup { parent: Inode, name: String },
    Getattr { inode: Inode },
    Readdir { inode: Inode },
    Open { inode: Inode, flags: u32 },
    Read { inode: Inode, offset: u64, size: u32 },
    Write {
        inode: Inode,
        offset: u64,
        data: Vec<u8>,
        flags: u32,
    },
    Create { parent: Inode, name: String, mode: u32 },
    Mkdir { parent: Inode, name: String, mode: u32 },
    Unlink { parent: Inode, name: String },
    Rmdir { parent: Inode, name: String },
    Rename {
        parent: Inode,
        name: String,
        new_parent: Inode,
        new_name: String,
    },
    Setattr { inode: Inode, attr: FileAttr },
    Statfs { inode: Inode },
    Flush { inode: Inode },
    Fsync { inode: Inode, datasync: bool },
}

/// RPC response types.
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    OkFileAttr(FileAttr),
    OkDirEntries(Vec<DirEntry>),
    OkOpen(u64),
    OkRead(Vec<u8>),
    OkWrite(u32),
    OkStatFs(StatFs),
    OkUnit,
    Err(FsError),
}
