# rucksfs-bench Implementation Plan (T-30)

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Rust-native metadata benchmark CLI tool that parallels mdtest methodology for academic performance evaluation of RucksFS.

**Architecture:** Independent binary crate at `benchmark/bench-tool/`. Uses `std::thread` for concurrency, `std::sync::Barrier` for synchronized start, `std::time::Instant` for timing. CLI via `clap` derive, output via `csv` crate + manual terminal formatting.

**Tech Stack:** Rust, clap (derive), csv, chrono, std::fs, std::thread, std::sync::Barrier

**Design doc:** `docs/plans/2026-03-06-rucksfs-bench-design.md`

---

### Task 1: Project Scaffold + CLI Skeleton

**Files:**
- Create: `benchmark/bench-tool/Cargo.toml`
- Create: `benchmark/bench-tool/src/main.rs`

**Step 1: Create Cargo.toml**

```toml
[package]
name = "rucksfs-bench"
version = "0.1.0"
edition = "2021"
publish = false

[[bin]]
name = "rucksfs-bench"
path = "src/main.rs"

[dependencies]
clap = { version = "4", features = ["derive"] }
csv = "1"
chrono = "0.4"
```

**Step 2: Create main.rs with CLI parsing**

Define all types and CLI structure using clap derive:

```rust
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rucksfs-bench", about = "Metadata benchmark tool for FUSE filesystems")]
struct Cli {
    /// FUSE mount point path
    #[arg(short, long)]
    mountpoint: PathBuf,

    /// Thread count list, comma-separated (e.g. "1,2,4,8")
    #[arg(short, long, default_value = "1", value_delimiter = ',')]
    threads: Vec<usize>,

    /// Number of files/dirs per thread
    #[arg(short, long, default_value = "10000")]
    num_files: usize,

    /// CSV output directory
    #[arg(short, long, default_value = "results")]
    output: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Clone)]
enum Command {
    Create { #[arg(long, default_value = "easy")] mode: BenchMode },
    Stat   { #[arg(long, default_value = "easy")] mode: BenchMode },
    Unlink { #[arg(long, default_value = "easy")] mode: BenchMode },
    Mkdir  { #[arg(long, default_value = "easy")] mode: BenchMode },
    Rmdir  { #[arg(long, default_value = "easy")] mode: BenchMode },
    Readdir { #[arg(long, default_value = "easy")] mode: BenchMode },
    Rename { #[arg(long, default_value = "easy")] mode: BenchMode },
    All,
}

#[derive(Clone, Copy, ValueEnum)]
enum BenchMode {
    Easy,
    Hard,
}

fn main() {
    let cli = Cli::parse();
    // Validate mountpoint exists
    if !cli.mountpoint.is_dir() {
        eprintln!("Error: mountpoint '{}' does not exist or is not a directory", cli.mountpoint.display());
        std::process::exit(1);
    }
    println!("rucksfs-bench: mountpoint={}, threads={:?}, num_files={}",
             cli.mountpoint.display(), cli.threads, cli.num_files);
}
```

**Step 3: Verify it compiles**

Run: `cd /data/workspace/rucksfs/benchmark/bench-tool && cargo build`
Expected: Compiles successfully, prints help with `cargo run -- --help`

**Step 4: Commit**

```
feat(bench): scaffold rucksfs-bench crate with CLI skeleton
```

---

### Task 2: Core Types + Runner Module

**Files:**
- Create: `benchmark/bench-tool/src/runner.rs`
- Modify: `benchmark/bench-tool/src/main.rs` (add `mod runner;`)

**Step 1: Create runner.rs with core types and BenchRunner**

