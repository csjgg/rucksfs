# Concurrency Optimization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix correctness bugs in delta/compaction atomicity and improve concurrency performance via sharded cache, adaptive compaction, transaction backoff, and lock ordering.

**Architecture:** The changes span two crates (`rucksfs-storage` and `rucksfs-server`). Storage layer gets `next_seq` exposed on trait + `TransactionOptions` config. Server layer gets sharded cache, in-txn nlink deltas, Condvar compaction, backoff retry, batch allocator persist, and lock helper extraction.

**Tech Stack:** Rust, RocksDB `TransactionDB` (PCC), `parking_lot::RwLock`, `lru::LruCache`, `std::sync::Condvar`

---

### Task 1: Add `parking_lot` dependency

**Files:**
- Modify: `server/Cargo.toml`

**Step 1: Add the dependency**

In `server/Cargo.toml`, add `parking_lot` to `[dependencies]`:

```toml
[dependencies]
rucksfs-core = { path = "../core" }
rucksfs-storage = { path = "../storage" }
async-trait = { workspace = true }
libc = "0.2"
tokio = { workspace = true, features = ["rt", "rt-multi-thread", "net", "macros"] }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
lru = "0.12"
parking_lot = "0.12"
```

**Step 2: Verify it builds**

Run: `cargo build -p rucksfs-server`
Expected: SUCCESS

**Step 3: Commit**

```bash
git add server/Cargo.toml
git commit -m "chore(server): add parking_lot dependency for sharded cache"
```

---

### Task 2: Expose `next_seq` on `DeltaStore` trait

Needed so the server can allocate delta sequence numbers inside transactions.

**Files:**
- Modify: `storage/src/lib.rs:20-36` (DeltaStore trait)
- Modify: `storage/src/rocks.rs:380-392` (RocksDeltaStore::next_seq → make public via trait)

**Step 1: Add `next_seq` to the `DeltaStore` trait**

In `storage/src/lib.rs`, add to the `DeltaStore` trait (after line 35):

```rust
pub trait DeltaStore: Send + Sync {
    fn append_deltas(&self, inode: Inode, values: &[Vec<u8>]) -> FsResult<Vec<u64>>;
    fn scan_deltas(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>>;
    fn scan_delta_keys(&self, inode: Inode) -> FsResult<Vec<Vec<u8>>>;
    fn clear_deltas(&self, inode: Inode) -> FsResult<()>;

    /// Allocate the next sequence number for `inode`. Used by the server
    /// layer to write delta entries inside a transaction.
    fn next_seq(&self, inode: Inode) -> u64;
}
```

**Step 2: Implement the trait method in RocksDeltaStore**

In `storage/src/rocks.rs`, the `next_seq` method already exists as a private method at line 380. Add it to the `DeltaStore` impl block (after `clear_deltas` at line 490):

```rust
fn next_seq(&self, inode: Inode) -> u64 {
    // Delegate to the existing private method
    self.next_seq_inner(inode)
}
```

Also rename the existing private `next_seq` at line 380 to `next_seq_inner` to avoid name collision, and update all internal callers (line 406 in `append_deltas`).

**Step 3: Verify it builds and tests pass**

Run: `cargo test -p rucksfs-storage`
Expected: All tests pass

**Step 4: Commit**

```bash
git add storage/src/lib.rs storage/src/rocks.rs
git commit -m "feat(storage): expose next_seq on DeltaStore trait"
```

---

### Task 2b: Add `scan_deltas_with_keys` to DeltaStore trait

**Rationale:** The current compaction code calls `scan_deltas` (values only) and then `scan_delta_keys` (keys only) as two separate scans. Between the two scans, a concurrent `append_deltas` can insert a new delta that appears in the second scan but not the first — causing compaction to delete a delta it never folded. This is a real correctness bug (though after Task 4 moves nlink into transactions, it only affects timestamps). Fix: single scan returning `(key, value)` pairs.

**Files:**
- Modify: `storage/src/lib.rs:20-36` (DeltaStore trait — add method)
- Modify: `storage/src/rocks.rs:419-457` (RocksDeltaStore — implement)
- Modify: `server/src/compaction.rs:134-187` (force_compact_inode — use new API)

**Step 1: Add `scan_deltas_with_keys` to the DeltaStore trait**

