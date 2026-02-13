//! Binary encoding/decoding for inode metadata and KV keys.
//!
//! # Layout
//!
//! `InodeValue` is serialized as a version-tagged binary blob:
//!
//! ```text
//! [version: u8][inode: u64 BE][size: u64 BE][mode: u32 BE][nlink: u32 BE]
//! [uid: u32 BE][gid: u32 BE][atime: u64 BE][mtime: u64 BE][ctime: u64 BE]
//! ```
//!
//! Inode keys use big-endian u64 so that byte-order == numeric order.
//! Directory entry keys use `parent_inode(8 BE bytes) + child_name(UTF-8)`.

use rucksfs_core::{FileAttr, FsError, FsResult, Inode};

/// Current binary format version.
const FORMAT_VERSION: u8 = 1;

/// Expected serialized size (1 + 8 + 8 + 4 + 4 + 4 + 4 + 8 + 8 + 8 = 57 bytes).
const SERIALIZED_SIZE: usize = 57;

/// Versioned binary representation of inode metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InodeValue {
    pub version: u8,
    pub inode: Inode,
    pub size: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
}

impl InodeValue {
    /// Build an `InodeValue` from a [`FileAttr`] reference.
    pub fn from_attr(attr: &FileAttr) -> Self {
        Self {
            version: FORMAT_VERSION,
            inode: attr.inode,
            size: attr.size,
            mode: attr.mode,
            nlink: attr.nlink,
            uid: attr.uid,
            gid: attr.gid,
            atime: attr.atime,
            mtime: attr.mtime,
            ctime: attr.ctime,
        }
    }

    /// Convert back to a [`FileAttr`].
    pub fn to_attr(&self) -> FileAttr {
        FileAttr {
            inode: self.inode,
            size: self.size,
            mode: self.mode,
            nlink: self.nlink,
            uid: self.uid,
            gid: self.gid,
            atime: self.atime,
            mtime: self.mtime,
            ctime: self.ctime,
        }
    }

    /// Serialize to a compact binary blob.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(SERIALIZED_SIZE);
        buf.push(self.version);
        buf.extend_from_slice(&self.inode.to_be_bytes());
        buf.extend_from_slice(&self.size.to_be_bytes());
        buf.extend_from_slice(&self.mode.to_be_bytes());
        buf.extend_from_slice(&self.nlink.to_be_bytes());
        buf.extend_from_slice(&self.uid.to_be_bytes());
        buf.extend_from_slice(&self.gid.to_be_bytes());
        buf.extend_from_slice(&self.atime.to_be_bytes());
        buf.extend_from_slice(&self.mtime.to_be_bytes());
        buf.extend_from_slice(&self.ctime.to_be_bytes());
        debug_assert_eq!(buf.len(), SERIALIZED_SIZE);
        buf
    }

    /// Deserialize from a binary blob.
    ///
    /// Returns `FsError::InvalidInput` when the data is too short or
    /// the version tag is unsupported.
    pub fn deserialize(data: &[u8]) -> FsResult<Self> {
        if data.len() < SERIALIZED_SIZE {
            return Err(FsError::InvalidInput(format!(
                "InodeValue: expected {} bytes, got {}",
                SERIALIZED_SIZE,
                data.len()
            )));
        }

        let version = data[0];
        if version != FORMAT_VERSION {
            return Err(FsError::InvalidInput(format!(
                "InodeValue: unsupported version {}",
                version
            )));
        }

        let inode = u64::from_be_bytes(data[1..9].try_into().unwrap());
        let size = u64::from_be_bytes(data[9..17].try_into().unwrap());
        let mode = u32::from_be_bytes(data[17..21].try_into().unwrap());
        let nlink = u32::from_be_bytes(data[21..25].try_into().unwrap());
        let uid = u32::from_be_bytes(data[25..29].try_into().unwrap());
        let gid = u32::from_be_bytes(data[29..33].try_into().unwrap());
        let atime = u64::from_be_bytes(data[33..41].try_into().unwrap());
        let mtime = u64::from_be_bytes(data[41..49].try_into().unwrap());
        let ctime = u64::from_be_bytes(data[49..57].try_into().unwrap());

        Ok(Self {
            version,
            inode,
            size,
            mode,
            nlink,
            uid,
            gid,
            atime,
            mtime,
            ctime,
        })
    }
}

