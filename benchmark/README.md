# RucksFS Benchmark & Test Suite

> Comprehensive correctness and performance evaluation for RucksFS.
>
> **Agent-friendly**: All scripts accept `--mountpoint <path>` as the only
> required argument. Exit code 0 = pass, non-zero = fail. Results are written
> to `benchmark/results/` as machine-parseable CSV/JSON.

---

## Academic References

This benchmark suite is designed with reference to the evaluation methodologies
used in the following file-system research papers:

| Paper | Venue | Evaluation Focus | What We Adopted |
|-------|-------|-----------------|-----------------|
| **TableFS** (Ren & Gibson, 2013) | USENIX ATC '13 | Metadata IOPS: 1M file create/stat/delete on ext4 vs LevelDB | `metadata_ops.sh` — single-dir and multi-dir create/stat/delete microbenchmarks |
| **SingularFS** (Liu et al., 2023) | USENIX ATC '23 | Billion-scale create/stat, NUMA-aware throughput | `metadata_ops.sh` — scalable file count, per-op latency histograms |
| **LocoFS** (Li et al., 2017) | SC '17 | Decoupled metadata KV utilization, readdir + getattr batch | `metadata_ops.sh` — readdir-then-stat pipeline latency |
| **BFO** (Yang et al., 2020) | ACM TOS | Batch file operations, small-file I/O amplification | `io_throughput.sh` — small-file sequential create-write-read-delete |
| **IO500 / mdtest** | SC BoF | MDEasy (private dirs), MDHard (shared dir) | `metadata_ops.sh` — both private-dir and shared-dir modes |
| **filebench** | Industry std. | Composite workloads (fileserver, webserver, varmail) | `io_throughput.sh` — mixed R/W workloads with configurable ratios |
| **pjdfstest** | POSIX compliance | 8,800+ POSIX syscall correctness tests | `run_pjdfstest.sh` — automated pjdfstest execution |

---

## Directory Layout

```
benchmark/
├── README.md                          # This file
├── run_all.sh                         # One-command entry: run everything
├── correctness/
│   ├── run_pjdfstest.sh               # POSIX compliance via pjdfstest
│   └── posix_conformance.sh           # Custom POSIX semantics tests
├── performance/
│   ├── metadata_ops.sh                # Metadata IOPS microbenchmark
│   ├── io_throughput.sh               # Data I/O throughput & latency
│   └── concurrent_stress.sh           # Concurrent / scalability stress
└── results/                           # Auto-generated output directory
    ├── *.csv                          # Machine-parseable results
    └── *.log                          # Human-readable logs
```

---

## Prerequisites

### Required

- **Linux** with FUSE support (`/dev/fuse` accessible)
- **RucksFS** mounted at a known path (e.g., `/mnt/rucksfs`)
- **bash** ≥ 4.0, **bc**, **time**, **stat**, **md5sum**

### Optional (for extended tests)

| Tool | Purpose | Install |
|------|---------|---------|
| **pjdfstest** | POSIX compliance (8,800+ tests) | `git clone https://github.com/saidsay-so/pjdfstest && cd pjdfstest && cargo build` |
| **fio** | I/O throughput micro-bench | `apt install fio` |
| **filebench** | Composite workloads | `apt install filebench` |
| **mdtest** (from IOR) | Standardized metadata bench | `apt install ior` or build from source |

---

## Quick Start

```bash
# 1. Mount RucksFS
cargo run -p rucksfs -- --mount /mnt/rucksfs --data-dir /tmp/rucksfs_bench_data

# 2. Run all benchmarks (correctness + performance)
./benchmark/run_all.sh --mountpoint /mnt/rucksfs

# 3. Run only correctness tests
./benchmark/correctness/posix_conformance.sh --mountpoint /mnt/rucksfs
./benchmark/correctness/run_pjdfstest.sh --mountpoint /mnt/rucksfs

# 4. Run only performance benchmarks
./benchmark/performance/metadata_ops.sh --mountpoint /mnt/rucksfs
./benchmark/performance/io_throughput.sh --mountpoint /mnt/rucksfs
./benchmark/performance/concurrent_stress.sh --mountpoint /mnt/rucksfs

# 5. View results
ls benchmark/results/
cat benchmark/results/metadata_ops_*.csv
```

