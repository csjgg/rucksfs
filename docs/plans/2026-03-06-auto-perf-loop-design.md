# Autonomous Performance Optimization Loop — Design

> Date: 2026-03-06
> Status: Draft
> Goal: Self-contained agent loop that iteratively profiles, optimizes, benchmarks, and reviews RucksFS performance without human intervention.

## 1. Overview

An autonomous loop where the agent:
1. Runs a full benchmark to establish current performance
2. Analyzes results to find the biggest bottleneck
3. Reads code to diagnose the root cause
4. Implements an optimization on a branch
5. Re-benchmarks and compares against baseline
6. Reviews code via subagent
7. Merges (if improved) or reverts (if not)
8. Repeats until no further gains are found

## 2. Current Performance Baseline (RucksFS vs ext4)

| Operation | RucksFS 1T (ops/s) | ext4 1T (ops/s) | Gap |
|-----------|--------------------:|----------------:|----:|
| stat      | 704,000 | 1,100,000 | 1.6x |
| create    | 5,200   | 167,000   | 32x  |
| unlink    | 3.2     | 238,000   | 74,000x |
| rename    | 21,400  | 190,000   | 9x   |
| mkdir     | 6,100   | 116,000   | 19x  |
| readdir   | 711     | 6,300     | 9x   |
| rmdir     | 18,500  | 131,000   | 7x   |

## 3. Known Optimization Surfaces

### 3.1 RocksDB Configuration (affects all operations)
- **block_cache**: Not configured (default ~8MB). Should be 256MB+.
- **write_buffer_size**: Default 64MB. May benefit from tuning per CF.
- **L0 filter/index pinning**: Not enabled. Eliminates I/O for bloom filter lookups.
- **cache_index_and_filter_blocks**: Not configured.

### 3.2 Delta / Compaction (affects unlink, create)
- **Delta threshold**: 32 deltas before compaction. May be too high for write-heavy workloads.
- **Deferred delete**: Synchronous physical deletion in `release()` path — catastrophically slow.
- **Read amplification**: Cache miss triggers prefix scan over all pending deltas.

### 3.3 Caching (affects stat, readdir)
- **Inode cache**: 10,000 entries, sharded LRU. Could be larger.
- **No dentry cache**: Every lookup hits RocksDB for directory entry resolution.
- **No negative cache**: Failed lookups re-hit storage.

### 3.4 Lock / Transaction (affects hard mode, high concurrency)
- **Nested mutex**: `open_handles` → `pending_deletes` in deferred delete path.
- **PCC row locks**: 5s timeout, deadlock detection enabled. Transaction scope could be narrowed.
- **Delta sequence allocator**: RwLock with double-checked locking — good, but first allocation per inode requires write lock.

## 4. Loop Architecture

```
                    ┌──────────────────────┐
                    │   0. Build Baseline   │
                    │   (main branch, full  │
                    │    benchmark)         │
                    └──────────┬───────────┘
                               │
                    ┌──────────▼───────────┐
               ┌───►│  1. Analyze Data      │
               │    │  Find biggest         │
               │    │  bottleneck operation  │
               │    └──────────┬───────────┘
               │               │
               │    ┌──────────▼───────────┐
               │    │  2. Diagnose Root     │
               │    │  Cause in Code        │
               │    └──────────┬───────────┘
               │               │
               │    ┌──────────▼───────────┐
               │    │  3. Implement on      │
               │    │  Branch               │
               │    │  + cargo test         │
               │    └──────────┬───────────┘
               │               │
               │    ┌──────────▼───────────┐
               │    │  4. Full Benchmark    │
               │    │  Compare to baseline  │
               │    └──────────┬───────────┘
               │               │
               │    ┌──────────▼───────────┐
               │    │  5. Code Review       │
               │    │  (subagent)           │
               │    └──────────┬───────────┘
               │               │
               │    ┌──────────▼───────────┐
               │    │  6. Decision          │
               │    │  Merge / Revert       │
               │    └──────────┬───────────┘
               │               │
               │    ┌──────────▼───────────┐
               │    │  7. Termination Check │
               │    │  3 consecutive fails  │
               │    │  → stop               │
               │    └──────────┬───────────┘
               │               │ continue
               └───────────────┘
```

## 5. Termination Conditions

