# RucksFS Performance Optimization Loop — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the 4-15x performance gap between RucksFS and ext4 on metadata operations through iterative, measured optimization rounds.

**Architecture:** Each round is an independent cycle: profile → implement one change → benchmark with strict discipline → merge or revert. All mutations happen on feature branches. The benchmark protocol enforces clean-slate cold starts with `-n 10000` to prevent the hot-cache inflation that invalidated previous measurements.

**Tech Stack:** Rust, RocksDB (rust-rocksdb), FUSE (fuser), rucksfs-bench

**Spec:** `docs/superpowers/specs/2026-03-13-perf-optimization-loop-design.md`

---

## Chunk 1: Infrastructure — Benchmark Script & Baseline

### Task 1: Create reusable benchmark script

Encapsulate the Testing Discipline rules into a single script so every round uses the exact same procedure. This eliminates human error in the clean/mount/bench cycle.

**Files:**
- Create: `scripts/bench-round.sh`

- [ ] **Step 1: Write the benchmark script**

```bash
#!/usr/bin/env bash
# bench-round.sh — Clean-slate benchmark for one RucksFS optimization round.
# Usage: ./scripts/bench-round.sh <label> [n=10000]
#
# Implements all Testing Discipline rules from the spec:
# - Rule 1: Clean slate (unmount + delete data + fresh mount) before every run
# - Rule 2: Sufficient scale (n=10000 default)
# - Rule 3: Consistency check (two runs, each on fresh mount, auto-compare)
# - Rule 6: No concurrent load (kill stale processes)
# - Rule 7: Record everything (CSV output)
# - Rule 8: Multi-thread sanity check (auto -t 4 run)
#
# Outputs CSV to results/<label>/run{1,2}/ and results/<label>/mt/

set -euo pipefail

LABEL="${1:?Usage: bench-round.sh <label> [n]}"
N="${2:-10000}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BENCH="$ROOT/benchmark/bench-tool/target/release/rucksfs-bench"
RUCKSFS="$ROOT/target/release/rucksfs"
MNT="$ROOT/benchmark/bench-tool/fuse-mnt"
DATA="$ROOT/benchmark/bench-tool/fuse-data"
RESULTS="$ROOT/results/${LABEL}"

# --- Pre-flight ---
echo "=== Pre-flight checks ==="

if ! [ -x "$RUCKSFS" ]; then
    echo "ERROR: $RUCKSFS not found. Run: cargo build --release -p rucksfs --features rocksdb"
    exit 1
fi
if ! [ -x "$BENCH" ]; then
    echo "ERROR: $BENCH not found. Run: cd benchmark/bench-tool && cargo build --release"
    exit 1
fi

# Kill stale processes (Rule 6)
echo "Killing stale rucksfs processes..."
fusermount -u "$MNT" 2>/dev/null || true
fusermount -uz "$MNT" 2>/dev/null || true
pkill -f "rucksfs.*--mount" 2>/dev/null || true
sleep 2

if pgrep -f "rucksfs.*--mount" >/dev/null 2>&1; then
    echo "ERROR: rucksfs process still running after pkill"
    exit 1
fi

mkdir -p "$RESULTS/run1" "$RESULTS/run2" "$RESULTS/mt"

# --- Function: clean + mount + bench ---
run_once() {
    local out_dir="$1"
    local threads="$2"

    echo "--- Cleaning data (Rule 1) ---"
    fusermount -u "$MNT" 2>/dev/null || true
    fusermount -uz "$MNT" 2>/dev/null || true
    sleep 1
    pkill -f "rucksfs.*--mount" 2>/dev/null || true
    sleep 1
    rm -rf "$DATA"/*
    mkdir -p "$DATA" "$MNT"

    echo "--- Mounting fresh RucksFS ---"
    "$RUCKSFS" --mount "$MNT" --data-dir "$DATA" &
    sleep 3

    # Verify FUSE mount
    if ! mount | grep -q "$MNT.*fuse"; then
        echo "ERROR: FUSE mount not detected at $MNT"
        pkill -f "rucksfs.*--mount" 2>/dev/null || true
        exit 1
    fi

    echo "--- Running benchmark (-t $threads -n $N) ---"
    timeout 1800 "$BENCH" -m "$MNT" -t "$threads" -n "$N" -o "$out_dir" all 2>&1

    echo "--- Unmounting ---"
    fusermount -u "$MNT" 2>/dev/null || true
    sleep 1
    pkill -f "rucksfs.*--mount" 2>/dev/null || true
    sleep 1
}

# --- Single-thread runs (Rule 3: two runs, each on fresh mount) ---
echo ""
echo "========== Run 1 of 2 (-t 1 -n $N) =========="
run_once "$RESULTS/run1" 1

echo ""
echo "========== Run 2 of 2 (-t 1 -n $N) =========="
run_once "$RESULTS/run2" 1

# --- Multi-thread sanity check (Rule 8) ---
echo ""
echo "========== Multi-thread sanity check (-t 4 -n $N) =========="
run_once "$RESULTS/mt" 4

# --- Consistency check (Rule 3) ---
echo ""
echo "=== Consistency Check ==="
CSV1=$(ls "$RESULTS/run1/"*.csv 2>/dev/null | head -1)
CSV2=$(ls "$RESULTS/run2/"*.csv 2>/dev/null | head -1)

if [ -z "$CSV1" ] || [ -z "$CSV2" ]; then
    echo "WARNING: Could not find CSV files for consistency check"
    exit 0
fi

echo "Comparing run1 vs run2 (>15% variance = UNRELIABLE):"
echo ""
printf "%-10s %-10s %-12s %-12s %-8s %-6s\n" "Operation" "Mode" "Run1 ops/s" "Run2 ops/s" "Diff%" "Status"
printf "%-10s %-10s %-12s %-12s %-8s %-6s\n" "--------" "----" "----------" "----------" "-----" "------"

FAIL=0
while IFS=, read -r ts op mode threads nfpt total dur ops lat; do
    [ "$op" = "operation" ] && continue  # skip header
    # Find matching line in CSV2
    ops2=$(awk -F, -v o="$op" -v m="$mode" -v t="$threads" \
        '$2==o && $3==m && $4==t {print $8}' "$CSV2")
    if [ -n "$ops2" ] && [ -n "$ops" ]; then
        # Calculate percentage difference
        diff=$(awk "BEGIN {
            avg = ($ops + $ops2) / 2;
            if (avg > 0) printf \"%.1f\", (($ops - $ops2) / avg) * 100;
            else printf \"0.0\";
        }")
        abs_diff=$(awk "BEGIN { d=$diff; if (d<0) d=-d; printf \"%.1f\", d }")
        status="OK"
        if awk "BEGIN { exit !($abs_diff > 15) }"; then
            status="WARN"
            FAIL=1
        fi
        printf "%-10s %-10s %-12.1f %-12.1f %-7s%% %-6s\n" "$op" "$mode" "$ops" "$ops2" "$diff" "$status"
    fi
done < "$CSV1"

echo ""
if [ "$FAIL" -eq 1 ]; then
    echo "WARNING: Some operations have >15% variance. Results may be unreliable."
else
    echo "All operations within 15% variance. Results are consistent."
fi

echo ""
echo "=== Complete. Results in $RESULTS/ ==="
```