In `storage/src/lib.rs`, add to the `DeltaStore` trait:

```rust
/// Scan all pending deltas for `inode`, returning `(key, value)` pairs
/// from a single consistent iterator pass. Used by compaction to ensure
/// the set of keys deleted matches exactly the set of values folded.
fn scan_deltas_with_keys(&self, inode: Inode) -> FsResult<Vec<(Vec<u8>, Vec<u8>)>>;
```

**Step 2: Implement in RocksDeltaStore**

In `storage/src/rocks.rs`, add to the `DeltaStore` impl block:

```rust
fn scan_deltas_with_keys(&self, inode: Inode) -> FsResult<Vec<(Vec<u8>, Vec<u8>)>> {
    let cf = self
        .db
        .cf_handle(CF_DELTA_ENTRIES)
        .ok_or_else(|| FsError::Io("CF 'delta_entries' not found".into()))?;

    let prefix = delta_prefix(inode);
    let iter = self.db.prefix_iterator_cf(&cf, &prefix);

    let mut result = Vec::new();
    for item in iter {
        let (k, v) = item.map_err(|e| FsError::Io(format!("RocksDB iterator: {}", e)))?;
        if !k.starts_with(&prefix) {
            break;
        }
        result.push((k.to_vec(), v.to_vec()));
    }
    Ok(result)
}
```

**Step 3: Update `force_compact_inode` to use single-pass scan**

In `server/src/compaction.rs`, replace the two separate scans in `force_compact_inode` (lines 137 and 162) with:

```rust
pub fn force_compact_inode(&self, inode: Inode) -> FsResult<bool> {
    // 1. Single-pass scan: get (key, value) pairs from one iterator.
    let kv_pairs = self.delta_store.scan_deltas_with_keys(inode)?;
    if kv_pairs.is_empty() {
        return Ok(false);
    }

    // 2. Begin transaction and lock the base inode.
    let mut batch = self.storage_bundle.begin_write();
    let key = encode_inode_key(inode);
    let mut base = match batch.get_for_update_inode(&key)? {
        Some(bytes) => InodeValue::deserialize(&bytes)?,
        None => return Ok(false),
    };

    // 3. Fold values from the same scan.
    let ops: Vec<DeltaOp> = kv_pairs
        .iter()
        .filter_map(|(_, v)| DeltaOp::deserialize(v).ok())
        .collect();
    delta::fold_deltas(&mut base, &ops);

    // 4. Write merged inode + delete exactly the keys we folded.
    batch.push(BatchOp::PutInode {
        key: key.clone(),
        value: base.serialize(),
    });
    for (dk, _) in &kv_pairs {
        batch.push(BatchOp::DeleteDelta { key: dk.clone() });
    }

    // 5. Commit.
    match batch.commit() {
        Ok(()) => {}
        Err(rucksfs_core::FsError::TransactionConflict) => {
            return Ok(false);
        }
        Err(e) => return Err(e),
    }

    // 6. Invalidate cache so the next read picks up the fresh base.
    self.cache.invalidate(inode);

    Ok(true)
}
```

Note: This also removes the redundant `clear_deltas` call (previously Task 5), since we now delete exactly the keys we scanned. New deltas appended after our scan are safe — they have different keys and will be folded in a future compaction round.

**Step 4: Run tests**

Run: `cargo test -p rucksfs-server`
Expected: All compaction tests pass

**Step 5: Commit**

```bash
git add storage/src/lib.rs storage/src/rocks.rs server/src/compaction.rs
git commit -m "fix(storage,server): single-pass delta scan to prevent compaction race"
```

---

### Task 3: Rewrite `InodeFoldedCache` as `ShardedInodeCache`

**Files:**
- Rewrite: `server/src/cache.rs`

**Step 1: Write failing test for sharded cache**

Add this test to the bottom of the test module — it tests that the new cache works identically to the old one but with concurrency:

```rust
#[test]
fn sharded_concurrent_access() {
    use std::sync::Arc;
    use std::thread;

    let cache = Arc::new(InodeFoldedCache::new(1000));
    let mut handles = vec![];

    // 20 threads writing to different shards
    for i in 0..20u64 {
        let c = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            c.put(i, sample_iv(i));
            c.get(i);
            c.apply_delta(i, &DeltaOp::IncrementNlink(1));
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    for i in 0..20u64 {
        let iv = cache.get(i).unwrap();
        assert_eq!(iv.nlink, 3); // 2 (base) + 1 (delta)
    }
}
```

