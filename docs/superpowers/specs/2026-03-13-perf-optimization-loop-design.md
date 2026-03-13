# Design: Autonomous Performance Optimization Loop

## Goal

Close the performance gap between RucksFS and ext4 for all metadata operations.
Target: every operation reaches 80%+ of ext4 throughput under `-t 1 -n 10000`.

Current gap (measured 2026-03-12, cold start, -n 10000):

| Operation | RucksFS (ops/s) | ext4 (ops/s) | Gap |
|-----------|----------------|-------------|-----|
| create | 15,699 | 168,133 | 10.7x |
| stat | 780,349 | 1,131,281 | 1.4x |
| rename | 14,752 | 176,695 | 12.0x |
| unlink | 14,643 | 211,399 | 14.4x |
| mkdir | 18,325 | 75,414 | 4.1x |
| readdir | 9.76 | 565 | 57.9x |
| rmdir | 16,470 | 147,104 | 8.9x |

## Loop Structure

Each round follows a fixed cycle:

```
clean → mount → baseline → analyze → implement → test → bench → compare → merge/revert → log
```

## Testing Discipline (MANDATORY)

These rules are non-negotiable. Violating any of them invalidates the round.

### Rule 1: Clean Slate Before Every Benchmark

Before EVERY benchmark run (baseline or post-optimization):

```bash
# 1. Unmount
fusermount -u <mount_point>

# 2. Wait for process to exit
sleep 2
# Verify: ps aux | grep rucksfs should show nothing

# 3. Delete ALL data
rm -rf <data_dir>/*

# 4. Recreate directories
mkdir -p <data_dir> <mount_point>

# 5. Fresh mount
./target/release/rucksfs --mount <mount_point> --data-dir <data_dir>

# 6. Verify mount is FUSE
mount | grep <mount_point>
# Must show "type fuse", not a regular directory
```

**Never reuse a mount point from a previous run.** Hot RocksDB caches, leftover
SST files, and WAL fragments all contaminate results.

### Rule 2: Sufficient Scale

- Always use `-n 10000` minimum
- Verify each operation's total duration > 50ms in the CSV output
- If any operation takes < 50ms, increase -n and re-run the entire benchmark

### Rule 3: Consistency Check

- Run every benchmark **twice** on the same fresh mount
- If any operation differs by > 15% between runs, the result is unreliable — investigate or re-run
- Report the average of the two runs

### Rule 4: ext4 Baseline

- Run ext4 baseline once at the start of the optimization campaign
- ext4 test directory must also be cleaned before each run: `rm -rf <dir>/* && sync`
- Re-run ext4 baseline if the machine environment changes (kernel update, load change)

### Rule 5: Compilation Before Benchmark

- `cargo build --release -p rucksfs --features rocksdb` before every benchmark
- `cargo test --workspace --exclude rucksfs-rpc` must pass before benchmarking
- Never benchmark a debug build

### Rule 6: No Concurrent Load

- Verify no other RucksFS instances are running: `ps aux | grep rucksfs`
- Verify no heavy I/O on the system: `iostat -x 1 3` (disk util < 10%)
- Kill stale rucksfs processes before starting

### Rule 7: Record Everything

Every benchmark run produces:
- CSV file saved to `results/round-N-{baseline,optimized}/`
- Human-readable table in the optimization log
- Git commit hash of the code being tested

## Decision Criteria

- **Merge**: at least one operation improves >= 10%, AND no other operation regresses > 5%
- **Revert**: does not meet merge criteria
- **Log either way**: every round gets a full entry in `benchmark/bench-tool/optimization-log.md`

## Candidate Optimization Points

Ordered by expected impact. Actual execution order may change based on findings.

### Write Path (create/unlink/rename/mkdir/rmdir: 4-15x slower)

1. **Reduce RocksDB ops per mutation** — create currently does 6 ops (check dir entry,
   alloc inode, put inode, put dir entry, put data location, parent deltas). Investigate
   which can be eliminated or combined into fewer write calls.

2. **Eliminate unnecessary `get_for_update`** — audit which reads truly need pessimistic
   locking vs. can use plain `get` or cached values.

3. **WriteBatch consolidation** — ensure all puts within a mutation go through a single
   `write()` syscall.

4. **Transaction fixed overhead** — measure PCC transaction create/commit cost in isolation.

### Read Path (stat: 1.4x slower)

5. **Inode cache with folded delta** — cache the fully-folded inode value. On mutation,
   update the cache entry directly. Stat returns cache hit without re-scanning deltas.

6. **Delta scan cost** — even with zero deltas, prefix scan has overhead. Measure and
   consider a "no pending deltas" fast path.

### Readdir (58x slower)

7. **Batch inode loading** — readdir currently loads each child inode individually with
   delta folding. Replace with RocksDB MultiGet for all child inodes in one call.

8. **Readdir result caching** — short-lived cache of directory listings.

9. **Benchmark tool audit** — verify -n 10000 readdir test logic is correct (does it
   create 10000 dirs each with 10000 entries? or 10000 readdir calls on smaller dirs?)

### FUSE Layer

10. **`block_on()` overhead** — each FUSE call goes through futures::executor. Evaluate
    whether a dedicated tokio runtime thread or sync-only path is faster.

11. **Unnecessary clones/copies** — audit data copying on FUSE path.

### RocksDB Configuration

12. **Block cache + bloom filter** — previously reverted at -n 100; re-test at -n 10000
    where cache management overhead is amortized over more operations.

13. **Column family tuning** — memtable size, compaction style, compression per CF.

14. **WAL configuration** — group commit, WAL size limits.

## Termination Conditions

- All operations reach 80%+ of ext4 throughput, OR
- 3 consecutive rounds with no effective improvement, OR
- Candidate list exhausted

## Output

Each round produces:
- Git branch: `opt/round-N-<short-description>`
- Benchmark CSVs: `results/round-N-{baseline,optimized}/`
- Log entry in `benchmark/bench-tool/optimization-log.md`
- Updated `docs/performance-optimizations.md` with honest numbers

Final deliverable: updated performance doc with real -n 10000 cold-start numbers showing
the gap to ext4 and what each optimization achieved.