- [ ] **Step 2: Make executable and test with a dry run**

```bash
chmod +x scripts/bench-round.sh
# Quick smoke test with tiny n to verify the script works
./scripts/bench-round.sh smoke-test 10
```

Expected: Script creates `results/smoke-test/{run1,run2,mt}/`, runs three benchmark cycles, produces CSV files, prints consistency check table.

- [ ] **Step 3: Commit**

```bash
git add scripts/bench-round.sh
git commit -m "chore(bench): add reusable benchmark script with clean-slate discipline"
```

### Task 2: Establish verified baseline

Run the benchmark script to get the official baseline numbers that all optimization rounds will compare against.

**Files:**
- Output: `results/baseline-n10000/{run1,run2,mt}/` (CSV files)
- Output: `results/ext4-baseline-n10000/` (ext4 reference)

- [ ] **Step 1: Build release binaries**

```bash
cargo build --release -p rucksfs --features rocksdb
cd benchmark/bench-tool && cargo build --release && cd -
```

- [ ] **Step 2: Run tests to confirm clean starting state**

```bash
cargo test --workspace --exclude rucksfs-rpc
```

Expected: All tests pass. (Note: `rucksfs-rpc` is excluded because it requires protoc v3.15+.)

- [ ] **Step 3: Run RucksFS baseline**

```bash
./scripts/bench-round.sh baseline-n10000 10000
```

Expected: CSV files in `results/baseline-n10000/`. Consistency check passes (< 15% variance).