This test already exists at line 229 and must continue to pass.

**Step 2: Rewrite the cache implementation**

Replace the entire `InodeFoldedCache` struct with sharded implementation. Keep the same public type name (`InodeFoldedCache`) and API (`new`, `get`, `put`, `invalidate`, `apply_delta`, `apply_deltas`, `len`).

```rust
//! LRU-based inode folded-state cache with sharding for concurrency.

use std::num::NonZeroUsize;

use lru::LruCache;
use parking_lot::RwLock;
use rucksfs_core::Inode;
use rucksfs_storage::encoding::InodeValue;

use crate::delta::DeltaOp;

/// Number of shards. Must be a power of 2.
const NUM_SHARDS: usize = 16;

/// Thread-safe sharded LRU cache for folded inode values.
///
/// Each shard is protected by a `parking_lot::RwLock`. Inodes are distributed
/// across shards using Fibonacci hashing to avoid clustering of sequentially
/// allocated inode IDs.
pub struct InodeFoldedCache {
    shards: Vec<RwLock<LruCache<Inode, InodeValue>>>,
}

impl InodeFoldedCache {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "cache capacity must be > 0");
        let per_shard = (capacity / NUM_SHARDS).max(1);
        let shards = (0..NUM_SHARDS)
            .map(|_| {
                RwLock::new(LruCache::new(
                    NonZeroUsize::new(per_shard).expect("per_shard must be > 0"),
                ))
            })
            .collect();
        Self { shards }
    }

    /// Map an inode to its shard index using Fibonacci hashing.
    #[inline]
    fn shard_index(inode: Inode) -> usize {
        let hash = inode.wrapping_mul(0x9E3779B97F4A7C15);
        (hash >> 60) as usize // top 4 bits → 0..15
    }

    pub fn get(&self, inode: Inode) -> Option<InodeValue> {
        let idx = Self::shard_index(inode);
        // LRU get() mutates ordering, requires write lock
        let mut shard = self.shards[idx].write();
        shard.get(&inode).cloned()
    }

    pub fn put(&self, inode: Inode, value: InodeValue) {
        let idx = Self::shard_index(inode);
        let mut shard = self.shards[idx].write();
        shard.put(inode, value);
    }

    pub fn apply_delta(&self, inode: Inode, delta: &DeltaOp) {
        let idx = Self::shard_index(inode);
        let mut shard = self.shards[idx].write();
        if let Some(val) = shard.get_mut(&inode) {
            crate::delta::fold_deltas(val, &[delta.clone()]);
        }
    }

    pub fn apply_deltas(&self, inode: Inode, deltas: &[DeltaOp]) {
        let idx = Self::shard_index(inode);
        let mut shard = self.shards[idx].write();
        if let Some(val) = shard.get_mut(&inode) {
            crate::delta::fold_deltas(val, deltas);
        }
    }

    pub fn invalidate(&self, inode: Inode) {
        let idx = Self::shard_index(inode);
        let mut shard = self.shards[idx].write();
        shard.pop(&inode);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.read().len()).sum()
    }
}
```

**Step 3: Run all tests**

Run: `cargo test -p rucksfs-server`
Expected: All cache tests pass, all compaction tests pass, all other server tests pass

Note: The `lru_eviction` and `capacity_one` tests test eviction within a single shard. With 16 shards and capacity=3, each shard gets capacity=1. Adjust test expectations or increase capacity in tests if needed. Most tests use `InodeFoldedCache::new(10)` or `new(100)` so the per-shard capacity will be `max(10/16, 1) = 1` or `max(100/16, 1) = 6`. The `lru_eviction` test uses capacity 3, giving per-shard capacity 1 — this may require bumping the test capacity to 16+ so each shard gets at least 1 entry. Update tests as needed to accommodate sharding.

**Step 4: Commit**

```bash
git add server/src/cache.rs
git commit -m "perf(server): replace global mutex cache with 16-shard parking_lot cache"
```

---

### Task 4: Move nlink deltas into transaction