```rust
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub enum BenchOp {
    Create,
    Stat,
    Unlink,
    Mkdir,
    Rmdir,
    Readdir,
    Rename,
}

impl std::fmt::Display for BenchOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchOp::Create => write!(f, "create"),
            BenchOp::Stat => write!(f, "stat"),
            BenchOp::Unlink => write!(f, "unlink"),
            BenchOp::Mkdir => write!(f, "mkdir"),
            BenchOp::Rmdir => write!(f, "rmdir"),
            BenchOp::Readdir => write!(f, "readdir"),
            BenchOp::Rename => write!(f, "rename"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BenchMode {
    Easy,
    Hard,
}

impl std::fmt::Display for BenchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchMode::Easy => write!(f, "easy"),
            BenchMode::Hard => write!(f, "hard"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BenchConfig {
    pub mountpoint: PathBuf,
    pub op: BenchOp,
    pub mode: BenchMode,
    pub num_threads: usize,
    pub num_files_per_thread: usize,
}

pub struct ThreadResult {
    pub thread_id: usize,
    pub ops_completed: u64,
    pub elapsed: Duration,
}

pub struct BenchResult {
    pub config: BenchConfig,
    pub thread_results: Vec<ThreadResult>,
    pub total_ops: u64,
    pub total_elapsed: Duration,
    pub ops_per_sec: f64,
    pub avg_latency_us: f64,
}

impl BenchConfig {
    /// Base directory for this benchmark run under the mountpoint
    pub fn bench_dir(&self) -> PathBuf {
        self.mountpoint.join("bench")
    }

    /// Working directory for a specific thread
    pub fn thread_dir(&self, thread_id: usize) -> PathBuf {
        match self.mode {
            BenchMode::Easy => self.bench_dir().join(format!("thread-{}", thread_id)),
            BenchMode::Hard => self.bench_dir().join("shared"),
        }
    }

    /// File path for a specific thread and file index
    pub fn file_path(&self, thread_id: usize, file_idx: usize) -> PathBuf {
        let dir = self.thread_dir(thread_id);
        match self.mode {
            BenchMode::Easy => dir.join(format!("file-{:06}", file_idx)),
            BenchMode::Hard => dir.join(format!("t{}-file-{:06}", thread_id, file_idx)),
        }
    }

    /// Directory path for mkdir/rmdir benchmarks
    pub fn dir_path(&self, thread_id: usize, dir_idx: usize) -> PathBuf {
        let dir = self.thread_dir(thread_id);
        match self.mode {
            BenchMode::Easy => dir.join(format!("dir-{:06}", dir_idx)),
            BenchMode::Hard => dir.join(format!("t{}-dir-{:06}", thread_id, dir_idx)),
        }
    }
}

/// Run a benchmark: setup -> barrier -> timed work -> collect results -> cleanup
pub fn run_bench<S, W, C>(
    config: &BenchConfig,
    setup: S,
    work: W,
    cleanup: C,
) -> BenchResult
where
    S: FnOnce(&BenchConfig),
    W: Fn(&BenchConfig, usize) -> u64 + Send + Sync + 'static,
    C: FnOnce(&BenchConfig),
{
    setup(config);

    let barrier = Arc::new(Barrier::new(config.num_threads));
    let work = Arc::new(work);
    let config_clone = config.clone();

    let handles: Vec<_> = (0..config.num_threads)
        .map(|tid| {
            let barrier = Arc::clone(&barrier);
            let work = Arc::clone(&work);
            let cfg = config_clone.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let start = Instant::now();
                let ops = work(&cfg, tid);
                let elapsed = start.elapsed();
                ThreadResult {
                    thread_id: tid,
                    ops_completed: ops,
                    elapsed,
                }
            })
        })
        .collect();

    let thread_results: Vec<ThreadResult> = handles
        .into_iter()
        .map(|h| h.join().expect("worker thread panicked"))
        .collect();

    let total_ops: u64 = thread_results.iter().map(|r| r.ops_completed).sum();
    let max_elapsed = thread_results
        .iter()
        .map(|r| r.elapsed)
        .max()
        .unwrap_or(Duration::ZERO);
    let sum_elapsed: Duration = thread_results.iter().map(|r| r.elapsed).sum();

    let total_secs = max_elapsed.as_secs_f64();
    let ops_per_sec = if total_secs > 0.0 { total_ops as f64 / total_secs } else { 0.0 };
    let avg_latency_us = if total_ops > 0 {
        sum_elapsed.as_secs_f64() / total_ops as f64 * 1_000_000.0
    } else {
        0.0
    };

    cleanup(config);

    BenchResult {
        config: config.clone(),
        thread_results,
        total_ops,
        total_elapsed: max_elapsed,
        ops_per_sec,
        avg_latency_us,
    }
}
```

**Step 2: Add `mod runner;` to main.rs**

Add `mod runner;` at the top of main.rs.

**Step 3: Verify it compiles**

Run: `cd /data/workspace/rucksfs/benchmark/bench-tool && cargo build`
Expected: Compiles with no errors

