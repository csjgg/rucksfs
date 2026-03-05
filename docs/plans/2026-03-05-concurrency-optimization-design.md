# Concurrency and Consistency Optimization Design

**Date:** 2026-03-05
**Status:** Approved
**Scope:** server, storage layers

## Background

RucksFS uses a 4-CF RocksDB TransactionDB (`inodes`, `dir_entries`, `delta_entries`, `system`) with PCC (Pessimistic Concurrency Control) transactions for atomic metadata mutations. Parent directory attribute updates (nlink, timestamps) are deferred as delta entries and compacted in a background thread.

This design addresses correctness risks and performance bottlenecks identified in the current implementation.

## Problems Identified

### P0 — Correctness

1. **Delta append outside transaction**: `create`, `mkdir`, `unlink`, `rmdir`, `rename`, `link`, `symlink` commit the main transaction first, then append parent deltas via non-transactional `WriteBatch`. If the process crashes between commit and delta append, nlink changes are lost — potentially corrupting directory link counts.

2. **Delta append vs compaction race**: `append_deltas` uses non-transactional writes while `force_compact_inode` reads/deletes deltas within a transaction. A new delta written between compaction's scan and commit can be silently lost when the compacted base overwrites it.

3. **Redundant `clear_deltas`**: `compaction.rs:181` calls `clear_deltas` after the transaction already deleted all delta keys, causing an unnecessary scan+delete pass.

### P1 — Performance

4. **Global cache mutex contention**: `InodeFoldedCache` uses a single `Mutex<LruCache>` — every `getattr`/`lookup` contends on this lock. Under concurrent `stat` workloads (e.g., `find`, `ls -lR`) this is a bottleneck.

5. **Compaction sleep polling**: Fixed 5-second `thread::sleep` interval is unresponsive to load changes.

6. **Frequent allocator persist**: Every `create`/`mkdir`/`symlink` persists the inode counter — one extra RocksDB write per file creation.

7. **No backoff on retry**: Transaction conflict retries happen immediately without backoff, increasing contention under load.

## Design

### Section 1: Transactional Delta for nlink Changes

**Decision:** Move `IncrementNlink` deltas into the main transaction. Keep timestamp deltas (`SetMtime`, `SetCtime`) outside the transaction.

**Rationale:**
- `nlink` is critical for filesystem consistency (affects `rmdir` empty-check, `unlink` deletion). Loss = filesystem corruption.
- Timestamp loss after crash is acceptable (only affects mtime/ctime accuracy).
- Minimizes transaction scope expansion (only one additional delta write per operation).

**Implementation:**

```rust
// In create(), mkdir(), etc. — before batch.commit():
if !nlink_deltas.is_empty() {
    for delta in &nlink_deltas {
        let seq = self.delta_store.next_seq(parent);
        let key = encode_delta_key(parent, seq);
        batch.push(BatchOp::PutDelta { key, value: delta.serialize() });
    }
}
batch.commit()?;

// After commit — timestamp deltas remain outside:
self.append_parent_deltas(parent, &timestamp_deltas);
```

**Changes required:**
- Expose `next_seq()` on `DeltaStore` trait (or add a `delta_seq_allocator` to `MetadataServer`)
- Split delta lists in each mutation method: nlink deltas → in-txn, timestamp deltas → post-txn
- Remove redundant `clear_deltas` call in `compaction.rs:181`

### Section 2: Sharded Cache

Replace `Mutex<LruCache>` with a 16-shard cache using `parking_lot::RwLock`.

```rust
const NUM_SHARDS: usize = 16;

pub struct ShardedInodeCache {
    shards: [parking_lot::RwLock<LruCache<Inode, InodeValue>>; NUM_SHARDS],
}

impl ShardedInodeCache {
    fn shard_index(inode: Inode) -> usize {
        // Fibonacci hashing to distribute sequential inodes
        let hash = inode.wrapping_mul(0x9E3779B97F4A7C15);
        (hash >> 60) as usize  // top 4 bits → 0..15
    }
}
```

**Key decisions:**
- `parking_lot::RwLock` — smaller, faster, no poisoning
- 16 shards, each with `capacity / 16` entries
- LRU `get()` requires write lock (updates LRU order), but contention reduced to 1/16
- Fibonacci hashing prevents clustering of sequentially-allocated inodes
- Public API unchanged (`get`, `put`, `invalidate`, `apply_deltas`) — no upstream changes needed

