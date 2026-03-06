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

#[allow(dead_code)]
pub struct ThreadResult {
    pub thread_id: usize,
    pub ops_completed: u64,
    pub elapsed: Duration,
}

#[allow(dead_code)]
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
