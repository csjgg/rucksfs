//! Delta operation types, encoding, and fold logic.
//!
//! A `DeltaOp` represents an incremental modification to an inode's attributes
//! (e.g. nlink change, timestamp update).  Instead of doing read-modify-write
//! on the base inode, callers **append** deltas and the system folds them on
//! read or during background compaction.

use rucksfs_core::{FsError, FsResult};
use rucksfs_storage::encoding::InodeValue;

// ---------------------------------------------------------------------------
// DeltaOp enum
// ---------------------------------------------------------------------------

/// Op-type tags used in binary encoding (single byte).
const OP_INCREMENT_NLINK: u8 = 1;
const OP_SET_MTIME: u8 = 2;
const OP_SET_CTIME: u8 = 3;
const OP_SET_ATIME: u8 = 4;

/// An incremental modification to an inode's attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOp {
    /// Increment (or decrement) the `nlink` field by the given signed amount.
    IncrementNlink(i32),
    /// Set `mtime` to the given timestamp (fold takes max).
    SetMtime(u64),
    /// Set `ctime` to the given timestamp (fold takes max).
    SetCtime(u64),
    /// Set `atime` to the given timestamp (fold takes max).
    SetAtime(u64),
}

// ---------------------------------------------------------------------------
// Serialization / Deserialization
// ---------------------------------------------------------------------------

impl DeltaOp {
    /// Serialize to a compact binary blob: `[op_type: u8][payload: BE]`.
    ///
    /// - `IncrementNlink(i32)` → 1 + 4 = 5 bytes
    /// - `Set{M,C,A}time(u64)` → 1 + 8 = 9 bytes
    pub fn serialize(&self) -> Vec<u8> {
        match self {
            DeltaOp::IncrementNlink(n) => {
                let mut buf = Vec::with_capacity(5);
                buf.push(OP_INCREMENT_NLINK);
                buf.extend_from_slice(&n.to_be_bytes());
                buf
            }
            DeltaOp::SetMtime(t) => {
                let mut buf = Vec::with_capacity(9);
                buf.push(OP_SET_MTIME);
                buf.extend_from_slice(&t.to_be_bytes());
                buf
            }
            DeltaOp::SetCtime(t) => {
                let mut buf = Vec::with_capacity(9);
                buf.push(OP_SET_CTIME);
                buf.extend_from_slice(&t.to_be_bytes());
                buf
            }
            DeltaOp::SetAtime(t) => {
                let mut buf = Vec::with_capacity(9);
                buf.push(OP_SET_ATIME);
                buf.extend_from_slice(&t.to_be_bytes());
                buf
            }
        }
    }

