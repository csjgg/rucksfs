# Autonomous Performance Optimization Loop — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement a self-contained agent loop that iteratively profiles, diagnoses, optimizes, benchmarks, and reviews RucksFS metadata performance — fully autonomously.

**Architecture:** The plan is split into two phases. Phase 1 (Tasks 1-4) sets up the loop infrastructure: baseline benchmark, optimization log, and a reusable benchmark-and-compare script. Phase 2 (Tasks 5+) is the actual optimization loop, executed repeatedly. Each round creates a branch, implements one optimization, benchmarks, reviews, and merges or reverts.

**Tech Stack:** Rust, RocksDB, FUSE (fuser crate), rucksfs-bench (custom benchmark tool)

---

## Phase 1: Loop Infrastructure

### Task 1: Establish Baseline Benchmark

**Purpose:** Run the benchmark on current `main` to get baseline numbers that all future rounds compare against.

**Step 1: Check disk space**

Run: `df -h /`
Expected: At least 5GB free. If not, abort and warn.

**Step 2: Build release binaries**

Run:
```bash
cargo build --release -p rucksfs
cd benchmark/bench-tool && cargo build --release && cd ../..
```
Expected: Both compile without error.

**Step 3: Mount RucksFS**

```bash
mkdir -p benchmark/bench-tool/fuse-mnt benchmark/bench-tool/fuse-data
./target/release/rucksfs \
  --mount benchmark/bench-tool/fuse-mnt \
  --data-dir benchmark/bench-tool/fuse-data &
RUCKSFS_PID=$!
sleep 2
```

Verify: `mount | grep rucksfs` should show the mount.

**Step 4: Run full baseline benchmark**

```bash
benchmark/bench-tool/target/release/rucksfs-bench \
  -m benchmark/bench-tool/fuse-mnt \
  -t 1,2,4 \
  -n 100 \
  -o benchmark/bench-tool/results-baseline \
  all
```

Expected: 42 rows output (7 ops × 2 modes × 3 thread counts). CSV written to `results-baseline/`.

**IMPORTANT**: The `unlink` operation may be extremely slow (minutes even with -n 100). Set a 10-minute timeout per benchmark invocation. If it times out, note which operation was running and record partial results.

**Step 5: Cleanup FUSE**

```bash
fusermount -u benchmark/bench-tool/fuse-mnt
wait $RUCKSFS_PID 2>/dev/null
rm -rf benchmark/bench-tool/fuse-mnt benchmark/bench-tool/fuse-data
```

**Step 6: Record baseline data**

Read the CSV from `results-baseline/` and store as the comparison baseline. The CSV format is:
```
timestamp,operation,mode,num_threads,num_files_per_thread,total_ops,duration_sec,ops_per_sec,avg_latency_us
```

Parse and record the 1-thread easy-mode `ops_per_sec` for each of the 7 operations as the primary baseline metrics.

**Step 7: No commit needed** (results are gitignored)

---

### Task 2: Create Optimization Log

**Files:**
- Create: `benchmark/bench-tool/optimization-log.md`

**Step 1: Create the log file**

```markdown
# RucksFS Performance Optimization Log

> Tracks each optimization round: target, approach, result, decision.

## Baseline — YYYY-MM-DD

| Operation | 1T easy ops/s | 2T easy ops/s | 4T easy ops/s |
|-----------|--------------|--------------|--------------|
| create    | <from baseline> | ... | ... |
| stat      | ... | ... | ... |
| unlink    | ... | ... | ... |
| mkdir     | ... | ... | ... |
| rmdir     | ... | ... | ... |
| readdir   | ... | ... | ... |
| rename    | ... | ... | ... |

---
```

Fill in actual numbers from Task 1 baseline.

**Step 2: Commit**

```bash
git add benchmark/bench-tool/optimization-log.md
git commit -m "docs(bench): initialize optimization log with baseline data"
```

---

### Task 3: First Optimization Round — RocksDB Block Cache

This is the highest-impact, lowest-risk optimization. The block cache is currently at RocksDB default (~8MB). Adding a shared 256MB block cache will reduce disk I/O for all operations.

**Files:**
- Modify: `storage/src/rocks.rs:50-100` (cf_options + open_rocks_db)

**Step 1: Create optimization branch**

```bash
git checkout -b opt/round-1-rocksdb-block-cache
```

**Step 2: Run existing tests to confirm green baseline**

```bash
cargo test --workspace
```
Expected: All ~192 tests pass.

**Step 3: Add shared block cache to RocksDB configuration**

Modify `storage/src/rocks.rs`. The `cf_options` function (line 50) currently creates `BlockBasedOptions` with only a bloom filter. Add a shared `Cache` object.

Changes needed in `cf_options`:
- Accept an additional `&rocksdb::Cache` parameter
- Set `block_opts.set_block_cache(cache)` for all CFs that use block-based table factory
- Set `block_opts.set_cache_index_and_filter_blocks(true)` to put index/filter in cache
- Set `block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true)` to keep L0 hot