// ---------------------------------------------------------------------------
// Key encoding helpers
// ---------------------------------------------------------------------------

/// Prefix byte for inode metadata keys.
const INODE_KEY_PREFIX: u8 = b'I';

/// Prefix byte for directory entry keys.
const DIR_ENTRY_KEY_PREFIX: u8 = b'D';

/// Encode an inode metadata key: `[b'I'][inode: u64 BE]`.
pub fn encode_inode_key(inode: Inode) -> Vec<u8> {
    let mut key = Vec::with_capacity(9);
    key.push(INODE_KEY_PREFIX);
    key.extend_from_slice(&inode.to_be_bytes());
    key
}

/// Encode a directory entry key: `[b'D'][parent: u64 BE][name: UTF-8]`.
pub fn encode_dir_entry_key(parent: Inode, name: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(9 + name.len());
    key.push(DIR_ENTRY_KEY_PREFIX);
    key.extend_from_slice(&parent.to_be_bytes());
    key.extend_from_slice(name.as_bytes());
    key
}

/// Build the scan prefix for all directory entries under `parent`:
/// `[b'D'][parent: u64 BE]`.
pub fn dir_entry_prefix(parent: Inode) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(9);
    prefix.push(DIR_ENTRY_KEY_PREFIX);
    prefix.extend_from_slice(&parent.to_be_bytes());
    prefix
}

/// Extract the child name from a directory entry key (strip prefix + parent bytes).
pub fn extract_child_name(key: &[u8]) -> FsResult<&str> {
    if key.len() < 9 || key[0] != DIR_ENTRY_KEY_PREFIX {
        return Err(FsError::InvalidInput(
            "invalid dir entry key".to_string(),
        ));
    }
    std::str::from_utf8(&key[9..])
        .map_err(|e| FsError::InvalidInput(format!("invalid UTF-8 in dir key: {}", e)))
}

// ---------------------------------------------------------------------------
// Delta key encoding helpers
// ---------------------------------------------------------------------------

/// Prefix byte for delta entry keys.
const DELTA_KEY_PREFIX: u8 = b'X';

/// Encode a delta entry key: `[b'X'][inode: u64 BE][seq: u64 BE]`.
///
/// Total length: 17 bytes.  Keys with the same inode are ordered by `seq`
/// when compared as byte strings (big-endian guarantees this).
pub fn encode_delta_key(inode: Inode, seq: u64) -> [u8; 17] {
    let mut key = [0u8; 17];
    key[0] = DELTA_KEY_PREFIX;
    key[1..9].copy_from_slice(&inode.to_be_bytes());
    key[9..17].copy_from_slice(&seq.to_be_bytes());
    key
}

/// Decode a delta entry key back into `(inode, seq)`.
///
/// Returns `FsError::InvalidInput` if `key` is shorter than 17 bytes or has
/// the wrong prefix.
pub fn decode_delta_key(key: &[u8]) -> FsResult<(Inode, u64)> {
    if key.len() < 17 || key[0] != DELTA_KEY_PREFIX {
        return Err(FsError::InvalidInput(
            "invalid delta key".to_string(),
        ));
    }
    let inode = u64::from_be_bytes(key[1..9].try_into().unwrap());
    let seq = u64::from_be_bytes(key[9..17].try_into().unwrap());
    Ok((inode, seq))
}