### Section 3: Adaptive Compaction

Replace `thread::sleep` polling with `Condvar` notification.

```rust
pub struct DeltaCompactionWorker<M, DS> {
    // ...existing fields...
    notify: Condvar,
    notify_mutex: Mutex<bool>,
}

impl DeltaCompactionWorker {
    pub fn mark_dirty(&self, inode: Inode) {
        // ...insert into dirty set...
        // Wake compaction thread
        let mut flag = self.notify_mutex.lock().unwrap();
        *flag = true;
        self.notify.notify_one();
    }

    pub fn run_loop(&self) {
        // Wait on Condvar with max_interval timeout
        // Process dirty inodes when notified or on timeout
    }
}
```

**Behavior:**
- Idle: sleeps until notified (no CPU waste)
- Under load: wakes immediately when dirty inodes are registered
- Fallback: max interval timeout ensures progress even if notification is missed

### Section 4: Transaction Optimizations

#### 4a. Exponential backoff with jitter on retry

```rust
fn execute_with_retry<F, T>(&self, mut f: F) -> FsResult<T> {
    for attempt in 0..TXN_MAX_RETRIES {
        match f() {
            Ok(v) => return Ok(v),
            Err(FsError::TransactionConflict) if attempt + 1 < TXN_MAX_RETRIES => {
                let base_us = 1000u64 << attempt;  // 1ms, 2ms, 4ms
                let jitter_us = pseudo_random_jitter(base_us / 2);
                std::thread::sleep(Duration::from_micros(base_us + jitter_us));
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}
```

#### 4b. Configure TransactionOptions

```rust
let mut txn_opts = TransactionOptions::default();
txn_opts.set_lock_timeout(5000);     // 5s lock wait timeout
txn_opts.set_deadlock_detect(true);  // Enable deadlock detection
```

#### 4c. Batch allocator persistence

Persist every 64 allocations instead of every allocation. Crash recovery may skip up to 63 inode numbers (not data loss — just number gaps).

```rust
pub fn alloc(&self) -> Inode {
    let ino = self.next.fetch_add(1, Ordering::Relaxed);
    ino
}

// Caller persists periodically or at shutdown
pub fn maybe_persist(&self, store: &dyn MetadataStore) -> FsResult<()> {
    let val = self.next.load(Ordering::Relaxed);
    if val % 64 == 0 {
        store.put(NEXT_INODE_KEY, &val.to_be_bytes())?;
    }
    Ok(())
}
```

### Section 5: Lock Ordering Documentation

Formalize the lock acquisition order for `open_handles` and `pending_deletes`:

```
Lock order: open_handles → pending_deletes (ALWAYS)
```

Extract a helper method to enforce this:

```rust
/// Check if inode should be deleted on release.
/// Lock order: open_handles → pending_deletes.
fn check_and_clear_deferred_delete(&self, inode: Inode) -> bool {
    let mut handles = self.open_handles.lock().expect("poisoned");
    if let Some(count) = handles.get_mut(&inode) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            handles.remove(&inode);
            let mut pending = self.pending_deletes.lock().expect("poisoned");
            return pending.remove(&inode);
        }
    }
    false
}
```

## Change Summary

| Module | File(s) | Change | Risk |
|--------|---------|--------|------|
| Storage trait | `storage/src/lib.rs` | Expose `next_seq` on `DeltaStore` | Low |
| Storage impl | `storage/src/rocks.rs` | Configure `TransactionOptions` | Low |
| Allocator | `storage/src/allocator.rs` | Batch persist (every 64) | Low |
| Cache | `server/src/cache.rs` | Rewrite as `ShardedInodeCache` | Medium (API stable) |
| Compaction | `server/src/compaction.rs` | Add `Condvar`, remove redundant `clear_deltas` | Low |
| MetadataServer | `server/src/lib.rs` | In-txn nlink delta, backoff retry, lock helpers | Medium |
| Dependencies | `server/Cargo.toml` | Add `parking_lot` | Low |

## Testing Strategy

1. All existing tests must pass (`cargo test --workspace`)
2. Add concurrent mutation tests for the cache sharding
3. Add crash-recovery test for nlink delta atomicity
4. Benchmark before/after with `task remote-test:bench-only`

## Dependencies

- `parking_lot` (for `RwLock`)
- No other new crate dependencies (jitter uses simple deterministic hash, not `rand`)