    /// Deserialize from a binary blob produced by [`DeltaOp::serialize`].
    pub fn deserialize(data: &[u8]) -> FsResult<Self> {
        if data.is_empty() {
            return Err(FsError::InvalidInput(
                "DeltaOp: empty data".to_string(),
            ));
        }
        let op_type = data[0];
        let payload = &data[1..];

        match op_type {
            OP_INCREMENT_NLINK => {
                if payload.len() < 4 {
                    return Err(FsError::InvalidInput(format!(
                        "DeltaOp::IncrementNlink: expected 4 payload bytes, got {}",
                        payload.len()
                    )));
                }
                let n = i32::from_be_bytes(payload[..4].try_into().unwrap());
                Ok(DeltaOp::IncrementNlink(n))
            }
            OP_SET_MTIME => {
                if payload.len() < 8 {
                    return Err(FsError::InvalidInput(format!(
                        "DeltaOp::SetMtime: expected 8 payload bytes, got {}",
                        payload.len()
                    )));
                }
                let t = u64::from_be_bytes(payload[..8].try_into().unwrap());
                Ok(DeltaOp::SetMtime(t))
            }
            OP_SET_CTIME => {
                if payload.len() < 8 {
                    return Err(FsError::InvalidInput(format!(
                        "DeltaOp::SetCtime: expected 8 payload bytes, got {}",
                        payload.len()
                    )));
                }
                let t = u64::from_be_bytes(payload[..8].try_into().unwrap());
                Ok(DeltaOp::SetCtime(t))
            }
            OP_SET_ATIME => {
                if payload.len() < 8 {
                    return Err(FsError::InvalidInput(format!(
                        "DeltaOp::SetAtime: expected 8 payload bytes, got {}",
                        payload.len()
                    )));
                }
                let t = u64::from_be_bytes(payload[..8].try_into().unwrap());
                Ok(DeltaOp::SetAtime(t))
            }
            _ => Err(FsError::InvalidInput(format!(
                "DeltaOp: unknown op_type {}",
                op_type
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Fold logic
// ---------------------------------------------------------------------------

/// Apply a sequence of deltas to a base `InodeValue` **in place**.
///
/// Semantics:
/// - `IncrementNlink(n)` → `base.nlink = (base.nlink as i64 + n as i64) as u32`
/// - `SetMtime(t)` → `base.mtime = max(base.mtime, t)`
/// - `SetCtime(t)` → `base.ctime = max(base.ctime, t)`
/// - `SetAtime(t)` → `base.atime = max(base.atime, t)`
pub fn fold_deltas(base: &mut InodeValue, deltas: &[DeltaOp]) {
    for delta in deltas {
        match delta {
            DeltaOp::IncrementNlink(n) => {
                // Use i64 arithmetic to avoid underflow panic, then clamp to u32.
                let new_val = (base.nlink as i64) + (*n as i64);
                base.nlink = new_val.max(0) as u32;
            }
            DeltaOp::SetMtime(t) => {
                base.mtime = base.mtime.max(*t);
            }
            DeltaOp::SetCtime(t) => {
                base.ctime = base.ctime.max(*t);
            }
            DeltaOp::SetAtime(t) => {
                base.atime = base.atime.max(*t);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_base() -> InodeValue {
        InodeValue {
            version: 1,
            inode: 42,
            size: 0,
            mode: 0o040755,
            nlink: 2,
            uid: 0,
            gid: 0,
            atime: 1000,
            mtime: 1000,
            ctime: 1000,
        }
    }

    // -- round-trip tests ---------------------------------------------------

    #[test]
    fn roundtrip_increment_nlink_positive() {
        let op = DeltaOp::IncrementNlink(3);
        let bytes = op.serialize();
        let restored = DeltaOp::deserialize(&bytes).unwrap();
        assert_eq!(op, restored);
    }

    #[test]
    fn roundtrip_increment_nlink_negative() {
        let op = DeltaOp::IncrementNlink(-1);
        let bytes = op.serialize();
        let restored = DeltaOp::deserialize(&bytes).unwrap();
        assert_eq!(op, restored);
    }

    #[test]
    fn roundtrip_increment_nlink_zero() {
        let op = DeltaOp::IncrementNlink(0);
        let bytes = op.serialize();
        let restored = DeltaOp::deserialize(&bytes).unwrap();
        assert_eq!(op, restored);
    }

    #[test]
    fn roundtrip_set_mtime() {
        let op = DeltaOp::SetMtime(1_700_000_000);
        let bytes = op.serialize();
        let restored = DeltaOp::deserialize(&bytes).unwrap();
        assert_eq!(op, restored);
    }

    #[test]
    fn roundtrip_set_ctime() {
        let op = DeltaOp::SetCtime(u64::MAX);
        let bytes = op.serialize();
        let restored = DeltaOp::deserialize(&bytes).unwrap();
        assert_eq!(op, restored);
    }

    #[test]
    fn roundtrip_set_atime() {
        let op = DeltaOp::SetAtime(0);
        let bytes = op.serialize();
        let restored = DeltaOp::deserialize(&bytes).unwrap();
        assert_eq!(op, restored);
    }

    #[test]
    fn deserialize_empty_data() {
        assert!(DeltaOp::deserialize(&[]).is_err());
    }

    #[test]
    fn deserialize_unknown_op_type() {
        assert!(DeltaOp::deserialize(&[255]).is_err());
    }

    #[test]
    fn deserialize_truncated_payload() {
        // IncrementNlink needs 4 bytes of payload, give it 2
        assert!(DeltaOp::deserialize(&[OP_INCREMENT_NLINK, 0, 0]).is_err());
        // SetMtime needs 8 bytes of payload, give it 4
        assert!(DeltaOp::deserialize(&[OP_SET_MTIME, 0, 0, 0, 0]).is_err());
    }

    // -- fold tests ---------------------------------------------------------

    #[test]
    fn fold_empty_deltas() {
        let mut base = sample_base();
        let original = base.clone();
        fold_deltas(&mut base, &[]);
        assert_eq!(base, original);
    }

    #[test]
    fn fold_single_increment_nlink() {
        let mut base = sample_base();
        fold_deltas(&mut base, &[DeltaOp::IncrementNlink(1)]);
        assert_eq!(base.nlink, 3);
    }

    #[test]
    fn fold_single_decrement_nlink() {
        let mut base = sample_base();
        fold_deltas(&mut base, &[DeltaOp::IncrementNlink(-1)]);
        assert_eq!(base.nlink, 1);
    }

    #[test]
    fn fold_nlink_clamps_to_zero() {
        let mut base = sample_base(); // nlink = 2
        fold_deltas(&mut base, &[DeltaOp::IncrementNlink(-100)]);
        assert_eq!(base.nlink, 0);
    }

    #[test]
    fn fold_set_mtime_takes_max() {
        let mut base = sample_base(); // mtime = 1000
        fold_deltas(
            &mut base,
            &[
                DeltaOp::SetMtime(500),  // earlier — should be ignored
                DeltaOp::SetMtime(2000), // later — should win
                DeltaOp::SetMtime(1500), // in between — should be ignored
            ],
        );
        assert_eq!(base.mtime, 2000);
    }

    #[test]
    fn fold_set_ctime_takes_max() {
        let mut base = sample_base(); // ctime = 1000
        fold_deltas(&mut base, &[DeltaOp::SetCtime(3000)]);
        assert_eq!(base.ctime, 3000);
    }

    #[test]
    fn fold_set_atime_takes_max() {
        let mut base = sample_base(); // atime = 1000
        fold_deltas(&mut base, &[DeltaOp::SetAtime(999)]);
        // 999 < 1000, so atime stays 1000
        assert_eq!(base.atime, 1000);
    }

    #[test]
    fn fold_mixed_deltas() {
        let mut base = sample_base(); // nlink=2, mtime=1000, ctime=1000
        fold_deltas(
            &mut base,
            &[
                DeltaOp::IncrementNlink(1),
                DeltaOp::SetMtime(2000),
                DeltaOp::SetCtime(2000),
                DeltaOp::IncrementNlink(1),
                DeltaOp::SetMtime(3000),
                DeltaOp::SetCtime(3000),
            ],
        );
        assert_eq!(base.nlink, 4);
        assert_eq!(base.mtime, 3000);
        assert_eq!(base.ctime, 3000);
        // atime untouched
        assert_eq!(base.atime, 1000);
    }

    #[test]
    fn fold_does_not_touch_other_fields() {
        let mut base = sample_base();
        fold_deltas(
            &mut base,
            &[DeltaOp::IncrementNlink(5), DeltaOp::SetMtime(9999)],
        );
        // These fields must remain unchanged
        assert_eq!(base.version, 1);
        assert_eq!(base.inode, 42);
        assert_eq!(base.size, 0);
        assert_eq!(base.mode, 0o040755);
        assert_eq!(base.uid, 0);
        assert_eq!(base.gid, 0);
    }
}