/// Build the scan prefix for all delta entries of a given `inode`:
/// `[b'X'][inode: u64 BE]`.
pub fn delta_prefix(inode: Inode) -> [u8; 9] {
    let mut prefix = [0u8; 9];
    prefix[0] = DELTA_KEY_PREFIX;
    prefix[1..9].copy_from_slice(&inode.to_be_bytes());
    prefix
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_attr() -> FileAttr {
        FileAttr {
            inode: 42,
            size: 1024,
            mode: 0o100644,
            nlink: 1,
            uid: 1000,
            gid: 1000,
            atime: 1_700_000_000,
            mtime: 1_700_000_001,
            ctime: 1_700_000_002,
        }
    }

    #[test]
    fn roundtrip_serialize_deserialize() {
        let attr = sample_attr();
        let iv = InodeValue::from_attr(&attr);
        let bytes = iv.serialize();
        assert_eq!(bytes.len(), SERIALIZED_SIZE);
        let restored = InodeValue::deserialize(&bytes).unwrap();
        assert_eq!(iv, restored);
        assert_eq!(restored.to_attr(), attr);
    }

    #[test]
    fn deserialize_too_short() {
        let result = InodeValue::deserialize(&[1u8; 10]);
        assert!(result.is_err());
        match result.unwrap_err() {
            FsError::InvalidInput(msg) => assert!(msg.contains("expected")),
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn deserialize_bad_version() {
        let mut bytes = InodeValue::from_attr(&sample_attr()).serialize();
        bytes[0] = 99; // bad version
        let result = InodeValue::deserialize(&bytes);
        assert!(result.is_err());
        match result.unwrap_err() {
            FsError::InvalidInput(msg) => assert!(msg.contains("unsupported version")),
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn deserialize_empty() {
        let result = InodeValue::deserialize(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn inode_key_encoding() {
        let key = encode_inode_key(42);
        assert_eq!(key.len(), 9);
        assert_eq!(key[0], b'I');
        let inode = u64::from_be_bytes(key[1..9].try_into().unwrap());
        assert_eq!(inode, 42);
    }

    #[test]
    fn inode_key_byte_order() {
        let k1 = encode_inode_key(1);
        let k2 = encode_inode_key(256);
        let k3 = encode_inode_key(u64::MAX);
        // Byte-order should match numeric order
        assert!(k1 < k2);
        assert!(k2 < k3);
    }

    #[test]
    fn dir_entry_key_encoding() {
        let key = encode_dir_entry_key(1, "hello.txt");
        assert_eq!(key[0], b'D');
        let parent = u64::from_be_bytes(key[1..9].try_into().unwrap());
        assert_eq!(parent, 1);
        let name = std::str::from_utf8(&key[9..]).unwrap();
        assert_eq!(name, "hello.txt");
    }

    #[test]
    fn dir_entry_key_ordering() {
        // Same parent, different names — sorted lexicographically
        let k_a = encode_dir_entry_key(1, "aaa");
        let k_b = encode_dir_entry_key(1, "bbb");
        assert!(k_a < k_b);

        // Different parents — smaller parent first
        let k_p1 = encode_dir_entry_key(1, "z");
        let k_p2 = encode_dir_entry_key(2, "a");
        assert!(k_p1 < k_p2);
    }

    #[test]
    fn dir_entry_prefix_and_extract() {
        let prefix = dir_entry_prefix(100);
        let key = encode_dir_entry_key(100, "file.rs");
        assert!(key.starts_with(&prefix));

        let name = extract_child_name(&key).unwrap();
        assert_eq!(name, "file.rs");
    }

    #[test]
    fn extract_child_name_bad_key() {
        assert!(extract_child_name(&[]).is_err());
        assert!(extract_child_name(&[b'X'; 9]).is_err());
    }

    // -- delta key tests ----------------------------------------------------

    #[test]
    fn delta_key_roundtrip() {
        let key = encode_delta_key(42, 7);
        assert_eq!(key.len(), 17);
        assert_eq!(key[0], b'X');
        let (inode, seq) = decode_delta_key(&key).unwrap();
        assert_eq!(inode, 42);
        assert_eq!(seq, 7);
    }

    #[test]
    fn delta_key_same_inode_ordered_by_seq() {
        let k1 = encode_delta_key(100, 0);
        let k2 = encode_delta_key(100, 1);
        let k3 = encode_delta_key(100, 255);
        let k4 = encode_delta_key(100, u64::MAX);
        assert!(k1 < k2);
        assert!(k2 < k3);
        assert!(k3 < k4);
    }

    #[test]
    fn delta_key_different_inodes_ordered_by_inode() {
        let k1 = encode_delta_key(1, u64::MAX);
        let k2 = encode_delta_key(2, 0);
        assert!(k1 < k2);
    }

    #[test]
    fn delta_prefix_matches_keys() {
        let prefix = delta_prefix(42);
        let k1 = encode_delta_key(42, 0);
        let k2 = encode_delta_key(42, 100);
        let k_other = encode_delta_key(43, 0);
        assert!(k1.starts_with(&prefix));
        assert!(k2.starts_with(&prefix));
        assert!(!k_other.starts_with(&prefix));
    }

    #[test]
    fn decode_delta_key_bad_input() {
        assert!(decode_delta_key(&[]).is_err());
        assert!(decode_delta_key(&[b'X'; 10]).is_err()); // too short
        assert!(decode_delta_key(&[b'I'; 17]).is_err()); // wrong prefix
    }
}