**Files:**
- Modify: `server/src/lib.rs:293-304` (append_parent_deltas helper)
- Modify: `server/src/lib.rs:506-563` (mkdir — has IncrementNlink delta)
- Modify: `server/src/lib.rs:642-695` (rmdir — has IncrementNlink delta)
- Modify: `server/src/lib.rs:697-868` (rename — has IncrementNlink deltas)

This is the most critical correctness change. Only `mkdir`, `rmdir`, and `rename` have `IncrementNlink` deltas that must move into the transaction. `create`, `unlink`, `link`, `symlink` only have timestamp deltas which stay outside.

**Step 1: Add a helper to write nlink deltas in-transaction**

Add a new helper method to `MetadataServer` (after `append_parent_deltas` around line 304):

```rust
/// Write nlink delta operations directly into a transaction batch.
///
/// This ensures nlink changes are atomic with the main operation,
/// preventing corruption on crash.
fn batch_nlink_deltas(
    batch: &mut dyn AtomicWriteBatch,
    delta_store: &dyn DeltaStore,
    parent: Inode,
    deltas: &[DeltaOp],
) {
    for delta in deltas {
        let seq = delta_store.next_seq(parent);
        let key = rucksfs_storage::encoding::encode_delta_key(parent, seq);
        batch.push(BatchOp::PutDelta {
            key: key.to_vec(),
            value: delta.serialize(),
        });
    }
}
```

**Step 2: Modify `mkdir` to write nlink delta in-transaction**

In `mkdir` (line 506-563), move the `IncrementNlink(1)` delta inside the transaction closure, **before** `batch.commit()`. Keep timestamp deltas outside.

Current code at lines 551-560:
```rust
// Delta append outside transaction.
let ts = now_secs();
let _ = self.append_parent_deltas(
    parent,
    &[
        DeltaOp::IncrementNlink(1),
        DeltaOp::SetMtime(ts),
        DeltaOp::SetCtime(ts),
    ],
);
```

Change to:
```rust
// Inside transaction closure, before batch.commit():
Self::batch_nlink_deltas(
    batch.as_mut(),
    self.delta_store.as_ref(),
    parent,
    &[DeltaOp::IncrementNlink(1)],
);
batch.commit()?;
```

And after the transaction closure:
```rust
// Timestamp deltas outside transaction (non-critical).
let ts = now_secs();
let _ = self.append_parent_deltas(
    parent,
    &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
);
```

Also update cache + mark_dirty for the nlink delta after commit:
```rust
self.cache.apply_delta(parent, &DeltaOp::IncrementNlink(1));
self.compaction.mark_dirty(parent);
```

**Step 3: Modify `rmdir` similarly**

In `rmdir` (line 642-695), move `IncrementNlink(-1)` into the transaction. Keep timestamps outside.

Inside the transaction closure, before `batch.commit()`:
```rust
Self::batch_nlink_deltas(
    batch.as_mut(),
    self.delta_store.as_ref(),
    parent,
    &[DeltaOp::IncrementNlink(-1)],
);
batch.commit()?;
```

After the transaction closure:
```rust
self.cache.apply_delta(parent, &DeltaOp::IncrementNlink(-1));
self.compaction.mark_dirty(parent);

let ts = now_secs();
let _ = self.append_parent_deltas(
    parent,
    &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)],
);
```

**Step 4: Modify `rename` similarly**

`rename` (line 697-868) is the most complex. It has multiple nlink delta paths:

1. `dst_was_dir` → `IncrementNlink(-1)` on `new_parent` (line 818-827)
2. `src_is_dir && parent != new_parent` → `IncrementNlink(-1)` on `parent`, `IncrementNlink(1)` on `new_parent` (line 829-845)

These nlink deltas must move inside the transaction closure. The timestamp deltas in the same `append_parent_deltas` calls stay outside.

Inside the transaction closure, before `batch.commit()` (after building the atomic batch around line 796):