**Step 4: Commit**

```
feat(bench): add core types and thread-orchestrated BenchRunner
```

---

### Task 3: Setup + Cleanup Module

**Files:**
- Create: `benchmark/bench-tool/src/setup.rs`
- Modify: `benchmark/bench-tool/src/main.rs` (add `mod setup;`)

**Step 1: Create setup.rs**

Functions to create/remove test directory structures. Used before and after each benchmark.

```rust
use crate::runner::{BenchConfig, BenchOp};
use std::fs;

/// Create the directory structure needed before a benchmark.
/// For create/mkdir: just the parent dirs.
/// For stat/unlink/rename/readdir/rmdir: pre-populate files or dirs.
pub fn setup(config: &BenchConfig) {
    // Always ensure bench base dir exists
    let bench_dir = config.bench_dir();
    fs::create_dir_all(&bench_dir).expect("failed to create bench dir");

    match config.mode {
        crate::runner::BenchMode::Easy => {
            for tid in 0..config.num_threads {
                fs::create_dir_all(config.thread_dir(tid))
                    .expect("failed to create thread dir");
            }
        }
        crate::runner::BenchMode::Hard => {
            fs::create_dir_all(config.thread_dir(0))
                .expect("failed to create shared dir");
        }
    }

    // Pre-populate files/dirs for operations that need existing entries
    match config.op {
        BenchOp::Stat | BenchOp::Unlink | BenchOp::Rename => {
            populate_files(config);
        }
        BenchOp::Rmdir => {
            populate_dirs(config);
        }
        BenchOp::Readdir => {
            populate_files(config);
        }
        BenchOp::Create | BenchOp::Mkdir => {
            // No pre-population needed; these create new entries
        }
    }
}

/// Remove the entire bench directory tree.
pub fn cleanup(config: &BenchConfig) {
    let bench_dir = config.bench_dir();
    if bench_dir.exists() {
        let _ = fs::remove_dir_all(&bench_dir);
    }
}

fn populate_files(config: &BenchConfig) {
    for tid in 0..config.num_threads {
        for i in 0..config.num_files_per_thread {
            let path = config.file_path(tid, i);
            fs::File::create(&path)
                .unwrap_or_else(|e| panic!("setup: create {:?} failed: {}", path, e));
        }
    }
}

fn populate_dirs(config: &BenchConfig) {
    for tid in 0..config.num_threads {
        for i in 0..config.num_files_per_thread {
            let path = config.dir_path(tid, i);
            fs::create_dir(&path)
                .unwrap_or_else(|e| panic!("setup: mkdir {:?} failed: {}", path, e));
        }
    }
}
```

**Step 2: Add `mod setup;` to main.rs**

**Step 3: Verify it compiles**

Run: `cd /data/workspace/rucksfs/benchmark/bench-tool && cargo build`

**Step 4: Commit**

```
feat(bench): add setup and cleanup for benchmark directory structures
```

---

### Task 4: Operations Module (7 syscall implementations)

**Files:**
- Create: `benchmark/bench-tool/src/ops.rs`
- Modify: `benchmark/bench-tool/src/main.rs` (add `mod ops;`)

**Step 1: Create ops.rs**

Each function takes `(config, thread_id)` and returns the number of ops completed. These are the `work` closures passed to `run_bench`.

