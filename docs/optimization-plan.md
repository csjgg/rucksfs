# RucksFS Iterative Optimization Plan

## Methodology

Every optimization follows a strict closed loop:

```
1. Baseline measurement (before)
2. Code change (minimal, isolated)
3. Test measurement (after)
4. Compare -> keep or revert
```

All measurements use the same mdtest command on the same Tencent Cloud testbed (3 machines in HK).

**Baseline command** (single-process):
```bash
mdtest -C -T -r -n 3000 -z 0 -d /mnt/rucksfs-dist/bench
```

**Scaling command** (multi-process):
```bash
mpirun -np {1,2,4} mdtest -C -T -r -n 3000 -z 0 -d /mnt/rucksfs-dist/bench
```

## Current Baseline

| Metric | rucksfs-dist | NFS+ext4 | JuiceFS-Redis | Target |
|--------|-------------|----------|---------------|--------|
| Create (np=1) | 639 | 774 | 1,237 | >1,500 |
| Stat (np=1) | 6,387 | 25,992 | 7,453 | >15,000 |
| Remove (np=1) | 889 | 869 | 1,148 | >1,500 |
| Create scaling (np=1→4) | 636→697 (1.1x) | 794→2,171 (2.7x) | 1,227→3,978 (3.2x) | >2.5x |

## Problem Diagnosis

### Why scaling is flat (1.1x at np=4)

**Root cause**: `fuser::mount2()` runs a **single-threaded** event loop.

```
Kernel FUSE queue -> [fuser reads 1 request] -> [block_on(gRPC call)] -> [reply] -> [read next]
                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                     Only 1 request in flight at any time
```

Even with 4 mdtest processes generating requests concurrently, fuser processes them **one at a time**. The kernel queues them in `/dev/fuse`, but userspace drains them sequentially.

### Why NFS beats us on file create (774 vs 639)

Two factors:
1. **Write path overhead**: NFS uses ext4 journal write (~1 syscall). RucksFS uses RocksDB LSM write (WAL + memtable insert, heavier).
2. **Round-trip overhead**: NFS CREATE is 1 RPC. RucksFS create is also 1 gRPC call, but gRPC/HTTP2/protobuf has higher per-message overhead than NFS/RPC/XDR.

### Cost breakdown for a single `create` operation

```
[FUSE dispatch]  ~0.05ms   fuser read from /dev/fuse + parse
[block_on entry] ~0.01ms   tokio Handle::block_on setup
[gRPC serialize] ~0.02ms   protobuf encode request (~30 bytes)
[network RTT]    ~0.20ms   same-VPC TCP round-trip
[server decode]  ~0.02ms   protobuf decode
[RocksDB write]  ~0.80ms   atomic WriteBatch (6 ops: inode + dir_entry + parent deltas)
[gRPC response]  ~0.02ms   protobuf encode response (~65 bytes)
[network return] ~0.20ms   TCP return trip
[FUSE reply]     ~0.02ms   write reply to /dev/fuse
─────────────────────────
Total            ~1.34ms   → theoretical max ~746 ops/s (close to measured 639)
```

**Bottleneck rank**: RocksDB write (60%) > Network RTT (30%) > Everything else (10%)

---

## Optimization Phases

### Phase 1: FUSE Multithreading (P0 — Scaling Fix)

**Goal**: Enable concurrent FUSE request processing to unlock scaling.

**Approach**: Replace `fuser::mount2()` (single-threaded) with `fuser::Session::mount()` + manual multi-threaded dispatch using a thread pool.

```rust
// Before (single-threaded):
fuser::mount2(fs, mountpoint, &options)?;

// After (multi-threaded):
let session = fuser::Session::new(fs, mountpoint.as_ref(), &options)?;
let mut unmounter = session.unmount_callable();

// Ctrl+C handler
ctrlc::set_handler(move || { unmounter.unmount().ok(); })?;

// Run session on a thread pool
session.join();  // fuser 0.15 Session::run() supports multi-threaded mode
```

Actually, the better approach for fuser 0.15: use `fuser::spawn_mount2()` which runs the event loop on a background thread, or implement the `Filesystem` trait with interior mutability and spawn tasks from within callbacks.

**Key insight**: The real fix is to make FUSE callbacks **non-blocking**. Instead of `block_on(async_call)` which blocks the fuser thread, we should:

1. Option A: Use `fuser::spawn_mount2()` and spawn async tasks from callbacks
2. Option B: Spawn a thread pool, each thread runs its own `/dev/fuse` read loop (requires raw `/dev/fuse` access)
3. Option C: Switch to `polyfuse` or `fuse3` crate that supports async natively

**Recommended**: Option A first (lowest risk), then evaluate if Option B is needed.

**Expected impact**: Scaling from 1.1x → 2.5x+ at np=4.

**Test plan**:
```bash
# Before: measure scaling baseline
for np in 1 2 4 8; do
  mpirun -np $np mdtest -C -T -r -n 3000 -z 0 -d /mnt/rucksfs-dist/bench
done

# After: same commands, compare scaling ratio
```

---

### Phase 2: Latency Instrumentation (P0 — Prerequisite for data-driven optimization)

**Goal**: Add fine-grained tracing to quantify time spent in each layer.

**Approach**: Add `tracing::instrument` spans to measure:
- FUSE callback entry → exit
- gRPC call start → response received
- RocksDB write batch start → commit
- Network serialization time

```rust
// In MetadataRpcClient::create():
#[tracing::instrument(skip(self), fields(parent, name))]
async fn create(&self, parent: u64, name: &str, ...) -> FsResult<FileAttr> {
    let _rpc_span = tracing::info_span!("grpc_create").entered();
    // ...
}

// In MetadataServer::create():
#[tracing::instrument(skip(self))]
fn create(&self, parent: u64, name: &str, ...) -> FsResult<FileAttr> {
    let _rocks_span = tracing::info_span!("rocksdb_write").entered();
    // ...
}
```

Output: structured JSON traces that can be aggregated to show:
- P50/P99 latency per operation per layer
- Where time is actually spent

**Expected impact**: No performance change, but enables all subsequent optimizations to be measured precisely.

**Test plan**: Run benchmark with `RUST_LOG=info`, parse trace output, generate latency breakdown table.

---

### Phase 3: RocksDB Write Path Optimization (P1 — Close the NFS gap)

**Goal**: Reduce per-create RocksDB write latency from ~0.8ms to ~0.4ms.

**Sub-optimizations** (each tested independently):

#### 3a. WAL + sync mode tuning

```rust
// Current: default sync writes (every write flushes WAL to disk)
// Proposed: group commit / async WAL
let mut write_opts = WriteOptions::default();
write_opts.set_sync(false);  // Don't fsync WAL per write
// Rely on WAL for crash recovery, but batch fsync
```

**Risk**: Data loss on crash (last few ms of writes). Acceptable for benchmark; configurable in production.

**Expected impact**: 30-50% reduction in write latency.

#### 3b. WriteBatch size reduction

Current create() does 6 operations in 1 WriteBatch:
1. Put inode metadata
2. Put directory entry
3. Put parent mtime delta
4. Put parent ctime delta
5. Put parent nlink delta
6. Put parent size delta (child count)

**Optimization**: Combine parent deltas into a single compound delta entry.

```rust
// Before: 4 separate delta puts
batch.put(delta_key(parent, "mtime", seq), new_mtime);
batch.put(delta_key(parent, "ctime", seq), new_ctime);
batch.put(delta_key(parent, "nlink", seq), +1);
batch.put(delta_key(parent, "size", seq), +1);

// After: 1 compound delta put
batch.put(delta_key(parent, seq), CompoundDelta { mtime, ctime, nlink: +1, size: +1 });
```

**Expected impact**: ~15% reduction in WriteBatch size and serialization overhead.

#### 3c. Column family and bloom filter tuning

```rust
// Add bloom filter to metadata CF for faster point lookups
let mut cf_opts = Options::default();
cf_opts.set_bloom_filter(10, false);  // 10 bits per key
cf_opts.set_memtable_whole_key_filtering(true);
```

**Expected impact**: Faster stat/lookup (less disk I/O), minor create improvement.

**Test plan for all 3a/3b/3c**: Each tested independently, measure single-process create ops/s.

---

### Phase 4: gRPC Overhead Reduction (P1)

**Goal**: Reduce per-RPC overhead.

#### 4a. Connection pooling / multiple HTTP/2 streams

Current: single gRPC channel, HTTP/2 multiplexing.

