# rucksfs-bench — Agent Usage Guide

> This document is for AI agents executing benchmarks. Not intended for human operators.

## Overview

`rucksfs-bench` is a Rust-native metadata benchmark tool that parallels [mdtest](https://github.com/LLNL/mdtest) methodology. It measures per-operation throughput (ops/s) and latency (us) for 7 POSIX metadata operations under configurable concurrency.

## Prerequisites

```bash
# Build the benchmark tool (independent crate, not a workspace member)
cd benchmark/bench-tool && cargo build --release

# Build RucksFS
cargo build --release -p rucksfs
```

Binary location: `benchmark/bench-tool/target/release/rucksfs-bench`

## CLI Reference

```
rucksfs-bench [OPTIONS] --mountpoint <PATH> <COMMAND>

Options:
  -m, --mountpoint <PATH>    Target directory (FUSE mount or local fs)
  -t, --threads <LIST>       Comma-separated thread counts [default: 1]
  -n, --num-files <N>        Files/dirs per thread [default: 10000]
  -o, --output <DIR>         CSV output directory [default: results]

Commands:
  create, stat, unlink, mkdir, rmdir, readdir, rename
    --mode <easy|hard>       [default: easy]
  all                        Run file chain + dir chain, both modes
```

## Running a Comparative Benchmark

### Step 1: Prepare FUSE mount

Create mount and data directories **inside `benchmark/bench-tool/`** (gitignored):

```bash
mkdir -p benchmark/bench-tool/fuse-mnt benchmark/bench-tool/fuse-data
```

Start RucksFS:

```bash
./target/release/rucksfs \
  --mount benchmark/bench-tool/fuse-mnt \
  --data-dir benchmark/bench-tool/fuse-data &
RUCKSFS_PID=$!
sleep 2
```

Verify:

```bash
mount | grep rucksfs
touch benchmark/bench-tool/fuse-mnt/test && rm benchmark/bench-tool/fuse-mnt/test
```

### Step 2: Run RucksFS benchmark

```bash
benchmark/bench-tool/target/release/rucksfs-bench \
  -m benchmark/bench-tool/fuse-mnt \
  -t 1,2,4,8 \
  -n <N> \
  -o benchmark/bench-tool/results-rucksfs \
  all
```

### Step 3: Run ext4 baseline

```bash
benchmark/bench-tool/target/release/rucksfs-bench \
  -m /tmp \
  -t 1,2,4,8 \
  -n <N> \
  -o benchmark/bench-tool/results-ext4 \
  all
```

### Step 4: Cleanup

```bash
fusermount -u benchmark/bench-tool/fuse-mnt
wait $RUCKSFS_PID 2>/dev/null
rm -rf benchmark/bench-tool/fuse-mnt benchmark/bench-tool/fuse-data
```

## Choosing `-n` (files per thread)

| Scenario | `-n` value | Notes |
|----------|-----------|-------|
| Quick smoke test | 100 | Finishes in seconds on ext4 |
| Standard benchmark | 1000 | Good balance of accuracy and speed |
| Paper-grade numbers | 10000 | Use for final results; slow ops (unlink with deferred delete) may take minutes |

**IMPORTANT**: RucksFS `unlink` triggers deferred deletion (background GC). With `-n 1000` and 8 threads, a single `unlink` phase can take **5+ minutes**. Plan timeouts accordingly:
- ext4 full `all` run: ~5 seconds
- RucksFS full `all` run with `-n 1000`: up to **30+ minutes** (dominated by unlink/create at high thread counts)

For initial performance profiling, run individual operations instead of `all`:

```bash
# Fast operations first
rucksfs-bench -m <mount> -t 1,2,4,8 -n 1000 stat --mode easy
rucksfs-bench -m <mount> -t 1,2,4,8 -n 1000 mkdir --mode easy
rucksfs-bench -m <mount> -t 1,2,4,8 -n 1000 rmdir --mode easy
rucksfs-bench -m <mount> -t 1,2,4,8 -n 1000 rename --mode easy

# Slow operations — may need lower -n or longer timeout
rucksfs-bench -m <mount> -t 1,2,4 -n 100 create --mode easy
rucksfs-bench -m <mount> -t 1,2,4 -n 100 unlink --mode easy
```

## Operation Chains in `all` Mode

`all` runs two chains per mode (easy + hard), avoiding redundant setup/cleanup:

- **File chain**: `create` → `stat` → `rename` → `unlink`
  - create produces files; stat reads them; rename moves them; unlink removes renamed files
- **Dir chain**: `mkdir` → `readdir` → `rmdir`
  - mkdir creates subdirs; readdir scans them; rmdir removes them

## Modes

| Mode | Directory layout | Concurrency model |
|------|-----------------|-------------------|
| **easy** | Each thread gets its own directory (`bench/thread-N/`) | No cross-thread contention |
| **hard** | All threads share one directory (`bench/shared/`) | Maximum metadata contention |

Easy mode measures raw throughput scaling. Hard mode measures contention behavior. Both are needed for a complete analysis (mirrors mdtest-easy vs mdtest-hard).

## Output

### Terminal

Formatted table with per-run rows + scaling analysis tables when multiple thread counts are used.

### CSV

Written to `-o` directory as `<timestamp>_bench.csv`:

```
timestamp,operation,mode,num_threads,num_files_per_thread,total_ops,duration_sec,ops_per_sec,avg_latency_us
```

## Key Metrics

| Metric | Formula | Notes |
|--------|---------|-------|
| `ops_per_sec` | `total_ops / max(thread_elapsed)` | mdtest convention: wall-clock throughput |
| `avg_latency_us` | `sum(thread_elapsed) / total_ops * 1e6` | Average per-op latency across all threads |
| `scaling_efficiency` | `ops_N / (ops_1 * N) * 100%` | 100% = linear scaling; shown in scaling analysis |

## Known Performance Characteristics (ext4 baseline, `-n 1000`)

| Operation | 1T ops/s | 8T easy efficiency |
|-----------|----------|-------------------|
| stat | ~1,100,000 | ~25% |
| unlink | ~238,000 | ~23% |
| rename | ~190,000 | ~67% |
| create | ~167,000 | ~5% |
| mkdir | ~116,000 | ~14% |
| readdir | ~6,300 | ~99% |

## Disk Space Warning

- RocksDB data (`fuse-data/`) grows with file count. With `-n 10000 -t 8`, expect ~500MB+.
- Cargo build artifacts (`target/`) are ~200MB (bench-tool) + ~40GB (workspace). **Always check `df -h` before running.**
- If disk fills up, the shell environment will break (no stdout). Recovery requires manual cleanup and session restart.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Bash tool returns exit 1 with no output | Disk full (`df -h /` shows 100%) | Free space, restart session |
| `unlink` takes minutes | Deferred delete + delta compaction in RucksFS | Expected behavior; reduce `-n` |
| `create` slow at high threads | RocksDB write contention | Expected; compare easy vs hard |
| `readdir` slow on RucksFS | Full directory scan through FUSE | Expected; FUSE adds per-entry overhead |
| Panic on FUSE mount | Mount point not empty or stale | `fusermount -u <path>`, recreate dir |