```rust
use crate::runner::BenchConfig;
use std::fs;

/// create: open(O_CREAT | O_WRONLY) + close
pub fn op_create(config: &BenchConfig, thread_id: usize) -> u64 {
    let mut count = 0u64;
    for i in 0..config.num_files_per_thread {
        let path = config.file_path(thread_id, i);
        if fs::File::create(&path).is_ok() {
            count += 1;
        }
    }
    count
}

/// stat: metadata()
pub fn op_stat(config: &BenchConfig, thread_id: usize) -> u64 {
    let mut count = 0u64;
    for i in 0..config.num_files_per_thread {
        let path = config.file_path(thread_id, i);
        if fs::metadata(&path).is_ok() {
            count += 1;
        }
    }
    count
}

/// unlink: remove_file()
pub fn op_unlink(config: &BenchConfig, thread_id: usize) -> u64 {
    let mut count = 0u64;
    for i in 0..config.num_files_per_thread {
        let path = config.file_path(thread_id, i);
        if fs::remove_file(&path).is_ok() {
            count += 1;
        }
    }
    count
}

/// mkdir: create_dir()
pub fn op_mkdir(config: &BenchConfig, thread_id: usize) -> u64 {
    let mut count = 0u64;
    for i in 0..config.num_files_per_thread {
        let path = config.dir_path(thread_id, i);
        if fs::create_dir(&path).is_ok() {
            count += 1;
        }
    }
    count
}

/// rmdir: remove_dir()
pub fn op_rmdir(config: &BenchConfig, thread_id: usize) -> u64 {
    let mut count = 0u64;
    for i in 0..config.num_files_per_thread {
        let path = config.dir_path(thread_id, i);
        if fs::remove_dir(&path).is_ok() {
            count += 1;
        }
    }
    count
}

/// readdir: read_dir() and consume all entries
pub fn op_readdir(config: &BenchConfig, thread_id: usize) -> u64 {
    let dir = config.thread_dir(thread_id);
    let mut count = 0u64;
    for _ in 0..config.num_files_per_thread {
        if let Ok(entries) = fs::read_dir(&dir) {
            // Consume iterator to force the readdir syscalls
            for _ in entries {}
            count += 1;
        }
    }
    count
}

/// rename: rename file from original name to a .renamed suffix
pub fn op_rename(config: &BenchConfig, thread_id: usize) -> u64 {
    let mut count = 0u64;
    for i in 0..config.num_files_per_thread {
        let src = config.file_path(thread_id, i);
        let mut dst = src.clone();
        dst.set_extension("renamed");
        if fs::rename(&src, &dst).is_ok() {
            count += 1;
        }
    }
    count
}
```

**Step 2: Add `mod ops;` to main.rs**

**Step 3: Verify it compiles**

Run: `cd /data/workspace/rucksfs/benchmark/bench-tool && cargo build`

**Step 4: Commit**

```
feat(bench): add 7 metadata operation implementations
```

---

### Task 5: Report Module (CSV + Terminal Output)

**Files:**
- Create: `benchmark/bench-tool/src/report.rs`
- Modify: `benchmark/bench-tool/src/main.rs` (add `mod report;`)

**Step 1: Create report.rs**

```rust
use crate::runner::BenchResult;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Append one BenchResult row to a CSV file.
/// Creates the file with headers if it doesn't exist.
pub fn write_csv(result: &BenchResult, output_dir: &Path, timestamp: &str) {
    fs::create_dir_all(output_dir).expect("failed to create output dir");

    let csv_path = output_dir.join(format!("{}_bench.csv", timestamp));
    let needs_header = !csv_path.exists();

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&csv_path)
        .expect("failed to open CSV file");

    if needs_header {
        writeln!(
            file,
            "timestamp,operation,mode,num_threads,num_files_per_thread,total_ops,duration_sec,ops_per_sec,avg_latency_us"
        )
        .expect("failed to write CSV header");
    }

    writeln!(
        file,
        "{},{},{},{},{},{},{:.3},{:.2},{:.2}",
        timestamp,
        result.config.op,
        result.config.mode,
        result.config.num_threads,
        result.config.num_files_per_thread,
        result.total_ops,
        result.total_elapsed.as_secs_f64(),
        result.ops_per_sec,
        result.avg_latency_us,
    )
    .expect("failed to write CSV row");
}

/// Print a formatted terminal table header.
pub fn print_header(mountpoint: &Path, timestamp: &str) {
    println!("RucksFS Benchmark Results");
    println!("========================");
    println!("Mountpoint: {}", mountpoint.display());
    println!("Date: {}", timestamp);
    println!();
    println!(
        "{:<10} {:<5} {:>7} {:>7} {:>10} {:>12} {:>12} {:>12}",
        "Operation", "Mode", "Threads", "Files/T", "Total Ops", "Duration(s)", "Ops/s", "Avg Lat(us)"
    );
    println!(
        "{:<10} {:<5} {:>7} {:>7} {:>10} {:>12} {:>12} {:>12}",
        "---------", "----", "-------", "-------", "---------", "-----------", "---------", "-----------"
    );
}

/// Print one result row to the terminal.
pub fn print_row(result: &BenchResult) {
    println!(
        "{:<10} {:<5} {:>7} {:>7} {:>10} {:>12.3} {:>12.2} {:>12.2}",
        result.config.op,
        result.config.mode,
        result.config.num_threads,
        result.config.num_files_per_thread,
        result.total_ops,
        result.total_elapsed.as_secs_f64(),
        result.ops_per_sec,
        result.avg_latency_us,
    );
}

/// Print scaling analysis table for results with the same op+mode but different thread counts.
pub fn print_scaling(results: &[BenchResult]) {
    if results.len() < 2 {
        return;
    }
    let op = &results[0].config.op;
    let mode = &results[0].config.mode;

    // Find single-thread baseline
    let baseline_ops = results
        .iter()
        .find(|r| r.config.num_threads == 1)
        .map(|r| r.ops_per_sec);

    println!();
    println!("Scaling Analysis ({}, {} mode):", op, mode);
    println!("{:>7}  {:>12}  {:>10}", "Threads", "Ops/s", "Efficiency");

    for r in results {
        let efficiency = match baseline_ops {
            Some(base) if base > 0.0 => {
                r.ops_per_sec / (base * r.config.num_threads as f64) * 100.0
            }
            _ => 0.0,
        };
        println!(
            "{:>7}  {:>12.2}  {:>9.1}%",
            r.config.num_threads, r.ops_per_sec, efficiency
        );
    }
}
```

