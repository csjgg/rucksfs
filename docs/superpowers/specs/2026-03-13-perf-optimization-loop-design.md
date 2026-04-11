# Design: Autonomous Performance Optimization Loop

## Goal

Close the performance gap between RucksFS and ext4 for all metadata operations.
Target: every operation reaches 80%+ of ext4 throughput under `-t 1 -n 10000`.

Current gap (measured 2026-03-12, cold start, fresh instance, -n 10000):

| Operation | RucksFS (ops/s) | ext4 (ops/s) | Gap |
|-----------|----------------|-------------|-----|
| create | 15,699 | 168,133 | 10.7x |
| stat | 780,349 | 1,131,281 | 1.4x |
| rename | 14,752 | 176,695 | 12.0x |
| unlink | 14,643 | 211,399 | 14.4x |
| mkdir | 18,325 | 75,414 | 4.1x |
| readdir | 9.76 | 565 | 57.9x |
| rmdir | 16,470 | 147,104 | 8.9x |

> **Note on previous results**: Earlier optimization rounds (1-11) measured at `-n 100`
> and reported ext4-parity on most operations. Those numbers were inflated by hot-run
> RocksDB cache effects (the second run on the same mount showed 9x higher throughput
> than cold start). At `-n 10000` cold start, the gap reappears because cache warming
> effects are amortized over more operations and no longer dominate the measurement.
>
> **Note on readdir**: The benchmark tool's readdir test at `-n 10000` creates 10,000
> directories and calls readdir on each. Each directory contains entries from the
> preceding mkdir phase. The extreme slowness (9.76 vs 565 ops/s) needs investigation —
> first confirm the benchmark semantics, then profile the readdir path.

## Loop Structure

Each round follows a fixed cycle:

```
clean → mount → baseline → analyze → implement → test → bench → compare → merge/revert → log
```

## Pre-flight Checklist (Run Once Before Campaign)

Before starting the first round, verify:

```bash
# 1. Binary builds
cargo build --release -p rucksfs --features rocksdb

# 2. Bench tool builds
cd benchmark/bench-tool && cargo build --release && cd -

# 3. Tests pass
cargo test --workspace --exclude rucksfs-rpc

# 4. FUSE mount works (trivial smoke test)
mkdir -p /tmp/rucksfs-preflight-mnt /tmp/rucksfs-preflight-data
./target/release/rucksfs --mount /tmp/rucksfs-preflight-mnt \
  --data-dir /tmp/rucksfs-preflight-data
# Verify: mount | grep preflight shows "type fuse"
fusermount -u /tmp/rucksfs-preflight-mnt
rm -rf /tmp/rucksfs-preflight-mnt /tmp/rucksfs-preflight-data

# 5. No stale processes
ps aux | grep rucksfs | grep -v grep  # should be empty

# 6. System load is low
iostat -x 1 3  # disk util < 10%
```

## Testing Discipline (MANDATORY)

These rules are non-negotiable. Violating any of them invalidates the round.

### Rule 1: Clean Slate Before Every Benchmark

Before EVERY benchmark run (baseline or post-optimization), including between
consistency check runs:

```bash
# 1. Unmount
fusermount -u <mount_point>

# 2. Kill any lingering rucksfs processes
sleep 2
pkill -f "rucksfs.*--mount" || true
sleep 1
# Verify: ps aux | grep rucksfs | grep -v grep should show nothing

# 3. Delete ALL data
rm -rf <data_dir>/*

# 4. Recreate directories
mkdir -p <data_dir> <mount_point>

# 5. Fresh mount
./target/release/rucksfs --mount <mount_point> --data-dir <data_dir>

# 6. Wait for mount
sleep 3

# 7. Verify mount is FUSE
mount | grep <mount_point>
# Must show "type fuse", not a regular directory
```

**Never reuse a mount point from a previous run.** Hot RocksDB caches, leftover
SST files, and WAL fragments all contaminate results.

### Rule 2: Sufficient Scale

- Always use `-n 10000` minimum
- Verify each operation's total duration > 50ms in the CSV output
- If any operation takes < 50ms, increase -n and re-run the entire benchmark
- Exception: stat may complete in < 50ms due to high throughput; this is acceptable
  if the total is > 10ms

### Rule 3: Consistency Check

- Run every benchmark **twice**, each on a completely fresh mount (clean → mount →
  bench → unmount → clean → mount → bench)
- If any operation differs by > 15% between runs, the result is unreliable — investigate
  or re-run
- Report the average of the two runs

### Rule 4: ext4 Baseline

- Run ext4 baseline once at the start of the optimization campaign
- ext4 test directory must also be cleaned before each run: `rm -rf <dir>/* && sync`
- Re-run ext4 baseline if the machine environment changes (kernel update, load change)

### Rule 5: Compilation Before Benchmark

- `cargo build --release -p rucksfs --features rocksdb` before every benchmark
- `cargo test --workspace --exclude rucksfs-rpc` must pass before benchmarking
  (note: `rucksfs-rpc` requires protoc v3.15+ which may not be available)
- Never benchmark a debug build

### Rule 6: No Concurrent Load

- Verify no other RucksFS instances are running: `ps aux | grep rucksfs | grep -v grep`
- Verify no heavy I/O on the system: `iostat -x 1 3` (disk util < 10%)
- Kill stale rucksfs processes before starting