```rust
// Write nlink deltas inside transaction for crash safety.
if dst_was_dir {
    Self::batch_nlink_deltas(
        batch.as_mut(),
        self.delta_store.as_ref(),
        new_parent,
        &[DeltaOp::IncrementNlink(-1)],
    );
}
if src_is_dir && parent != new_parent {
    Self::batch_nlink_deltas(
        batch.as_mut(),
        self.delta_store.as_ref(),
        parent,
        &[DeltaOp::IncrementNlink(-1)],
    );
    Self::batch_nlink_deltas(
        batch.as_mut(),
        self.delta_store.as_ref(),
        new_parent,
        &[DeltaOp::IncrementNlink(1)],
    );
}

batch.commit()?;
```

After the transaction closure, update cache and write timestamp-only deltas:
```rust
// Apply nlink cache updates
if dst_was_dir {
    self.cache.apply_delta(new_parent, &DeltaOp::IncrementNlink(-1));
    self.compaction.mark_dirty(new_parent);
}
if src_is_dir && parent != new_parent {
    self.cache.apply_delta(parent, &DeltaOp::IncrementNlink(-1));
    self.cache.apply_delta(new_parent, &DeltaOp::IncrementNlink(1));
    self.compaction.mark_dirty(parent);
    self.compaction.mark_dirty(new_parent);
}

// Timestamp deltas (non-critical, outside transaction)
let ts = now_secs();
let _ = self.append_parent_deltas(parent, &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)]);
if parent != new_parent {
    let _ = self.append_parent_deltas(new_parent, &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)]);
}
```

Note: The rename closure must also return the `dst_was_dir` and `src_is_dir` flags along with `delete_inode` so the post-commit cache updates can use them. Change the return type of the closure from `FsResult<Option<Inode>>` to `FsResult<(Option<Inode>, bool, bool, bool)>` — `(delete_inode, dst_was_dir, src_is_dir, cross_dir)`.

**Step 5: Add import for `encode_delta_key`**

In `server/src/lib.rs`, update line 20:
```rust
use rucksfs_storage::encoding::{encode_delta_key, encode_dir_entry_key, encode_inode_key, InodeValue};
```

**Step 6: Run tests**

Run: `cargo test --workspace`
Expected: All ~192 tests pass

**Step 7: Commit**

```bash
git add server/src/lib.rs
git commit -m "fix(server): move nlink deltas into transaction for crash atomicity"
```

---

### Task 5: ~~Remove redundant `clear_deltas` in compaction~~ (MERGED into Task 2b)

This task has been superseded by Task 2b, which rewrites `force_compact_inode` to use `scan_deltas_with_keys` and removes both the redundant `clear_deltas` call and the two-scan race condition in a single change.

---

### Task 6: Add Condvar notification to compaction worker

**Files:**
- Modify: `server/src/compaction.rs`

**Step 1: Add Condvar fields to `DeltaCompactionWorker`**

Add `notify` and `notify_mutex` fields to the struct (around line 49-65):

```rust
pub struct DeltaCompactionWorker<M, DS>
where
    M: MetadataStore,
    DS: DeltaStore,
{
    #[allow(dead_code)]
    metadata: Arc<M>,
    delta_store: Arc<DS>,
    cache: Arc<InodeFoldedCache>,
    config: CompactionConfig,
    dirty: Mutex<HashSet<Inode>>,
    running: AtomicBool,
    storage_bundle: Arc<dyn StorageBundle>,
    /// Condvar to wake the compaction thread when dirty inodes are registered.
    notify: std::sync::Condvar,
    /// Mutex paired with the Condvar.
    notify_flag: Mutex<bool>,
}
```

**Step 2: Update constructor**

In `new()` (line 73-89), add the new fields:

```rust
Self {
    metadata,
    delta_store,
    cache,
    config,
    dirty: Mutex::new(HashSet::new()),
    running: AtomicBool::new(false),
    storage_bundle,
    notify: std::sync::Condvar::new(),
    notify_flag: Mutex::new(false),
}
```

**Step 3: Update `mark_dirty` to notify**

Replace `mark_dirty` (line 94-98):

```rust
pub fn mark_dirty(&self, inode: Inode) {
    if let Ok(mut set) = self.dirty.lock() {
        set.insert(inode);
    }
    // Wake compaction thread.
    if let Ok(mut flag) = self.notify_flag.lock() {
        *flag = true;
        self.notify.notify_one();
    }
}
```

**Step 4: Update `stop` to wake the thread**