**Step 2: Add `mod report;` to main.rs**

**Step 3: Verify it compiles**

Run: `cd /data/workspace/rucksfs/benchmark/bench-tool && cargo build`

**Step 4: Commit**

```
feat(bench): add CSV and terminal report output
```

---

### Task 6: Wire Everything Together in main.rs

**Files:**
- Modify: `benchmark/bench-tool/src/main.rs`

**Step 1: Rewrite main.rs to dispatch subcommands to runner**

Complete main.rs that ties CLI -> runner -> ops -> report:

```rust
mod ops;
mod report;
mod runner;
mod setup;

use clap::{Parser, Subcommand, ValueEnum};
use runner::{BenchConfig, BenchMode, BenchOp, BenchResult, run_bench};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rucksfs-bench", about = "Metadata benchmark tool for FUSE filesystems")]
struct Cli {
    /// FUSE mount point path
    #[arg(short, long)]
    mountpoint: PathBuf,

    /// Thread count list, comma-separated (e.g. "1,2,4,8")
    #[arg(short, long, default_value = "1", value_delimiter = ',')]
    threads: Vec<usize>,

    /// Number of files/dirs per thread
    #[arg(short, long, default_value = "10000")]
    num_files: usize,

    /// CSV output directory
    #[arg(short, long, default_value = "results")]
    output: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Clone)]
enum Command {
    /// File creation benchmark (open O_CREAT + close)
    Create {
        #[arg(long, default_value = "easy")]
        mode: CliMode,
    },
    /// File stat benchmark
    Stat {
        #[arg(long, default_value = "easy")]
        mode: CliMode,
    },
    /// File unlink benchmark
    Unlink {
        #[arg(long, default_value = "easy")]
        mode: CliMode,
    },
    /// Directory mkdir benchmark
    Mkdir {
        #[arg(long, default_value = "easy")]
        mode: CliMode,
    },
    /// Directory rmdir benchmark
    Rmdir {
        #[arg(long, default_value = "easy")]
        mode: CliMode,
    },
    /// Directory readdir benchmark
    Readdir {
        #[arg(long, default_value = "easy")]
        mode: CliMode,
    },
    /// File rename benchmark
    Rename {
        #[arg(long, default_value = "easy")]
        mode: CliMode,
    },
    /// Run all operations (file chain + dir chain)
    All,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliMode {
    Easy,
    Hard,
}

impl From<CliMode> for BenchMode {
    fn from(m: CliMode) -> Self {
        match m {
            CliMode::Easy => BenchMode::Easy,
            CliMode::Hard => BenchMode::Hard,
        }
    }
}

fn main() {
    let cli = Cli::parse();

    if !cli.mountpoint.is_dir() {
        eprintln!(
            "Error: mountpoint '{}' does not exist or is not a directory",
            cli.mountpoint.display()
        );
        std::process::exit(1);
    }

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();

    let ops_and_modes: Vec<(BenchOp, BenchMode)> = match &cli.command {
        Command::Create { mode } => vec![(BenchOp::Create, (*mode).into())],
        Command::Stat { mode } => vec![(BenchOp::Stat, (*mode).into())],
        Command::Unlink { mode } => vec![(BenchOp::Unlink, (*mode).into())],
        Command::Mkdir { mode } => vec![(BenchOp::Mkdir, (*mode).into())],
        Command::Rmdir { mode } => vec![(BenchOp::Rmdir, (*mode).into())],
        Command::Readdir { mode } => vec![(BenchOp::Readdir, (*mode).into())],
        Command::Rename { mode } => vec![(BenchOp::Rename, (*mode).into())],
        Command::All => {
            // File chain: create -> stat -> rename -> unlink
            // Dir chain: mkdir -> readdir -> rmdir
            // Run both easy and hard for each
            let mut v = Vec::new();
            for mode in [BenchMode::Easy, BenchMode::Hard] {
                v.push((BenchOp::Create, mode));
                v.push((BenchOp::Stat, mode));
                v.push((BenchOp::Rename, mode));
                v.push((BenchOp::Unlink, mode));
                v.push((BenchOp::Mkdir, mode));
                v.push((BenchOp::Readdir, mode));
                v.push((BenchOp::Rmdir, mode));
            }
            v
        }
    };

    report::print_header(&cli.mountpoint, &timestamp);

    let mut all_results: Vec<BenchResult> = Vec::new();

    for (op, mode) in &ops_and_modes {
        for &num_threads in &cli.threads {
            let config = BenchConfig {
                mountpoint: cli.mountpoint.clone(),
                op: *op,
                mode: *mode,
                num_threads,
                num_files_per_thread: cli.num_files,
            };

            let work_fn = get_work_fn(*op);

            let result = run_bench(
                &config,
                |c| setup::setup(c),
                work_fn,
                |c| setup::cleanup(c),
            );

            report::print_row(&result);
            report::write_csv(&result, &cli.output, &timestamp);
            all_results.push(result);
        }
    }

    // Print scaling analysis if multiple thread counts were provided
    if cli.threads.len() > 1 {
        // Group results by (op, mode) and print scaling for each group
        let mut seen = Vec::new();
        for r in &all_results {
            let key = (r.config.op.to_string(), r.config.mode.to_string());
            if !seen.contains(&key) {
                seen.push(key.clone());
                let group: Vec<&BenchResult> = all_results
                    .iter()
                    .filter(|x| {
                        x.config.op.to_string() == key.0
                            && x.config.mode.to_string() == key.1
                    })
                    .collect();
                // print_scaling expects &[BenchResult], collect owned refs
                // We need a slight adaptation — pass references
                print_scaling_refs(&group);
            }
        }
    }
}

fn get_work_fn(op: BenchOp) -> fn(&BenchConfig, usize) -> u64 {
    match op {
        BenchOp::Create => ops::op_create,
        BenchOp::Stat => ops::op_stat,
        BenchOp::Unlink => ops::op_unlink,
        BenchOp::Mkdir => ops::op_mkdir,
        BenchOp::Rmdir => ops::op_rmdir,
        BenchOp::Readdir => ops::op_readdir,
        BenchOp::Rename => ops::op_rename,
    }
}

fn print_scaling_refs(results: &[&BenchResult]) {
    if results.len() < 2 {
        return;
    }
    let op = &results[0].config.op;
    let mode = &results[0].config.mode;
    let baseline_ops = results
        .iter()
        .find(|r| r.config.num_threads == 1)
        .map(|r| r.ops_per_sec);

    println!();
    println!("Scaling Analysis ({}, {} mode):", op, mode);
    println!("{:>7}  {:>12}  {:>10}", "Threads", "Ops/s", "Efficiency");

    for r in results {
        let efficiency = match baseline_ops {
            Some(base) if base > 0.0 => {
                r.ops_per_sec / (base * r.config.num_threads as f64) * 100.0
            }
            _ => 0.0,
        };
        println!(
            "{:>7}  {:>12.2}  {:>9.1}%",
            r.config.num_threads, r.ops_per_sec, efficiency
        );
    }
}
```

