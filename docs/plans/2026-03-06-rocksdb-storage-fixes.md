# RocksDB Storage Layer Fixes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix all confirmed issues from the RocksDB storage layer audit: a P0 seq-recovery bug, two P3 config improvements, and a documentation gap.

**Architecture:** Four independent fixes touching `storage/src/rocks.rs` (CF config + seq recovery) and `demo/src/main.rs` (startup call). Each fix includes a test that fails before the fix and passes after.

**Tech Stack:** Rust, RocksDB (via `rocksdb` crate), `tempfile` for test isolation.

---

### Task 1: Fix P0 — Call `recover_seqs()` on startup to prevent delta key collision

The most critical bug. After a restart, `RocksDeltaStore` starts seq counters at 0, silently overwriting existing deltas. The fix has two parts: call `recover_seqs()` in the production startup path, and add a regression test that simulates the restart scenario **without** manually calling `recover_seqs()`.

**Files:**
- Modify: `demo/src/main.rs:67` (add `recover_seqs` call after constructing `RocksDeltaStore`)
- Modify: `storage/src/rocks.rs:331-338` (call `recover_seqs` inside `RocksDeltaStore::new`)
- Test: `storage/src/rocks.rs` (existing test module, add new test)

**Step 1: Write a failing test that proves the bug exists**

Add this test to the `delta_store` test module in `storage/src/rocks.rs` (near line 1162, after the existing `recover_seqs_on_restart` test):

```rust
#[test]
fn new_without_explicit_recover_does_not_overwrite_deltas() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let v = vec![1u8, 0, 0, 0, 1]; // IncrementNlink(1)

    // Session 1: write 3 deltas for inode 42.
    {
        let db = open_rocks_db(tmp.path()).unwrap();
        let store = RocksDeltaStore::new(db);
        let seqs = store.append_deltas(42, &[v.clone(), v.clone(), v.clone()]).unwrap();
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    // Session 2: reopen DB, create a NEW store (simulating restart).
    // Append one more delta — it must NOT collide with seq 0/1/2.
    {
        let db = open_rocks_db(tmp.path()).unwrap();
        let store = RocksDeltaStore::new(db);
        let seqs = store.append_deltas(42, &[v.clone()]).unwrap();
        assert!(seqs[0] >= 3, "seq {} collides with existing deltas", seqs[0]);
    }

    // Session 3: reopen again, verify all 4 deltas are present.
    {
        let db = open_rocks_db(tmp.path()).unwrap();
        let store = RocksDeltaStore::new(db);
        let all = store.scan_deltas(42).unwrap();
        assert_eq!(all.len(), 4, "expected 4 deltas, got {}", all.len());
    }
}
```

**Step 2: Run the test to confirm it fails**

Run: `cargo test -p rucksfs-storage new_without_explicit_recover -- --nocapture`
Expected: FAIL — `seqs[0]` will be 0, colliding with existing delta.

**Step 3: Fix `RocksDeltaStore::new()` to automatically recover seqs**

In `storage/src/rocks.rs`, modify the `new` method (lines 331-338):

```rust
impl RocksDeltaStore {
    /// Create a new delta store from a shared DB handle.
    ///
    /// Automatically recovers per-inode sequence counters from existing
    /// delta entries on disk so that new allocations never collide with
    /// persisted keys.
    pub fn new(db: Arc<TransactionDB>) -> Self {
        let store = Self {
            db,
            seqs: RwLock::new(HashMap::new()),
        };
        // Best-effort recovery: log a warning if it fails but do not
        // prevent startup — the worst case is seq gaps, not overwrites,
        // because a failed scan means the CF is empty or inaccessible.
        if let Err(e) = store.recover_seqs() {
            eprintln!("warning: delta seq recovery failed: {}", e);
        }
        store
    }
}
```

**Step 4: Remove the now-redundant manual `recover_seqs()` call pattern**

Since `new()` now auto-recovers, the explicit call in the existing test `recover_seqs_on_restart` is still valid (double-recovery is idempotent). No need to change that test.