The loop stops when ANY of:
- **3 consecutive rounds with no improvement** (ops/s gain <10% on any operation, or regression >5% on any operation)
- **All known optimization surfaces exhausted** (optimization log shows all identified items tried)
- **Unrecoverable error** (disk full, build failure that can't be fixed, FUSE mount failure)

## 6. Improvement Criteria

A round is considered "improved" when:
- **At least one operation** shows ≥10% ops/s improvement
- **No operation** shows >5% ops/s regression
- `cargo test --workspace` passes
- Code review has no blocking issues

## 7. Branch Management

```
main (baseline)
  ├── opt/round-1-<description>
  ├── opt/round-2-<description>   (merged → main)
  ├── opt/round-3-<description>   (reverted)
  └── opt/round-N-...
```

- Branch naming: `opt/round-<N>-<short-description>`
- Each round starts from latest `main`
- Merge: `git merge --no-ff` to preserve history
- Revert: `git branch -D` to discard

## 8. Benchmark Execution Protocol

Each round runs the same benchmark:

```bash
# Build
cargo build --release -p rucksfs
cd benchmark/bench-tool && cargo build --release && cd ../..

# Mount
mkdir -p benchmark/bench-tool/fuse-mnt benchmark/bench-tool/fuse-data
./target/release/rucksfs \
  --mount benchmark/bench-tool/fuse-mnt \
  --data-dir benchmark/bench-tool/fuse-data &
sleep 2

# Benchmark (all ops, 1/2/4 threads, 100 files per thread)
benchmark/bench-tool/target/release/rucksfs-bench \
  -m benchmark/bench-tool/fuse-mnt \
  -t 1,2,4 \
  -n 100 \
  -o benchmark/bench-tool/results-round-<N> \
  all

# Cleanup
fusermount -u benchmark/bench-tool/fuse-mnt
rm -rf benchmark/bench-tool/fuse-mnt benchmark/bench-tool/fuse-data
```

Parameters: `-t 1,2,4 -n 100 all` (all 7 ops, easy+hard modes, 3 thread counts).

## 9. Anti-Loop Rules

To prevent the agent from spinning on the same bottleneck:

1. **Same operation limit**: After 2 consecutive rounds targeting the same operation, force switch to a different one.
2. **No retry of failed approaches**: Each specific optimization approach is logged. If it was tried and reverted, it is marked "tried" and not attempted again.
3. **Diminishing returns**: If the best remaining gap to ext4 is <2x for all operations, declare optimization complete.

## 10. Optimization Log

Maintained at `benchmark/bench-tool/optimization-log.md`:

```markdown
## Round N — YYYY-MM-DD
- **Target operation**: unlink
- **Bottleneck analysis**: deferred delete performs synchronous physical deletion in release() path
- **Optimization**: move physical deletion to background thread pool
- **Branch**: opt/round-N-async-deferred-delete
- **Result**: unlink 3.2 → 850 ops/s (+265x), no regressions
- **Decision**: MERGED
- **Baseline updated**: yes
```

## 11. Code Review Protocol

Each round's changes are reviewed by a subagent (code-reviewer type) checking:
- Correctness: no new bugs, no data loss risks
- Consistency: maintains existing storage format and API contracts
- Style: follows project conventions (CLAUDE.md, code-style.md)
- Testing: new logic has test coverage where applicable
- Safety: no breaking changes to persistence layer

## 12. Decision Matrix

| Benchmark Result | Test Result | Review Result | Decision |
|-----------------|-------------|---------------|----------|
| Improved ≥10%   | Pass        | Pass          | **MERGE** |
| Improved ≥10%   | Pass        | Issues found  | Fix issues, re-benchmark |
| Improved ≥10%   | Fail        | —             | Fix tests or **REVERT** |
| No improvement  | —           | —             | **REVERT** |
| Regression >5%  | —           | —             | **REVERT** |

## 13. Disk Space Safety

Before each benchmark round:
- Check `df -h /` — abort if <5GB free
- Clean previous round's FUSE data before starting new round
- Never leave FUSE mounted between rounds

## 14. Artifacts

| Artifact | Location | Gitignored |
|----------|----------|------------|
| Baseline results | `benchmark/bench-tool/results-baseline/` | Yes |
| Round N results | `benchmark/bench-tool/results-round-<N>/` | Yes |
| Optimization log | `benchmark/bench-tool/optimization-log.md` | **No** (committed) |
| FUSE mount point | `benchmark/bench-tool/fuse-mnt/` | Yes |
| RocksDB data | `benchmark/bench-tool/fuse-data/` | Yes |