**Step 2: Verify it compiles**

Run: `cd /data/workspace/rucksfs/benchmark/bench-tool && cargo build`

**Step 3: Test the CLI help and arg parsing**

Run: `cd /data/workspace/rucksfs/benchmark/bench-tool && cargo run -- --help`
Expected: Shows usage with all subcommands

Run: `cd /data/workspace/rucksfs/benchmark/bench-tool && cargo run -- -m /tmp -t 1,2 -n 10 create --mode easy`
Expected: Runs against /tmp (local filesystem), creates 10 files per thread, prints result table

**Step 4: Commit**

```
feat(bench): wire CLI dispatch, operation chains, and scaling analysis
```

---

### Task 7: Smoke Test Against Local Filesystem

**Files:** None (manual test only)

**Step 1: Run single operation on /tmp**

```bash
cd /data/workspace/rucksfs/benchmark/bench-tool
cargo run --release -- -m /tmp -t 1 -n 100 create --mode easy
```

Expected: Table row with create results, CSV file in `results/`

**Step 2: Run scaling test**

```bash
cargo run --release -- -m /tmp -t 1,2,4 -n 1000 create --mode easy
```

Expected: 3 rows (1/2/4 threads) + scaling analysis table

**Step 3: Run `all` subcommand**

```bash
cargo run --release -- -m /tmp -t 1,2 -n 100 all
```