- [ ] **Step 4: Run ext4 baseline (twice for consistency)**

```bash
EXT4_DIR="/tmp/ext4-bench-baseline"
mkdir -p "$EXT4_DIR"

# Run 1
rm -rf "$EXT4_DIR"/*
sync
benchmark/bench-tool/target/release/rucksfs-bench -m "$EXT4_DIR" -t 1 -n 10000 -o results/ext4-baseline-n10000/run1 all

# Run 2
rm -rf "$EXT4_DIR"/*
sync
benchmark/bench-tool/target/release/rucksfs-bench -m "$EXT4_DIR" -t 1 -n 10000 -o results/ext4-baseline-n10000/run2 all
```

Save output. This is the target to match (80%+ of these numbers).

- [ ] **Step 5: Record baseline in optimization log**

Add a new section to `benchmark/bench-tool/optimization-log.md`:

```markdown
## New Baseline — 2026-03-13 (n=10000, cold start)

| Operation | RucksFS easy 1T | ext4 easy 1T | Gap | 80% Target |
|-----------|----------------|-------------|-----|------------|
| create    | <measured>     | <measured>  | Nx  | <ext4*0.8> |
| stat      | <measured>     | <measured>  | Nx  | <ext4*0.8> |
| ...       | ...            | ...         | ... | ...        |

Commit: <hash>
```

- [ ] **Step 6: Commit baseline data**

```bash
git add results/baseline-n10000/ results/ext4-baseline-n10000/ benchmark/bench-tool/optimization-log.md
git commit -m "docs(bench): record n=10000 cold-start baseline"
```

---

## Chunk 2: Round 1 — Audit & Reduce Write Path Operations

The biggest gap is in mutation operations (create/rename/unlink: 10-15x slower). The `create` path currently does:

1. `get_for_update_dir_entry` — pessimistic lock + read on parent dir entry (RocksDB get)
2. `allocator.alloc()` — atomic counter increment
3. `batch_put_inode` — put inode value (57 bytes)
4. `batch_put_dir_entry` — put dir entry (parent + name → child inode)
5. `batch_put_data_location` — put data location (inode → address)
6. `batch_parent_deltas` — put 2 delta entries (SetMtime, SetCtime)
7. `batch.commit()` — atomic transaction commit (single WAL write)

Then post-commit:
8. `allocator.maybe_persist()` — conditional RocksDB write (every 64 allocs)
9. `cache.put()` — in-memory LRU update
10. `cache.apply_deltas()` — in-memory cache update
11. `compaction.mark_dirty()` — mutex + possible condvar notify

### Task 3: Profile create to identify dominant cost

Before optimizing, measure where time is actually spent.

**Files:**
- Modify: `server/src/lib.rs` (temporary instrumentation, removed after profiling)

- [ ] **Step 0: Create feature branch**

```bash
git checkout main
git checkout -b opt/round-1-write-path-audit
```

- [ ] **Step 1: Add timing instrumentation to create**

Add `std::time::Instant` measurements around each phase in `create()` at `server/src/lib.rs:511`. This is temporary — will be removed after profiling.

Key measurements:
- `begin_write()` duration (transaction creation)
- `get_for_update_dir_entry()` duration (pessimistic lock acquisition)
- All `batch_put_*` calls combined (serialization + buffering)
- `commit()` duration (WAL write)
- Post-commit work (cache + compaction)

Log every 1000th operation to stderr to avoid flooding.

- [ ] **Step 2: Build and run with instrumentation**

```bash
cargo build --release -p rucksfs --features rocksdb
# Clean + mount + benchmark, capture stderr for profiling
fusermount -u benchmark/bench-tool/fuse-mnt 2>/dev/null || true
pkill -f "rucksfs.*--mount" 2>/dev/null || true
rm -rf benchmark/bench-tool/fuse-data/*
mkdir -p benchmark/bench-tool/fuse-data benchmark/bench-tool/fuse-mnt
./target/release/rucksfs --mount benchmark/bench-tool/fuse-mnt --data-dir benchmark/bench-tool/fuse-data 2>create-profile.log &
sleep 3
benchmark/bench-tool/target/release/rucksfs-bench -m benchmark/bench-tool/fuse-mnt -t 1 -n 10000 create
fusermount -u benchmark/bench-tool/fuse-mnt
```

- [ ] **Step 3: Analyze profile output**

