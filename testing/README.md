# RucksFS Automated Remote Testing

Taskfile-based automated testing framework for RucksFS. Handles the full
lifecycle: **build -> upload -> mount FUSE -> run benchmarks -> collect results**
on a remote test host via SSH.

> **Agent-First Design**: All tasks are designed for AI agent execution — short-lived
> SSH calls, structured output, clear exit codes, and async benchmark support.

## Prerequisites

| Tool | Purpose |
|------|---------|
| [Task](https://taskfile.dev/installation/) | Task runner (v3+) |
| [yq](https://github.com/mikefarah/yq) | YAML parser (optional, falls back to grep/awk) |
| Rust toolchain | `cargo build --release -p rucksfs` |
| SSH key access | Passwordless SSH to remote host |

The **remote host** must have:

- Linux with FUSE support (`/dev/fuse`)
- `fusermount` or `fusermount3`
- `bash` >= 4.0, `bc`, `stat`, `md5sum`, `dd`
- Root access (for FUSE mount with `AllowOther`)

## Quick Start

```bash
cd testing
cp env.example.yml env.yml
vim env.yml          # fill in host, SSH key, etc.
task test            # Full automated test (sync, blocks until done)
```

---

## Configuration

Copy `env.example.yml` to `env.yml` and edit:

| Section | Key Fields | Description |
|---------|------------|-------------|
| `remote` | `host`, `user`, `ssh_key`, `port` | SSH connection to the test machine |
| `rucksfs` | `mountpoint`, `data_dir` | FUSE mount point and RocksDB data directory |
| `benchmark` | `mode`, `num_files`, `max_threads`, ... | Benchmark parameters |

### Key Benchmark Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `benchmark.mode` | `all` | `all`, `correctness`, or `performance` |
| `benchmark.num_files` | `1000` | Files per metadata benchmark (B1-B9) |
| `benchmark.num_dirs` | `50` | Directories for multi-dir tests (B2, B5) |
| `benchmark.max_threads` | `4` | Max concurrency for stress tests (C1-C7) |
| `benchmark.large_size_mb` | `16` | Large file size for I/O tests (T1-T2) |
| `benchmark.small_count` | `1000` | Small files for pipeline test (T5) |
| `benchmark.skip_pjdfstest` | `true` | Skip pjdfstest (requires separate install) |

---

## Available Tasks

List all tasks: `task --list`

### Composite Workflows

| Task | Mode | Description |
|------|:----:|-------------|
| `test` | sync | Full flow: validate -> build -> deploy -> check -> mount -> bench -> collect |
| `test:agent` | **async** | Same but launches benchmark in background, returns immediately |
| `quick-bench` | sync | Re-run: clean -> restart -> bench -> collect (skip build/deploy) |
| `quick-bench:agent` | **async** | Same but async |
| `clean-bench:agent` | **async** | Deep-clean + rotate dirs + reboot -> configure -> bench |

### Build Tasks

| Task | Description |
|------|-------------|
| `build` | Build `rucksfs` release binary locally (`cargo build --release -p rucksfs`) |

### Deploy Tasks

| Task | Description |
|------|-------------|
| `deploy` | Full deploy: upload + install + configure |
| `deploy:upload` | Upload binary, benchmark scripts, and helper scripts to remote |
| `deploy:install` | Install rucksfs binary to `/usr/local/bin` on remote |
| `deploy:configure` | Configure FUSE, create mount/data directories |

### Benchmark Tasks

| Task | Description |
|------|-------------|
| `bench:start` | **[Agent]** Launch benchmark in background, return immediately with PID |
| `bench:status` | **[Agent]** Check progress. Exit `0` = done, exit `1` = running |
| `bench:stop` | **[Agent]** Stop metrics collection + collect logs |
| `bench:run` | [Sync] Run benchmark blocking until complete |
| `bench` | Alias for `bench:run` |
| `collect` | Pull results (CSV + logs + metrics) to local `results/` |

### Cleanup Tasks

| Task | Description |
|------|-------------|
| `clean` | Standard cleanup: stop -> unmount -> remove data -> clean temp |
| `clean:full-rotate` | **Deep cleanup**: standard + rotate data dirs + reboot |
| `clean:data` | Remove RucksFS data directory |
| `clean:mounts` | Unmount FUSE mount |
| `clean:remote-data` | Remove remote temp files and benchmark results |
| `clean:rotate-dirs` | Rotate data directory (timestamped backup) |
| `clean:reboot` | Reboot remote host + wait for recovery |

### Service Management (FUSE Mount)

| Task | Description |
|------|-------------|
| `service:start` | Mount RucksFS via FUSE on remote |
| `service:stop` | Unmount and stop rucksfs process |
| `service:restart` | Stop + start |
| `service:status` | Show mount status, process info, disk usage |

### Utility

| Task | Description |
|------|-------------|
| `validate` | Check `env.yml` exists and print config summary |
| `check` | Run environment checks (FUSE, tools, disk) |
| `ssh` | Open interactive SSH session |
| `logs` | Show recent RucksFS application logs |

---

## Agent Testing Workflows

### Workflow 1: First-Time Full Test

Use this when deploying from scratch or after code changes.

```
# Step 1: Build + Deploy + Configure
task build
task deploy
task check

# Step 2: Start RucksFS FUSE mount
task service:start

# Step 3: Launch benchmark (returns immediately)
task bench:start
# Output: "benchmark_started pid=<PID>"

# Step 4: Poll until complete (every 10-15 seconds)
task bench:status
# Exit code 1 -> still running, wait and retry
# Exit code 0 -> finished, proceed to step 5

# Step 5: Collect results
task bench:stop
task collect
```

**Or as a single command** (steps 1-3 combined):
```
task test:agent
# Then poll: task bench:status
# Then finalize: task bench:stop && task collect
```

### Workflow 2: Quick Re-Run (Skip Build/Deploy)

Use this to re-run benchmark with different parameters.

```
task clean:remote-data
task service:restart
task bench:start
task bench:status        # poll until exit 0
task bench:stop && task collect
```

**Or**: `task quick-bench:agent` then poll + collect.

### Workflow 3: Deep Clean + Benchmark (Pristine State)

Use this for **accurate benchmarking**. Ensures no leftover data.

```
task clean:full-rotate   # deep cleanup + reboot
task deploy:configure    # re-configure
task service:restart
task bench:start
task bench:status        # poll until exit 0
task bench:stop && task collect
```

**Or**: `task clean-bench:agent` then poll + collect.

### Workflow 4: Change Parameters

```
# Edit env.yml (e.g., change benchmark.num_files to 10000)
# Then use Workflow 2 or 3
task quick-bench:agent   # or task clean-bench:agent
task bench:status        # poll
task bench:stop && task collect
```

---

## `bench:status` Protocol

| Exit Code | Meaning | Agent Action |
|:---------:|---------|-------------|
| `0` | Benchmark finished | Proceed to `bench:stop` -> `collect` |
| `1` | Still running | Wait 10-15 seconds, poll again |

### Structured Output

```
status=running|finished|no_benchmark
pid=<PID>
elapsed=<HH:MM:SS>
```

---

## Benchmark Suites

The framework runs the benchmark scripts from `benchmark/` on the remote host:

### Correctness Tests
- **POSIX Conformance** (`posix_conformance.sh`): 10 test suites (S1-S10) covering file CRUD, directories, rename, metadata, edge cases, error semantics, hard links, symlinks, persistence, and statfs
- **pjdfstest** (`run_pjdfstest.sh`): 8,800+ POSIX compliance tests (optional, requires installation)

### Performance Benchmarks
- **Metadata Ops** (`metadata_ops.sh`): B1-B11 covering create, stat, delete, mkdir, readdir, rename, mixed workload, deep tree traversal
- **I/O Throughput** (`io_throughput.sh`): T1-T8 covering sequential/random read/write, small file pipeline, append, integrity, overwrite
- **Concurrent Stress** (`concurrent_stress.sh`): C1-C7 covering concurrent create (private/shared dir), mixed R/W, create-delete storm, rename, thread scaling, lock contention

### Output Format
- CSV files in `results/run_<timestamp>/benchmark/`
- Human-readable logs alongside CSV
- System metrics in `results/run_<timestamp>/metrics/`
- Application/kernel logs in `results/run_<timestamp>/logs/`

---

## Cleanup Strategy

### Standard (`clean`)
- Stop rucksfs process -> unmount FUSE -> remove data directory -> clean temp files

### Deep (`clean:full-rotate`)
- Standard + rotate data directory (timestamped backup) + reboot machine
- Guarantees: no page cache, no stale mounts, fresh database

---

## Directory Layout

```
testing/
├── Taskfile.yml          # Main automation
├── env.example.yml       # Configuration template
├── env.yml               # Local config (git-ignored)
├── .gitignore            # Ignores env.yml and results/
├── scripts/
│   ├── remote-setup.sh   # Remote: install, configure, check
│   ├── collect-metrics.sh # Remote: system metrics (vmstat, iostat)
│   └── collect-logs.sh   # Remote: log capture (dmesg, journal)
├── results/              # Test results (git-ignored)
│   └── run_YYYYMMDD_HHMMSS/
│       ├── benchmark/    # CSV + log files from benchmark scripts
│       ├── metrics/      # vmstat, iostat, mpstat, top logs
│       └── logs/         # rucksfs.log, dmesg, journal
└── README.md             # This file
```

---

## Architecture

```
┌──────────────────────────────────────────────────┐
│                  Local Machine                    │
│                                                   │
│  task build       -> cargo build --release        │
│  task deploy      -> SCP binary + scripts         │
│  task service:start -> SSH start rucksfs FUSE     │
│  task bench:start -> SSH (~2s, returns PID)       │
│  task bench:status -> SSH (~1s, exit 0/1)         │
│  task bench:stop  -> SSH (~5s, stop metrics)      │
│  task collect     -> SCP results to local         │
└──────────────────────────────────────────────────┘
          │ SSH
          ▼
┌──────────────────────────────────────────────────┐
│                  Remote Host                      │
│                                                   │
│  /usr/local/bin/rucksfs     (binary)              │
│  /mnt/rucksfs               (FUSE mountpoint)     │
│  /var/lib/rucksfs           (RocksDB + RawDisk)   │
│  /opt/rucksfs-test/         (work dir)            │
│  /opt/rucksfs-test/benchmark/ (benchmark scripts) │
└──────────────────────────────────────────────────┘
```

## Design Principles

- **File-based configuration** -- all params in `env.yml`, no interactive prompts
- **Idempotent tasks** -- safe to re-run any task
- **Clear exit codes** -- non-zero on failure
- **Structured output** -- CSV results, key=value status
- **Async benchmark** -- no long-running SSH; all calls return within seconds
- **Deep cleanup** -- rotate data dirs + reboot for pristine state
- **Reuses existing benchmarks** -- leverages the `benchmark/` directory scripts directly