---

## Test Categories

### 1. Correctness: POSIX Conformance (`correctness/posix_conformance.sh`)

Self-contained POSIX semantics tests covering all 15+ file operations.
No external dependencies required.

**Test Suites:**

| Suite | Operations Tested | Reference |
|-------|-------------------|-----------|
| S1: Basic CRUD | create, write, read, stat, unlink | TableFS §5.1 |
| S2: Directory Ops | mkdir, rmdir, readdir, nested dirs | TableFS §5.1, LocoFS §6.2 |
| S3: Rename Semantics | same-dir, cross-dir, overwrite, POSIX atomicity | SingularFS §5.3 |
| S4: Metadata Consistency | chmod, chown, utimens, nlink tracking | POSIX.1-2017 |
| S5: Edge Cases | long filenames, special chars, empty files, sparse files | pjdfstest |
| S6: Error Semantics | ENOENT, EEXIST, ENOTDIR, ENOTEMPTY, EISDIR | POSIX.1-2017 |
| S7: Hard Links | link, nlink count, unlink last link | POSIX.1-2017 |
| S8: Symlinks | symlink, readlink, dangling symlinks | POSIX.1-2017 |
| S9: Persistence | write → unmount → remount → read verification | SingularFS §5.5 |
| S10: statfs | filesystem statistics validity | POSIX.1-2017 |

### 2. Correctness: pjdfstest (`correctness/run_pjdfstest.sh`)

Runs the industry-standard pjdfstest suite (8,800+ POSIX compliance tests)
against the mounted filesystem. Requires pjdfstest to be installed.

### 3. Performance: Metadata Operations (`performance/metadata_ops.sh`)

Microbenchmarks for metadata-intensive workloads, directly inspired by
TableFS, SingularFS, and mdtest evaluation methodologies.

**Benchmarks:**

| Bench | Description | Metric | Reference |
|-------|-------------|--------|-----------|
| B1: File Create | Create N empty files in a single directory | ops/sec | TableFS §5.2, mdtest-hard |
| B2: File Create (Multi-Dir) | Create N files spread across M directories | ops/sec | mdtest-easy |
| B3: File Stat | stat() each of N files | ops/sec | TableFS §5.2, SingularFS §6.1 |
| B4: File Delete | Delete N files | ops/sec | TableFS §5.2 |
| B5: mkdir | Create N directories | ops/sec | SingularFS §6.2 |
| B6: readdir | List directory with N entries | entries/sec | LocoFS §6.3 |
| B7: readdir + stat | readdir then stat each entry (ls -l pattern) | ops/sec | LocoFS §6.3, BFO §4 |
| B8: Rename | Rename N files | ops/sec | SingularFS §6.3 |
| B9: Mixed Workload | 50% create + 30% stat + 20% delete | ops/sec | IO500 composite |
| B10: Deep Tree | Create/traverse 100-level deep directory tree | latency(ms) | SingularFS §6.4 |

**Configurable Parameters:**
- `--num-files N` (default: 10000)
- `--num-dirs M` (default: 100)
- `--depth D` (default: 50)

### 4. Performance: I/O Throughput (`performance/io_throughput.sh`)

Data path performance benchmarks.

| Bench | Description | Metric | Reference |
|-------|-------------|--------|-----------|
| T1: Sequential Write | Write large file (configurable size) | MB/s | fio seqwrite |
| T2: Sequential Read | Read large file | MB/s | fio seqread |
| T3: Random Write 4K | Random 4K writes | IOPS | fio randwrite |
| T4: Random Read 4K | Random 4K reads | IOPS | fio randread |
| T5: Small File Pipeline | Create → write(4K) → close → open → read → delete | ops/sec | BFO §4.1 |
| T6: Append Workload | Repeated append to same file | MB/s | filebench varmail |
| T7: Data Integrity | Write → checksum → read → verify | pass/fail | — |
| T8: Overwrite | Overwrite portions of existing file | MB/s | fio randrw |