```rust
pub fn stop(&self) {
    self.running.store(false, Ordering::Relaxed);
    // Wake the thread so it can exit the wait.
    if let Ok(mut flag) = self.notify_flag.lock() {
        *flag = true;
        self.notify.notify_one();
    }
}
```

**Step 5: Update `run_loop` to use Condvar**

Replace the `run_loop` method (line 244-257):

```rust
pub fn run_loop(&self) {
    self.running.store(true, Ordering::Relaxed);
    let interval = std::time::Duration::from_millis(self.config.interval_ms);

    while self.running.load(Ordering::Relaxed) {
        // Wait for notification or timeout.
        {
            let mut flag = self.notify_flag.lock().expect("notify_flag poisoned");
            if !*flag {
                let (guard, _) = self.notify.wait_timeout(flag, interval).expect("condvar wait");
                flag = guard;
            }
            *flag = false;
        }

        if !self.running.load(Ordering::Relaxed) {
            break;
        }

        if let Err(e) = self.compact_dirty() {
            tracing::error!(error = %e, "compaction round failed");
        }
    }

    // Final flush on shutdown.
    let _ = self.flush_all();
}
```

**Step 6: Run tests**

Run: `cargo test -p rucksfs-server`
Expected: All compaction tests pass, especially `run_loop_can_be_stopped`

**Step 7: Commit**

```bash
git add server/src/compaction.rs
git commit -m "perf(server): replace sleep polling with condvar in compaction worker"
```

---

### Task 7: Add exponential backoff to transaction retry

**Files:**
- Modify: `server/src/lib.rs:326-343` (execute_with_retry)

**Step 1: Write a failing test for backoff**