### Rule 7: Record Everything

Every benchmark run produces:
- CSV file saved to `results/round-N-{baseline,optimized}/`
- Human-readable table in the optimization log
- Git commit hash of the code being tested

### Rule 8: Multi-thread Sanity Check

- Primary benchmark: `-t 1 -n 10000` (this is the decision basis)
- Secondary benchmark: `-t 4 -n 10000` (sanity check only)
- Multi-thread throughput must not regress > 20% from baseline at `-t 4`
- If multi-thread regresses but single-thread improves, investigate before merging

## Recovery Procedures

### Benchmark Hangs

- If the benchmark tool does not produce output for 5 minutes, kill it: `pkill rucksfs-bench`
- If a specific operation (e.g., readdir at -n 10000) is known to be very slow,
  set a timeout of 30 minutes for the full `all` run
- Log the hang in the optimization log

### FUSE Mount Stuck

- If `fusermount -u` fails: `fusermount -uz <mount_point>` (lazy unmount)
- Then `kill -9` any rucksfs processes
- Wait 5 seconds, verify unmount with `mount | grep <mount_point>`

### Build Failures

- If `cargo build` fails, fix the build error before proceeding
- Do not benchmark a binary that was built with a different source version

### Unexpected Regression with Logically Sound Optimization

- If an optimization that should improve performance shows regression, do NOT
  immediately revert. First:
  1. Re-run benchmark to confirm regression is reproducible
  2. Check system load: `iostat -x 1 3`, `top -b -n 1 | head -20`
  3. Use `strace -c -p <pid>` on the rucksfs process to compare syscall profile
  4. Only revert after confirming the regression is real and not environmental

## Decision Criteria

- **Merge**: at least one operation improves >= 10%, AND no other operation regresses > 5%
- **Revert**: does not meet merge criteria
- **Log either way**: every round gets a full entry in `benchmark/bench-tool/optimization-log.md`

## Candidate Optimization Points

Ordered by expected impact. Actual execution order may change based on findings.
Difficulty estimates: S = small/focused change, M = moderate, L = large/architectural.

### Write Path (create/unlink/rename/mkdir/rmdir: 4-15x slower)

1. **Reduce RocksDB ops per mutation** [M] — create currently does 6 ops (check dir entry,
   alloc inode, put inode, put dir entry, put data location, parent deltas). Investigate
   which can be eliminated or combined into fewer write calls.

2. **Eliminate unnecessary `get_for_update`** [M] — audit which reads truly need pessimistic
   locking vs. can use plain `get` or cached values.

3. **WriteBatch consolidation** [S] — ensure all puts within a mutation go through a single
   `write()` syscall.

4. **Transaction fixed overhead** [S] — measure PCC transaction create/commit cost in isolation.

### Read Path (stat: 1.4x slower)

5. **Inode cache with folded delta** [M] — cache the fully-folded inode value. On mutation,
   update the cache entry directly. Stat returns cache hit without re-scanning deltas.

6. **Delta scan cost** [S] — even with zero deltas, prefix scan has overhead. Measure and
   consider a "no pending deltas" fast path.

### Readdir (58x slower)

7. **Batch inode loading** [M] — readdir currently loads each child inode individually with
   delta folding. Replace with RocksDB MultiGet for all child inodes in one call.

8. **Readdir result caching** [M] — short-lived cache of directory listings.

9. **Benchmark tool audit** [S] — verify -n 10000 readdir test logic is correct (does it
   create 10000 dirs each with 10000 entries? or 10000 readdir calls on smaller dirs?).
   If the test semantics are unreasonable, fix the tool before optimizing the FS.

### FUSE Layer

10. **`block_on()` overhead** [L] — each FUSE call goes through futures::executor. Evaluate
    whether a dedicated tokio runtime thread or sync-only path is faster.

11. **Unnecessary clones/copies** [S] — audit data copying on FUSE path.

### RocksDB Configuration

12. **Block cache + bloom filter** [S] — previously reverted at -n 100; re-test at -n 10000
    where cache management overhead is amortized over more operations.

13. **Column family tuning** [M] — memtable size, compaction style, compression per CF.

14. **WAL configuration** [S] — group commit, WAL size limits.

## Git Workflow

- Branch from `main` for each round: `opt/round-N-<short-description>`
- Follow conventional commits per `.claude/rules/git-commits.md` (e.g.,
  `perf(server): reduce RocksDB ops in create`)
- Do NOT add Co-Authored-By lines (per project rules)
- On merge: merge branch to `main` with a merge commit
- On revert: delete the branch, no merge
- Keep reverted branches locally for reference but do not push

## Termination Conditions

- All operations reach 80%+ of ext4 throughput, OR
- 3 consecutive rounds with no effective improvement, OR
- Candidate list exhausted, OR
- Maximum 15 rounds

## Output

Each round produces:
- Git branch: `opt/round-N-<short-description>`
- Benchmark CSVs: `results/round-N-{baseline,optimized}/`
- Log entry in `benchmark/bench-tool/optimization-log.md`
- Updated `docs/performance-optimizations.md` with honest numbers

Final deliverable: updated performance doc with real -n 10000 cold-start numbers showing
the gap to ext4 and what each optimization achieved.