```rust
// Create multiple channels and round-robin
let channels: Vec<Channel> = (0..4)
    .map(|_| Channel::from_static(addr).connect_lazy())
    .collect();
```

**Expected impact**: Better throughput under concurrent load (Phase 1 makes this relevant).

#### 4b. Combine write + report_write into single RPC

Current write path:
```
Client                    DataServer              MetadataServer
  |--- WriteData(ino) ------->|                        |
  |<-- WriteDataReply --------|                        |
  |--- ReportWrite(ino) ------------------------------>|
  |<-- ReportWriteReply ------------------------------|
```

Optimized write path:
```
Client                    DataServer              MetadataServer
  |--- WriteData(ino) ------->|                        |
  |                           |--- UpdateMeta -------->|
  |<-- WriteDataReply --------|                        |
```

DataServer calls MetadataServer internally, saving 1 client round-trip.

**Expected impact**: ~30% improvement in write throughput.

**Risk**: Increases coupling between DataServer and MetadataServer. May defer this.

---

### Phase 5: Read Path Optimization (P2 — Stat performance)

**Goal**: Improve stat from 6,387 to >15,000 ops/s.

#### 5a. Client-side inode cache

```rust
struct VfsCore {
    meta_client: Arc<dyn MetadataOps>,
    data_client: Arc<dyn DataOps>,
    attr_cache: Arc<DashMap<u64, (FileAttr, Instant)>>,  // inode -> (attr, expiry)
}
```

Cache getattr results for 1 second (matching FUSE entry_timeout).

**Expected impact**: Dramatic stat improvement for repeated accesses. mdtest stat benchmark does lookup+stat on recently created files, so cache hit rate should be very high.

#### 5b. Readdir prefetch

When readdir returns entries, prefetch and cache their attributes.

**Expected impact**: Faster `ls -l` and stat-after-readdir patterns.

---

### Phase 6: Batch Operations (P2 — Throughput ceiling)

**Goal**: Amortize per-operation overhead across multiple operations.

#### 6a. Client-side write buffering

Buffer small writes and flush as a single gRPC call.

#### 6b. Batch create RPC

New RPC: `BatchCreate(parent, names[]) -> attrs[]`
Server processes all creates in a single RocksDB WriteBatch.

**Expected impact**: Could 3-5x create throughput by amortizing RPC + RocksDB overhead.

**Risk**: Requires protocol change; only helps batch workloads (mdtest).

---

## Execution Order and Timeline

| Order | Phase | Effort | Expected Impact | Risk |
|-------|-------|--------|-----------------|------|
| 1 | Phase 2: Instrumentation | 2h | Enable measurement | None |
| 2 | Phase 1: FUSE multithreading | 4-8h | Scaling 1.1x → 2.5x+ | Medium (fuser API complexity) |
| 3 | Phase 3a: WAL sync tuning | 1h | Create +30-50% | Low |
| 4 | Phase 3b: Compound deltas | 2h | Create +15% | Low |
| 5 | Phase 5a: Attr cache | 2h | Stat 2-4x | Low |
| 6 | Phase 4a: Connection pool | 1h | Throughput under concurrency | Low |
| 7 | Phase 3c: Bloom filters | 1h | Stat +10-20% | None |
| 8 | Phase 4b: Combine write RPCs | 4h | Write +30% | Medium |
| 9 | Phase 5b: Readdir prefetch | 2h | ls -l perf | Low |
| 10 | Phase 6: Batch ops | 4-8h | Create 3-5x | High (protocol change) |

## Success Criteria

After all optimizations:

| Metric | Current | Target | Stretch |
|--------|---------|--------|---------|
| Create (np=1) | 639 | 1,500 | 3,000 |
| Stat (np=1) | 6,387 | 15,000 | 30,000 |
| Remove (np=1) | 889 | 1,500 | 3,000 |
| Create scaling (np=4/np=1) | 1.1x | 2.5x | 3.5x |

Comparison targets:
- Beat NFS+ext4 on all metrics
- Beat JuiceFS-Redis on create and stat
- Demonstrate near-linear scaling up to np=4

## Decision Framework

After each optimization:

```
IF improvement > 5%:
    KEEP the change, commit, update baseline
ELIF improvement 0-5%:
    KEEP if code is clean and adds no complexity
    REVERT if it adds complexity
ELIF regression:
    REVERT immediately
```