### 5. Performance: Concurrent Stress (`performance/concurrent_stress.sh`)

Scalability and concurrency tests inspired by SingularFS and LocoFS.

| Bench | Description | Metric | Reference |
|-------|-------------|--------|-----------|
| C1: Concurrent Create | N processes each create M files in private dirs | agg ops/sec | SingularFS §6.1 |
| C2: Shared Dir Create | N processes create files in same directory | agg ops/sec | mdtest-hard |
| C3: Concurrent Read/Write | Mixed concurrent readers and writers | ops/sec | SingularFS §6.5 |
| C4: Create-Delete Storm | Concurrent create + immediate delete | ops/sec | — |
| C5: Concurrent Rename | N processes rename different files simultaneously | ops/sec | SingularFS §6.3 |
| C6: Thread Scaling | Run B1 with 1,2,4,8,16 threads, measure scaling | scaling factor | SingularFS §6.6 |
| C7: Lock Contention | All threads operate on same directory | ops/sec | SingularFS §6.2 |

---

## Output Format

### CSV Output

All performance benchmarks produce CSV files in `benchmark/results/`:

```csv
timestamp,benchmark,variant,num_files,num_threads,ops_total,duration_sec,ops_per_sec,latency_avg_us,latency_p99_us
2026-03-04T18:00:00,metadata_ops,file_create_single_dir,10000,1,10000,2.34,4273,234,890
```

### Correctness Output

Correctness tests produce structured logs:

```
[PASS] S1.01: create regular file
[PASS] S1.02: write and read back content
[FAIL] S7.01: hard link nlink count (expected=2, got=1)
---
Results: 42 passed, 1 failed, 0 skipped, 43 total
```

---

## Comparing with Other Systems

To compare RucksFS with ext4/tmpfs (as described in TableFS and SingularFS papers):

```bash
# Test against ext4
./benchmark/performance/metadata_ops.sh --mountpoint /tmp/ext4_test

# Test against tmpfs
./benchmark/performance/metadata_ops.sh --mountpoint /dev/shm/tmpfs_test

# Test against RucksFS
./benchmark/performance/metadata_ops.sh --mountpoint /mnt/rucksfs
```

Then compare the CSV files side by side. A helper script for generating
comparison tables may be added in the future.

---

## Forward-Looking Test Coverage

Some tests cover features not yet implemented in RucksFS. These will report
`[SKIP]` rather than `[FAIL]` when the operation returns `ENOSYS` or
`EOPNOTSUPP`. This allows tracking implementation progress:

| Feature | Test Coverage | Current Status |
|---------|--------------|----------------|
| Hard links (`link`) | S7, C5 | ❌ Not implemented |
| Symbolic links (`symlink`/`readlink`) | S8 | ❌ Not implemented |
| Extended attributes (`xattr`) | — | ❌ Not implemented |
| File locking (`flock`/`fcntl`) | — | ❌ Not implemented |
| mmap | — | ❌ Not implemented |
| Deferred unlink (open handle) | S6.07 | ❌ Not implemented |
| Truncate | S4.05, T8 | ✅ Implemented |
| chmod/chown | S4.01-S4.04 | ✅ Implemented |
| Persistence across remount | S9 | ✅ Implemented |

---

## Adding New Tests

### For agents

1. Create a new `.sh` file in the appropriate subdirectory.
2. Accept `--mountpoint <path>` as the first argument.
3. Source `../lib/bench_helpers.sh` (if created) for shared utilities.
4. Output results to `benchmark/results/<test_name>_<timestamp>.csv`.
5. Exit 0 on success, non-zero on failure.
6. Update this README with the new test description.

### Test naming convention

```
benchmark/{correctness,performance}/<category>_<focus>.sh
```

---

## Known Limitations

- All benchmarks are single-machine. Distributed benchmarks (multi-client
  mdtest, etc.) are out of scope for the standalone RucksFS mode.
- FUSE overhead is inherent in all measurements. For apples-to-apples
  comparison with kernel filesystems, account for ~10-50μs per-op FUSE
  context-switch overhead.
- pjdfstest requires root access for full coverage (uid/gid switching tests).