```bash
grep "CREATE profile" create-profile.log | head -20
```

Identify which phase dominates: `begin_write` (transaction creation), `lock` (get_for_update), `puts` (batch operations), or `commit` (WAL write).

- [ ] **Step 4: Remove instrumentation**

Revert the timing code. Do NOT commit instrumented code.

- [ ] **Step 5: Based on profiling results, implement the optimization**

The specific optimization depends on profiling results. Most likely candidates:

**If `commit` dominates (> 30μs):** The WAL write is the bottleneck. Investigate RocksDB `WriteBatch` options, WAL group commit, or `set_sync(false)` confirmation.

**If `begin_write` dominates (> 10μs):** Transaction creation overhead. Consider reusing transaction objects or switching to plain `WriteBatch` (non-transactional) for create, since create's only conflict check is the dir entry existence test which could use a different mechanism.

**If `lock` (get_for_update) dominates:** The pessimistic lock on the dir entry is expensive. Consider replacing with an optimistic check: do a plain `get` first, and only use `get_for_update` if the entry exists (to return EEXIST atomically). For the common case (entry doesn't exist), skip the lock.

**If `puts` dominate:** The serialization and key encoding overhead. Profile further to find which put is expensive.

- [ ] **Step 6: Run tests**

```bash
cargo test --workspace --exclude rucksfs-rpc
```

- [ ] **Step 7: Commit the optimization**

```bash
git add -A
git commit -m "perf(server): <description based on what was optimized>"
```

- [ ] **Step 8: Benchmark the optimization**

```bash
./scripts/bench-round.sh round-1-<description> 10000
```

Compare against baseline. Apply decision criteria (>= 10% improvement, no > 5% regression). Also check -t 4 results for multi-thread sanity (no > 20% regression from baseline at -t 4).

- [ ] **Step 9: Merge or revert**

If merge criteria met:
```bash
git checkout main
git merge opt/round-1-<description>
```

If not met:
```bash
git checkout main
# Keep branch for reference but don't merge
```

- [ ] **Step 10: Log results**

Update `benchmark/bench-tool/optimization-log.md` with the round's results, on `main`.

```bash
git add benchmark/bench-tool/optimization-log.md results/round-1-*/
git commit -m "docs(bench): log round 1 results"
```

- [ ] **Step 11: Termination check**

Evaluate: are all operations at 80%+ of ext4? If yes, skip to Chunk 6. If no, continue to next round.

---

## Chunk 3: Round 2 — Readdir Investigation & Fix

Readdir is 58x slower than ext4 (9.76 vs 565 ops/s). But first, understand the benchmark semantics.

### Task 4: Audit readdir benchmark semantics and optimize

**Files:**
- Read: `benchmark/bench-tool/src/ops.rs:104-116`
- Read: `benchmark/bench-tool/src/main.rs:244-260` (dir chain setup)
- Modify: `storage/src/rocks.rs:262-285` (likely optimization target)
- Modify: `server/src/lib.rs:503-509` (possible caching)

- [ ] **Step 0: Create feature branch**

```bash
git checkout main
git checkout -b opt/round-2-readdir-optimization
```

- [ ] **Step 1: Confirm readdir benchmark behavior**

The readdir benchmark (`op_readdir_dirs` at `ops.rs:106`) does:
- Loop `n` times (10,000 iterations)
- Each iteration calls `fs::read_dir(&dir)` on the thread directory
- The thread directory contains `n` subdirectories created by the preceding `mkdir` phase
- So: 10,000 readdir calls × 10,000 entries per directory = **100 million entries scanned**

ext4 gets 565 ops/s = **1.77ms per readdir** on a 10,000-entry directory.
RucksFS gets 9.76 ops/s = **102ms per readdir**. This is 58x slower and a real performance issue.

- [ ] **Step 2: Profile the readdir path**

Add temporary timing to `readdir` in `server/src/lib.rs:503` and `list_dir` in `storage/src/rocks.rs:262` to identify whether the bottleneck is:
- RocksDB prefix iterator creation
- Per-entry key decoding (`extract_child_name`, `decode_dir_value`)
- Iterator overhead (seeking, next calls)
- FUSE serialization of large directory listings

- [ ] **Step 3: Implement readdir optimization based on findings**

Possible optimizations:
- **RocksDB ReadOptions tuning**: set `fill_cache(true)`, `set_prefix_same_as_start(true)`
- **Readdir result caching**: cache directory listings for repeated reads on same directory
- **Reduce per-entry overhead**: optimize `extract_child_name` and `decode_dir_value`
- **Avoid Vec allocation**: use iterator-based approach instead of collecting all entries

- [ ] **Step 4: Test, benchmark, decide**

```bash
cargo test --workspace --exclude rucksfs-rpc
./scripts/bench-round.sh round-2-readdir 10000
```

Same merge/revert/log process as Task 3 steps 8-11.

---

## Chunk 4: Rounds 3-5 — Systematic Write Path Optimization

### Task 5: Round 3 — Eliminate data_location write for create

Every `create` writes a `data_location` entry mapping `inode → default_data_location`. The read path already has a default fallback (at `server/src/lib.rs:~1012-1017`), so this write may be redundant.

**Files:**
- Modify: `server/src/lib.rs:540-544` (remove `batch_put_data_location` in `create`)
- Verify: all data_location read sites already fall back to default

- [ ] **Step 0: Create feature branch**

```bash
git checkout main
git checkout -b opt/round-3-skip-data-location
```

- [ ] **Step 1: Verify data_location read fallback exists everywhere**

Search for all reads of data_location. Confirm every read site has a default fallback.

```bash
rg "data_location" server/src/ storage/src/ client/src/
```

The existing fallback at `server/src/lib.rs` returns `self.default_data_location.address.clone()` when no entry exists. Verify no other read site lacks this.

- [ ] **Step 2: Remove the write from create (and mkdir if applicable)**

Remove the `batch_put_data_location` call from `create` at `server/src/lib.rs:540-544`.
Check if `mkdir`, `symlink`, or `link` also write data_location unnecessarily.

- [ ] **Step 3: Test, benchmark, decide**

```bash
cargo test --workspace --exclude rucksfs-rpc
./scripts/bench-round.sh round-3-skip-data-location 10000
```

Same merge/revert/log process.

### Task 6: Round 4 — Inode cache capacity tuning

The `load_inode` function at `server/src/lib.rs:205-232` has a cache with capacity 10,000 (`DEFAULT_CACHE_CAPACITY` at line 47). With `-n 10000`, the working set fills the cache exactly, causing thrashing.

**Files:**
- Modify: `server/src/lib.rs:47` — increase `DEFAULT_CACHE_CAPACITY`
- Possibly modify: `server/src/cache.rs` — if cache lock overhead is the issue

- [ ] **Step 0: Create feature branch**

```bash
git checkout main
git checkout -b opt/round-4-cache-capacity
```

- [ ] **Step 1: Check cache hit rate with temporary instrumentation**

Add atomic hit/miss counters to `load_inode`. Run benchmark. Remove instrumentation.

- [ ] **Step 2: Based on hit rate, adjust capacity or optimize**

If hit rate is low: increase capacity to 100,000+.
If hit rate is high but stat is still slow: investigate `cache.get()` lock contention.

- [ ] **Step 3: Test, benchmark, decide**

### Task 7: Round 5 — RocksDB configuration tuning at scale

Previous Round 1 (block cache) was reverted at `-n 100`. At `-n 10000`, amortization may make it beneficial.

**Files:**
- Modify: `storage/src/rocks.rs` (RocksDB open options)

- [ ] **Step 0: Create feature branch**

```bash
git checkout main
git checkout -b opt/round-5-rocksdb-config
```

- [ ] **Step 1: Add block cache + bloom filter**

In the RocksDB open path in `storage/src/rocks.rs`, add:
- 256MB shared LRU block cache
- 10-bit bloom filter per SST
- Cache index and filter blocks
- Pin L0 filter and index in cache

- [ ] **Step 2: Test, benchmark, decide**

---

## Chunk 5: Rounds 6+ — Advanced Optimizations

Execute only if the 80% target is not yet met after Chunks 2-4.

**Termination check:** Before starting this chunk, evaluate:
- Are all operations at 80%+ of ext4? → Skip to Chunk 6.
- Have 3 consecutive rounds shown no improvement? → Skip to Chunk 6.
- Have 15 rounds been completed? → Skip to Chunk 6.

### Task 8: Round 6 — Replace `block_on` with sync server

The FUSE layer calls `futures::executor::block_on()` for every operation (`client/src/fuse.rs`). Since `MetadataServer` methods are `async` but contain no actual `.await` points (all work is synchronous RocksDB calls), the async overhead is pure waste.

**Files:**
- Modify: `core/src/lib.rs` — add sync trait or make existing trait non-async
- Modify: `server/src/lib.rs` — remove `async` from methods
- Modify: `client/src/fuse.rs` — remove `block_on` wrappers
- Modify: `client/src/embedded.rs`, `client/src/vfs_core.rs` — update trait impls
- Modify: `demo/src/main.rs` — update if needed

This is an [L] change touching the full trait chain: core → server → client/embedded → client/vfs_core → client/fuse → demo. Only attempt if earlier rounds haven't closed the gap sufficiently.

- [ ] **Step 0: Create feature branch**

```bash
git checkout main
git checkout -b opt/round-6-sync-metadata
```

- [ ] **Step 1: Audit all async methods for actual await points**

```bash
rg "\.await" server/src/lib.rs | grep -v "delete_data\|data_server"
```

If all `.await` calls are in data paths (not metadata), the metadata methods can be made synchronous.

- [ ] **Step 2: Implement sync path for metadata operations**

Option A: Add a parallel `MetadataOpsSync` trait with non-async methods.
Option B: Remove `async` from `MetadataOps` trait entirely and update all impls.

- [ ] **Step 3: Update FUSE layer to call sync methods directly**

Remove `block_on` wrappers in `client/src/fuse.rs`.

- [ ] **Step 4: Test, benchmark, decide**

### Task 9: Round 7 — Eliminate get_for_update where unnecessary

In `create`, `get_for_update_dir_entry` acquires a pessimistic row lock to check if the name exists. With PCC transactions, write-write conflicts are already detected on commit, so the lock may be redundant.

**Files:**
- Modify: `server/src/lib.rs` — create, mkdir, symlink, link
- Possibly modify: `storage/src/rocks.rs` — add non-locking get method to AtomicWriteBatch

- [ ] **Step 0: Create feature branch**

```bash
git checkout main
git checkout -b opt/round-7-optimistic-create
```

- [ ] **Step 1: Analyze conflict scenarios**

Verify that PCC transaction write-conflict detection covers the duplicate-name case without needing `get_for_update`.

- [ ] **Step 2: Replace get_for_update with plain get**

If `AtomicWriteBatch` does not expose a non-locking `get` method, add one.

- [ ] **Step 3: Test with concurrent creates to verify correctness**

Write a test that spawns multiple threads creating the same filename — exactly one should succeed, others get EEXIST or TransactionConflict (retried).

- [ ] **Step 4: Test, benchmark, decide**

### Task 10: Rounds 8-15 — Remaining candidates

Execute based on remaining gap. Each follows the standard cycle: branch → profile → implement → test → benchmark → merge/revert → log.

Candidates:
- **WAL configuration tuning** [S] — group commit, WAL size limits
- **Column family tuning** [M] — memtable size, compaction style, compression per CF
- **Reduce allocator.maybe_persist overhead** [S] — increase persist interval
- **Optimize key encoding** [S] — reduce allocation in `encode_dir_entry_key`
- **Batch multiple FUSE requests** [L] — if fuser supports it

---

## Chunk 6: Completion & Documentation

### Task 11: Update performance documentation

**Files:**
- Modify: `docs/performance-optimizations.md`
- Modify: `benchmark/bench-tool/optimization-log.md`

- [ ] **Step 1: Update performance-optimizations.md with final numbers**

Replace all data with honest `-n 10000` cold-start measurements. Include:
- Final RucksFS vs ext4 comparison table
- Per-round improvement attribution
- Methodology section describing the clean-slate protocol

- [ ] **Step 2: Commit documentation**

```bash
git add docs/performance-optimizations.md benchmark/bench-tool/optimization-log.md
git commit -m "docs: update performance data with verified n=10000 measurements"
```

### Task 12: Final verification

- [ ] **Step 1: Run complete test suite**

```bash
cargo test --workspace --exclude rucksfs-rpc
```

- [ ] **Step 2: Run final benchmark**

```bash
./scripts/bench-round.sh final-verification 10000
```

This automatically runs -t 1 (twice for consistency) and -t 4 (sanity check).

- [ ] **Step 3: Verify all operations meet 80% of ext4 target**

Compare final numbers against ext4 baseline. List any operations still below 80%.

- [ ] **Step 4: Commit final results**

```bash
git add results/final-verification*/
git commit -m "docs(bench): record final verification benchmark"
```
