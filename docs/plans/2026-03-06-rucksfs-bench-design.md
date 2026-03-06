# rucksfs-bench: Rust Metadata Benchmark Tool (T-30)

> Date: 2026-03-06
> Status: Approved

## Overview

A Rust-native metadata benchmark tool that benchmarks FUSE filesystem metadata operations
through direct syscalls. Designed to parallel mdtest (the standard HPC metadata benchmark)
in methodology, enabling direct performance comparison for academic evaluation.

**Positioning:**
- pjdfstest = POSIX correctness verification
- rucksfs-bench = concurrent metadata performance measurement (mdtest counterpart)

## CLI Interface

```
rucksfs-bench [OPTIONS] <COMMAND>

Options:
  -m, --mountpoint <PATH>    FUSE mount point (required)
  -t, --threads <LIST>       Thread count list, e.g. "1,2,4,8" (default: "1")
  -n, --num-files <N>        Files per thread (default: 10000)
  -o, --output <DIR>         CSV output directory (default: ./results)

Commands:
  create    File creation (open O_CREAT + close)
  stat      File stat
  unlink    File deletion
  mkdir     Directory creation
  rmdir     Directory deletion
  readdir   Directory listing
  rename    File rename
  all       Run all operations sequentially
```

Each subcommand supports:
- `--mode easy|hard` (default: easy)
  - **easy**: each thread operates in its own subdirectory (`/bench/thread-0/`, `/bench/thread-1/`...)
  - **hard**: all threads share a single directory (`/bench/shared/`)

## Operations

| Operation | Syscall | Setup Required | Cleanup |
|-----------|---------|----------------|---------|
| create | `open(O_CREAT \| O_WRONLY) + close` | Create parent dirs | Remove created files |
| stat | `stat()` | Create files first | Remove files |
| unlink | `unlink()` | Create files first | None (files removed by test) |
| mkdir | `mkdir()` | Create parent dirs | Remove created dirs |
| rmdir | `rmdir()` | Create dirs first | None (dirs removed by test) |
| readdir | `opendir + readdir + closedir` | Create dirs with files | Remove dirs and files |
| rename | `rename()` | Create source files | Remove renamed files |

### Operation Chains (`all` mode)

Two independent chains run in sequence:

1. **File chain**: create -> stat -> rename -> unlink
2. **Directory chain**: mkdir -> readdir -> rmdir

Within each chain, earlier operations set up files/dirs for later ones,
avoiding redundant setup/cleanup overhead.

## Test Modes

### mdtest-easy

Each thread operates in an isolated subdirectory:
```
/mountpoint/bench/thread-0/file-0000, file-0001, ...
/mountpoint/bench/thread-1/file-0000, file-0001, ...
```
No contention between threads. Measures maximum parallelism.

### mdtest-hard

All threads operate in a shared directory:
```
/mountpoint/bench/shared/t0-file-0000, t0-file-0001, ...
/mountpoint/bench/shared/t1-file-0000, t1-file-0001, ...
```
High contention on directory metadata. Measures lock/transaction scalability.

## Execution Flow

```
1. Main thread: setup(config)           // Create directory structure / pre-populate files
2. Spawn N threads, each holds Arc<Barrier>
3. barrier.wait()                        // All threads start simultaneously
4. Each thread: for i in 0..num_files { syscall(file_i); }
5. Each thread: record Instant elapsed
6. Main thread: join all, collect ThreadResult
7. Compute aggregate metrics
8. cleanup(config)                       // Remove test files/dirs
```

### Metrics Calculation

- **ops_per_sec** = `total_ops / max(thread_elapsed)` (mdtest convention: wall-clock throughput)
- **avg_latency_us** = `sum(thread_elapsed) / total_ops * 1_000_000`
- **scaling_efficiency** = `(ops_N / (ops_1 * N)) * 100%`

## Architecture

```
benchmark/bench-tool/
├── Cargo.toml
└── src/
    ├── main.rs          # CLI parsing (clap derive), entry point
    ├── runner.rs         # BenchRunner: thread orchestration, barrier, timing
    ├── ops.rs            # 7 operations as pure std::fs syscalls
    ├── report.rs         # CSV writing + terminal table output
    └── setup.rs          # Test directory creation / cleanup
```

### Key Types

```rust
struct BenchConfig {
    mountpoint: PathBuf,
    op: BenchOp,              // Create | Stat | Unlink | Mkdir | Rmdir | Readdir | Rename
    mode: BenchMode,          // Easy | Hard
    num_threads: usize,
    num_files_per_thread: usize,
}

struct ThreadResult {
    thread_id: usize,
    ops_completed: u64,
    elapsed: Duration,
}

struct BenchResult {
    config: BenchConfig,
    thread_results: Vec<ThreadResult>,
    total_ops: u64,
    total_elapsed: Duration,
    ops_per_sec: f64,
    avg_latency_us: f64,
}
```

## Output Format

### CSV (`results/<timestamp>_<op>.csv`)

```csv
timestamp,operation,mode,num_threads,num_files_per_thread,total_ops,duration_sec,ops_per_sec,avg_latency_us
20260306_143000,create,easy,4,10000,40000,2.345,17057.57,58.63
```

### Terminal Summary

```
RucksFS Benchmark Results
=========================
Mountpoint: /mnt/rucksfs
Date: 2026-03-06 14:30:00

Operation  Mode  Threads  Files/T  Total Ops  Duration(s)  Ops/s      Avg Lat(us)
---------  ----  -------  -------  ---------  -----------  ---------  -----------
create     easy  4        10000    40000      2.345        17057.57   58.63
create     hard  4        10000    40000      5.678        7044.03    141.96
stat       easy  4        10000    40000      0.892        44843.05   22.30
...
```

### Scaling Analysis (when --threads has multiple values)

```
Scaling Analysis (create, easy mode):
Threads  Ops/s      Efficiency
1        5000.00    100.0%
2        9500.00    95.0%
4        17000.00   85.0%
8        28000.00   70.0%
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| clap (derive) | CLI argument parsing |
| libc | Low-level syscall access if needed |
| csv | CSV file writing |
| chrono | Timestamp formatting |

No heavy dependencies (no tokio, serde, etc.).
Timing via `std::time::Instant`, file ops via `std::fs`, synchronization via `std::sync::Barrier`.

## Build & Run

```bash
# Build
cd benchmark/bench-tool && cargo build --release

# Run single operation
./target/release/rucksfs-bench -m /mnt/rucksfs -t 1,2,4,8 -n 10000 create --mode easy

# Run all operations
./target/release/rucksfs-bench -m /mnt/rucksfs -t 1,2,4,8 -n 10000 all
```

Not a workspace member — built independently to avoid slowing down main project compilation.

## Integration

- `benchmark/run_all.sh` can invoke `rucksfs-bench all` for metadata benchmarks
- `testing/Taskfile.yml` can add a `bench:rust` task
- Correctness testing remains with pjdfstest + existing shell scripts

## Concurrency Model

**std::thread** (not async/tokio), because:
1. `tokio::fs` wraps sync syscalls in `spawn_blocking` — not true async I/O
2. Async scheduling adds uncontrollable queuing latency, polluting timing
3. Direct thread-per-worker maps cleanly to mdtest's MPI rank model
4. Simpler code, easier to reason about performance characteristics