Changes needed in `open_rocks_db`:
- Create a shared `Cache::new_lru_cache(256 * 1024 * 1024)` (256MB)
- Pass it to `cf_options`

The function signature changes from:
```rust
fn cf_options(name: &str) -> Options
```
to:
```rust
fn cf_options(name: &str, block_cache: &rocksdb::Cache) -> Options
```

And `open_rocks_db` creates the cache:
```rust
let block_cache = rocksdb::Cache::new_lru_cache(256 * 1024 * 1024);
```

Then passes it when building CF descriptors:
```rust
let cf_descriptors: Vec<ColumnFamilyDescriptor> = ALL_CFS
    .iter()
    .map(|name| ColumnFamilyDescriptor::new(*name, cf_options(name, &block_cache)))
    .collect();
```

**Step 4: Run tests**

```bash
cargo test --workspace
```
Expected: All tests still pass (this is a configuration-only change, no logic changes).

**Step 5: Commit on branch**

```bash
git add storage/src/rocks.rs
git commit -m "perf(storage): add 256MB shared block cache with pinned L0 filters"
```

**Step 6: Build release and benchmark**

Follow the same benchmark protocol as Task 1 (build, mount, run with `-t 1,2,4 -n 100 all`, cleanup).

Output to: `benchmark/bench-tool/results-round-1/`

**Step 7: Compare results**

Parse the round-1 CSV and compare each operation's `ops_per_sec` against baseline.

**Improvement criteria:**
- Any operation ≥10% better AND no operation >5% worse → IMPROVED
- Otherwise → NOT IMPROVED

**Step 8: Code review**

Use the code-reviewer subagent to review the diff on `opt/round-1-rocksdb-block-cache` vs `main`.

Review checklist:
- Does the Cache import exist in rocksdb crate? (Check Cargo.toml dependencies)
- Is the cache shared correctly across all CFs?
- No breaking changes to public API?
- No changes to data format?

**Step 9: Decision**

If IMPROVED + review passes:
```bash
git checkout main
git merge --no-ff opt/round-1-rocksdb-block-cache -m "perf(storage): add 256MB shared block cache"
```
Copy round-1 results as new baseline:
```bash
rm -rf benchmark/bench-tool/results-baseline
cp -r benchmark/bench-tool/results-round-1 benchmark/bench-tool/results-baseline
```

If NOT IMPROVED:
```bash
git checkout main
git branch -D opt/round-1-rocksdb-block-cache
```

**Step 10: Update optimization log**

Append a round entry to `benchmark/bench-tool/optimization-log.md`:

```markdown
## Round 1 — YYYY-MM-DD — RocksDB Block Cache
- **Target**: all operations (infrastructure-level)
- **Bottleneck**: block_cache at default 8MB, no index/filter caching
- **Optimization**: 256MB shared LRU block cache, pin L0 filter/index
- **Branch**: opt/round-1-rocksdb-block-cache
- **Result**: <ops/s changes per operation>
- **Decision**: MERGED / REVERTED
- **Baseline updated**: yes / no
```

**Step 11: Commit log update**

```bash
git add benchmark/bench-tool/optimization-log.md
git commit -m "docs(bench): log round 1 results — rocksdb block cache"
```

---

### Task 4+: Subsequent Optimization Rounds (Adaptive Loop)

From this point, the agent enters the adaptive loop. Each round follows the SAME structure as Task 3 but with a different optimization target chosen dynamically.

**Round selection logic:**

At the start of each round:
1. Read the latest baseline CSV data
2. Compare each operation's ops/s against ext4 baseline (from design doc section 2)
3. Rank operations by gap-to-ext4 (largest gap = highest priority)
4. Check `optimization-log.md` for anti-loop rules:
   - Same operation targeted 2 rounds in a row? → force switch
   - Specific approach already tried and failed? → skip it
5. Select the target operation and diagnose its bottleneck by reading the relevant code path

**Known optimization candidates** (consult these when diagnosing bottlenecks):

#### For `unlink` (gap: 74,000x)
- **Async deferred delete**: `server/src/lib.rs:702-714` — `data_client.delete_data(inode).await` is called synchronously. Consider spawning deletion as a background task.
- **Batch delta cleanup**: `server/src/lib.rs:693` — `delta_store.clear_deltas()` is synchronous. Could batch or defer.
- **Release path**: `server/src/lib.rs:1196-1201` — physical delete on release is blocking.

#### For `create` (gap: 32x)
- **Transaction scope**: `server/src/lib.rs:522-580` — the entire create is one PCC transaction. Consider splitting timestamp deltas out (already partially done).
- **WAL write**: `storage/src/rocks.rs:676-693` — WAL sync is off by default but each commit still writes WAL. Consider group commit or write buffering.
- **Inode allocation persistence**: already batched every 64 allocs (good).

#### For `mkdir` (gap: 19x)
- Similar to create — transaction overhead dominates.
- **Dir entry encoding**: `storage/src/encoding.rs` — check if key encoding is efficient for dir CFs.