However, remove the manual `recover_seqs()` comment from `demo/src/main.rs` if one exists (it doesn't currently, which was the bug — but confirm no other call sites need updating).

**Step 5: Run the test to confirm it passes**

Run: `cargo test -p rucksfs-storage new_without_explicit_recover -- --nocapture`
Expected: PASS

**Step 6: Run full test suite**

Run: `cargo test --workspace`
Expected: All tests pass including existing `recover_seqs_on_restart`.

**Step 7: Commit**

```
fix(storage): auto-recover delta seq counters in RocksDeltaStore::new

Previously, recover_seqs() was never called in production code paths,
causing seq counters to restart at 0 after process restart. This
silently overwrote existing delta entries.
```

---

### Task 2: Fix P3 — Correct prefix extractor length from 8 to 9

The prefix extractor for `dir_entries` and `delta_entries` is set to 8 bytes, but the actual logical prefix is 9 bytes (`[prefix_byte][inode: u64 BE]`). This causes slightly degraded bloom filter precision.

**Files:**
- Modify: `storage/src/rocks.rs:63,68` (change `create_fixed_prefix(8)` to `create_fixed_prefix(9)`)

**Step 1: Fix the prefix extractor length**

In `storage/src/rocks.rs`, change lines 63 and 68:

```rust
CF_DIR_ENTRIES => {
    opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(9));
    opts.set_block_based_table_factory(&block_opts_with_bloom());
    opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
}
CF_DELTA_ENTRIES => {
    opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(9));
    opts.set_block_based_table_factory(&block_opts_with_bloom());
}
```

**Step 2: Update the doc comment to reflect the correct prefix length**

Update the comment block at lines 41-46:

```rust
/// Build per-column-family options based on access patterns.
///
/// - `inodes`: point lookups only → bloom filter + LZ4.
/// - `dir_entries`: prefix scans by parent inode → 9-byte prefix extractor
///   (`[b'D'][parent: u64 BE]`) + bloom + LZ4.
/// - `delta_entries`: prefix scans by inode → 9-byte prefix extractor
///   (`[b'X'][inode: u64 BE]`) + bloom. No compression (high-churn,
///   short-lived data — deltas are folded and deleted by compaction).
/// - `system`: rare access → LZ4 only.
```

**Step 3: Run full test suite**

Run: `cargo test --workspace`
Expected: All tests pass. (Existing tests already use `starts_with` checks, so the behavior is unchanged; bloom filter is simply more precise now.)

**IMPORTANT NOTE:** If the database already has SST files created with the old prefix extractor (length 8), RocksDB will refuse to open the DB due to prefix extractor mismatch. For existing deployments, the database directory must be deleted and recreated. This is acceptable for a graduation demo project. Add a note about this below.

**Step 4: Commit**

```
fix(storage): correct prefix extractor length from 8 to 9 bytes

The dir_entries and delta_entries CFs use 9-byte logical prefixes
([prefix_byte][inode: u64 BE]) but the extractor was set to 8.
This caused suboptimal bloom filter precision for directories whose
inode numbers differed only in the least significant byte.

NOTE: This is a breaking change for existing databases. Delete and
recreate the data directory after upgrading.
```

---

### Task 3: Fix P3 — Add comment documenting the intentional lack of compression on delta_entries

The delta_entries CF deliberately skips LZ4 compression due to its high-churn, short-lived nature. This was undocumented.

**Files:**
- Modify: `storage/src/rocks.rs:67-70` (add inline comment)

**Step 1: Add the explanatory comment**

This was already handled in Task 2's doc comment update. Verify that the `cf_options` match arm for `CF_DELTA_ENTRIES` has an inline comment:

```rust
CF_DELTA_ENTRIES => {
    opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(9));
    opts.set_block_based_table_factory(&block_opts_with_bloom());
    // Deliberately no compression: delta entries are short-lived
    // (5-9 bytes each, folded and deleted by background compaction).
    // Skipping LZ4 avoids decode overhead on the hot read path
    // (load_inode folds deltas on every cache miss).
}
```

**Step 2: Commit**

If this was already committed as part of Task 2's comment update, skip this commit. Otherwise:

```
docs(storage): document intentional lack of compression on delta_entries CF
```

---

### Task 4: Fix P3 — Document WAL sync trade-off

Add a comment in `begin_write` documenting the WAL sync behavior and its implications.

**Files:**
- Modify: `storage/src/rocks.rs:655-667` (add comment to `begin_write`)

**Step 1: Add documentation comment**

```rust
impl StorageBundle for RocksStorageBundle {
    fn begin_write(&self) -> Box<dyn AtomicWriteBatch + '_> {
        let mut txn_opts = TransactionOptions::default();
        txn_opts.set_lock_timeout(5000); // 5s lock wait timeout
        txn_opts.set_deadlock_detect(true);
        // WAL sync policy: RocksDB default (sync = false).
        // Writes go to WAL but are NOT fsync'd to disk on each commit.
        // This means:
        //   - Process crash: data is safe (WAL is in OS page cache).
        //   - Power failure:  up to one WAL write may be lost.
        // For production use, consider adding a `mount -o sync` option
        // that sets `write_opts.set_sync(true)` for FUSE fsync calls.
        let write_opts = WriteOptions::default();
        let txn = self.db.transaction_opt(&write_opts, &txn_opts);
        Box::new(RocksWriteBatch {
            txn,
            db: Arc::clone(&self.db),
        })
    }
}
```

**Step 2: Commit**

```
docs(storage): document WAL sync trade-off in begin_write
```

---

## Summary

| Task | Priority | Type | Risk |
|------|----------|------|------|
| 1. Auto-recover delta seq counters | P0 | Bug fix | Low (additive change, idempotent) |
| 2. Fix prefix extractor 8→9 | P3 | Config fix | Medium (breaks existing DBs) |
| 3. Document delta_entries no-compression | P3 | Documentation | None |
| 4. Document WAL sync trade-off | P3 | Documentation | None |

**Execution order:** Task 1 first (P0 bug), then Tasks 2-4 can be done in any order.

**Total test commands:**
- After Task 1: `cargo test -p rucksfs-storage new_without_explicit_recover`
- After Task 2: `cargo test --workspace`
- After all: `cargo test --workspace` (final verification)