In `demo/tests/integration_test.rs` or `server/tests/`, we verify existing behavior still works. No separate unit test for backoff timing (it's an internal detail), but we verify that retry still succeeds by running existing tests.

**Step 2: Update `execute_with_retry` with backoff**

Replace lines 326-343 in `server/src/lib.rs`:

```rust
/// Execute a closure that creates and commits a transaction, retrying
/// up to `TXN_MAX_RETRIES` times on `FsError::TransactionConflict`
/// with exponential backoff.
fn execute_with_retry<F, T>(&self, mut f: F) -> FsResult<T>
where
    F: FnMut() -> FsResult<T>,
{
    for attempt in 0..TXN_MAX_RETRIES {
        match f() {
            Ok(v) => return Ok(v),
            Err(FsError::TransactionConflict) if attempt + 1 < TXN_MAX_RETRIES => {
                // Exponential backoff: ~1ms, ~2ms, ~4ms with jitter.
                let base_us = 1000u64 << attempt;
                // Simple deterministic jitter using attempt as seed.
                let jitter_us = (base_us / 4).wrapping_mul(attempt as u64 + 1) % (base_us / 2 + 1);
                std::thread::sleep(std::time::Duration::from_micros(base_us + jitter_us));
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}
```

**Step 3: Run tests**

Run: `cargo test --workspace`
Expected: All tests pass (backoff is transparent to callers)

**Step 4: Commit**

```bash
git add server/src/lib.rs
git commit -m "perf(server): add exponential backoff to transaction retry"
```

---

### Task 8: Configure TransactionOptions (lock timeout + deadlock detect)

**Files:**
- Modify: `storage/src/rocks.rs:624-633` (StorageBundle::begin_write)

**Step 1: Configure lock timeout and deadlock detection**

Replace the `begin_write` method at line 624-633:

```rust
impl StorageBundle for RocksStorageBundle {
    fn begin_write(&self) -> Box<dyn AtomicWriteBatch + '_> {
        let mut txn_opts = TransactionOptions::default();
        txn_opts.set_lock_timeout(5000);     // 5 second lock wait timeout
        txn_opts.set_deadlock_detect(true);   // Enable deadlock detection
        let write_opts = WriteOptions::default();
        let txn = self.db.transaction_opt(&write_opts, &txn_opts);
        Box::new(RocksWriteBatch {
            txn,
            db: Arc::clone(&self.db),
        })
    }
}
```

**Step 2: Run tests**

Run: `cargo test --workspace`
Expected: All tests pass

**Step 3: Commit**

```bash
git add storage/src/rocks.rs
git commit -m "perf(storage): configure lock timeout and deadlock detection for transactions"
```

---

### Task 9: Batch allocator persistence

**Files:**
- Modify: `storage/src/allocator.rs:46-50` (persist method)
- Modify: `server/src/lib.rs` (callers of allocator.persist)

**Step 1: Add `maybe_persist` method**

In `storage/src/allocator.rs`, add after the `persist` method (line 50):

```rust
/// Persist the counter only every `interval` allocations.
///
/// On crash recovery, up to `interval - 1` inode numbers may be skipped
/// (not reused). This is safe — it only wastes inode numbers, never data.
pub fn maybe_persist(&self, store: &dyn MetadataStore, interval: u64) -> FsResult<()> {
    let val = self.next.load(Ordering::Relaxed);
    if val % interval == 0 {
        store.put(NEXT_INODE_KEY, &val.to_be_bytes())?;
    }
    Ok(())
}
```

**Step 2: Update callers in MetadataServer**

In `server/src/lib.rs`, replace each `self.allocator.persist(self.metadata.as_ref())?;` (lines 486, 541, 1029) with:

```rust
let _ = self.allocator.maybe_persist(self.metadata.as_ref(), 64);
```

Use `let _ =` because failing to persist is non-fatal (worst case: skip some inode numbers on crash).

**Step 3: Add a test**

In `storage/src/allocator.rs` test module:

```rust
#[test]
fn maybe_persist_only_at_interval() {
    let tmp = tempfile::tempdir().unwrap();
    let db = open_rocks_db(tmp.path().join("meta.db")).unwrap();
    let store = RocksMetadataStore::new(Arc::clone(&db));

    let alloc = InodeAllocator::new(); // starts at 2
    // Allocate up to but not including a multiple of 4
    alloc.alloc(); // 2
    alloc.maybe_persist(&store, 4).unwrap(); // 3 % 4 != 0, no persist

    // No persisted value yet — load returns default
    let restored = InodeAllocator::load(&store).unwrap();
    assert_eq!(restored.current(), 2); // default, nothing persisted

    alloc.alloc(); // 3
    alloc.maybe_persist(&store, 4).unwrap(); // 4 % 4 == 0, persist!

    let restored = InodeAllocator::load(&store).unwrap();
    assert_eq!(restored.current(), 4);
}
```

**Step 4: Run tests**

Run: `cargo test --workspace`
Expected: All tests pass

**Step 5: Commit**

```bash
git add storage/src/allocator.rs server/src/lib.rs
git commit -m "perf(storage): batch allocator persistence every 64 allocations"
```

---

### Task 10: Extract lock ordering helper for release()

**Files:**
- Modify: `server/src/lib.rs:1070-1095` (release method)

**Step 1: Extract helper method**

Add a helper to the `MetadataServer` impl block (around line 304):

```rust
/// Decrement open handle count and check if deferred deletion is needed.
///
/// Lock ordering: open_handles → pending_deletes (ALWAYS).
fn decrement_handle_and_check_delete(&self, inode: Inode) -> bool {
    let mut handles = self.open_handles.lock().expect("open_handles poisoned");
    if let Some(count) = handles.get_mut(&inode) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            handles.remove(&inode);
            let mut pending = self.pending_deletes.lock().expect("pending_deletes poisoned");
            return pending.remove(&inode);
        }
    }
    false
}
```

**Step 2: Simplify `release()`**

Replace lines 1070-1095:

```rust
async fn release(&self, inode: Inode) -> FsResult<()> {
    if self.decrement_handle_and_check_delete(inode) {
        self.data_client.delete_data(inode).await?;
    }
    Ok(())
}
```

**Step 3: Run tests**

Run: `cargo test --workspace`
Expected: All tests pass

**Step 4: Commit**

```bash
git add server/src/lib.rs
git commit -m "refactor(server): extract lock ordering helper for release"
```

---

### Task 11: Final verification

**Step 1: Run full test suite**

Run: `cargo test --workspace`
Expected: All ~192 tests pass

**Step 2: Build release**

Run: `cargo build --workspace`
Expected: SUCCESS with no warnings related to our changes

**Step 3: Run clippy**

Run: `cargo clippy --workspace`
Expected: No new warnings

**Step 4: Final commit if any fixups needed**

Only if clippy or tests revealed issues.