Expected: Full table with all 7 ops x 2 modes x 2 thread counts = 28 rows, plus scaling tables

**Step 4: Verify CSV output**

```bash
cat results/*_bench.csv
```

Expected: Valid CSV with header + data rows matching terminal output

**Step 5: Fix any issues found, then commit**

```
test(bench): verify smoke test against local filesystem
```

---

### Task 8: Operation Chain Fix for `all` Mode

**Files:**
- Modify: `benchmark/bench-tool/src/main.rs`

The `all` command's file chain (create -> stat -> rename -> unlink) requires that `create` leaves files in place for `stat`, `stat` leaves them for `rename`, etc. Currently each op does full setup+cleanup. For the `all` chain, we need setup once at the start of each chain, and cleanup once at the end.

**Step 1: Refactor `all` mode to use chain-aware execution**

In main.rs, when the command is `All`, instead of looping through `ops_and_modes` with individual setup/cleanup, run two chains:

```rust
// In the All branch, for each (mode, thread_count):
// File chain: setup for create -> run create (no cleanup) -> run stat (no setup/cleanup)
//          -> run rename (no setup/cleanup) -> run unlink (no setup/cleanup) -> cleanup
// Dir chain: setup for mkdir -> run mkdir (no cleanup) -> run readdir (no setup/cleanup)
//         -> run rmdir (no setup/cleanup) -> cleanup
```

Add a `run_bench_no_setup_cleanup` variant to `runner.rs` that skips setup/cleanup phases, or pass no-op closures.

**Step 2: Verify `all` mode works correctly**

```bash
cargo run --release -- -m /tmp -t 1 -n 100 all
```

Expected: All ops run without "file not found" errors in stat/rename/unlink phases.

**Step 3: Commit**

```
fix(bench): chain operations in all mode to avoid redundant setup/cleanup
```

---

### Task 9: Final Polish + .gitignore

**Files:**
- Create: `benchmark/bench-tool/.gitignore`
- Modify: `docs/TODO.md` — update T-30 status to ✅

**Step 1: Add .gitignore**

```
/target
/results
```

**Step 2: Update TODO.md T-30 status**

Change T-30 from `⬜` to `✅` and add details about the implementation.

**Step 3: Final build + lint check**

```bash
cd /data/workspace/rucksfs/benchmark/bench-tool
cargo build --release
cargo clippy 2>&1 | head -20   # Fix any warnings
```

**Step 4: Commit**

```
chore(bench): add gitignore and mark T-30 as complete
```

---

## Summary

| Task | Description | Key Files |
|------|-------------|-----------|
| 1 | Project scaffold + CLI skeleton | Cargo.toml, main.rs |
| 2 | Core types + BenchRunner | runner.rs |
| 3 | Setup + cleanup module | setup.rs |
| 4 | 7 operation implementations | ops.rs |
| 5 | CSV + terminal report output | report.rs |
| 6 | Wire everything together | main.rs (full rewrite) |
| 7 | Smoke test on local filesystem | (manual test) |
| 8 | Fix operation chains in `all` mode | main.rs, runner.rs |
| 9 | Polish, .gitignore, update TODO | .gitignore, TODO.md |