#### For `readdir` (gap: 9x)
- **Prefix scan**: reads all entries via RocksDB prefix iterator. Consider buffered/batched reads.
- **Per-entry `load_inode`**: each dir entry triggers a separate inode lookup. Consider `multi_get`.

#### For `rename` (gap: 9x)
- **Multiple row locks**: rename locks src/dst parent entries + inodes. Minimize lock duration.
- **Transaction scope**: `server/src/lib.rs:823-848` — sorted lock acquisition is good but scope is wide.

#### For `stat` (gap: 1.6x)
- **Cache hit rate**: with -n 100, cache should be warm. Gap may be irreducible FUSE overhead.
- **Delta folding on miss**: `server/src/lib.rs:205-232` — prefix scan cost on cache miss.

#### For `rmdir` (gap: 7x)
- Similar to unlink but simpler (no data deletion). Transaction overhead.

**Each round follows these steps identically:**

1. `git checkout -b opt/round-<N>-<description>` from main
2. `cargo test --workspace` (green baseline)
3. Implement the optimization (minimal changes)
4. `cargo test --workspace` (still green)
5. Commit on branch
6. Build release + mount + benchmark (`-t 1,2,4 -n 100 all`) + cleanup
7. Compare to baseline CSV
8. Code review via subagent
9. Decision: merge (with baseline update) or revert
10. Update optimization-log.md + commit

**Termination conditions** (check after each round):
- 3 consecutive rounds with no improvement → STOP
- All operations within 2x of ext4 → STOP (declare success)
- Unrecoverable error (disk full, unfixable build failure) → STOP

**Anti-loop rules:**
- Same operation targeted 2 consecutive rounds → must switch target
- Failed approach marked "tried" in log → never retry same approach
- If agent is stuck (can't identify new optimization for chosen target), switch to next-worst operation

---

## Key Reference: File Locations

| Component | File | Key Lines |
|-----------|------|-----------|
| RocksDB CF options | `storage/src/rocks.rs` | 50-84 (`cf_options`) |
| RocksDB open | `storage/src/rocks.rs` | 90-100 (`open_rocks_db`) |
| Transaction begin | `storage/src/rocks.rs` | 676-693 (`begin_write`) |
| Inode cache | `server/src/cache.rs` | 18-46 (`NUM_SHARDS`, `InodeFoldedCache`) |
| Cache capacity | `server/src/lib.rs` | 47 (`DEFAULT_CACHE_CAPACITY`) |
| Load inode (cache + delta fold) | `server/src/lib.rs` | 205-232 |
| Execute with retry | `server/src/lib.rs` | 370-397 |
| Deferred delete check | `server/src/lib.rs` | 403-414 |
| create() | `server/src/lib.rs` | 522-580 |
| unlink() | `server/src/lib.rs` | 648-724 |
| release() | `server/src/lib.rs` | 1196-1201 |
| Compaction config | `server/src/compaction.rs` | 22-40 |
| Delta threshold | `server/src/compaction.rs` | 25 (`DEFAULT_DELTA_THRESHOLD = 32`) |
| Compaction interval | `server/src/compaction.rs` | 22 (`DEFAULT_INTERVAL_MS = 5000`) |
| Key encoding | `storage/src/encoding.rs` | entire file |
| Demo CLI | `demo/src/main.rs` | 22-43 |
| Benchmark tool guide | `benchmark/bench-tool/AGENT_GUIDE.md` | entire file |
| Benchmark CSV format | `benchmark/bench-tool/src/report.rs` | 7-30 |

## Key Reference: Benchmark Commands

```bash
# Build everything
cargo build --release -p rucksfs
cd benchmark/bench-tool && cargo build --release && cd ../..

# Mount
mkdir -p benchmark/bench-tool/fuse-mnt benchmark/bench-tool/fuse-data
./target/release/rucksfs \
  --mount benchmark/bench-tool/fuse-mnt \
  --data-dir benchmark/bench-tool/fuse-data &
sleep 2

# Run (all ops, 1/2/4 threads, 100 files/thread)
benchmark/bench-tool/target/release/rucksfs-bench \
  -m benchmark/bench-tool/fuse-mnt \
  -t 1,2,4 -n 100 \
  -o benchmark/bench-tool/results-round-<N> \
  all

# Cleanup
fusermount -u benchmark/bench-tool/fuse-mnt
wait $RUCKSFS_PID 2>/dev/null
rm -rf benchmark/bench-tool/fuse-mnt benchmark/bench-tool/fuse-data
```

## Key Reference: Disk Space

- Check `df -h /` before every benchmark round. Abort if <5GB free.
- RocksDB data (`fuse-data/`) grows with file count. With `-n 100 -t 4`, expect ~50MB.
- Always clean up `fuse-mnt/` and `fuse-data/` after each benchmark.
- `target/` can be 40GB+. Only rebuild if source changed.

## Key Reference: Timeouts

- `unlink` on RucksFS can take minutes even with -n 100. Set 10-minute timeout on benchmark.
- `create` at high thread counts may take minutes. Same timeout applies.
- ext4 baseline with same parameters finishes in seconds.
