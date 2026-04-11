//! rucksfs-bench — Microbenchmark tool for MetadataOps.
//!
//! Bypasses FUSE entirely. Tests MetadataOps directly at two layers:
//!   Layer 1: Local MetadataServer (in-process RocksDB)
//!   Layer 2: gRPC MetadataRpcClient (remote MetadataServer)
//!
//! Usage:
//!   rucksfs-bench --mode local --threads 1,2,4,8 --ops 5000
//!   rucksfs-bench --mode grpc --meta-addr http://10.0.1.8:8001 --threads 1,2,4,8 --ops 5000

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use rucksfs_core::{DataLocation, MetadataOps};
use rucksfs_server::MetadataServer;
use rucksfs_storage::{
    open_rocks_db, RocksDeltaStore, RocksDirectoryIndex, RocksMetadataStore, RocksStorageBundle,
};

#[derive(Parser, Debug)]
#[command(name = "rucksfs-bench", version, about = "RucksFS Metadata Microbenchmark")]
struct Cli {
    /// Benchmark mode: "local" (in-process) or "grpc" (remote).
    #[arg(long, default_value = "local")]
    mode: String,

    /// MetadataServer gRPC address (only for grpc mode).
    #[arg(long, value_name = "ADDR")]
    meta_addr: Option<String>,

    /// Comma-separated thread counts to test (e.g., "1,2,4,8,16").
    #[arg(long, default_value = "1,2,4,8,16")]
    threads: String,

    /// Number of operations per thread.
    #[arg(long, default_value = "5000")]
    ops: u64,

    /// Data directory for local mode (temp dir if not specified).
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

struct BenchResult {
    op: String,
    threads: usize,
    total_ops: u64,
    elapsed: Duration,
    latencies: Vec<Duration>,
}

impl BenchResult {
    fn ops_per_sec(&self) -> f64 {
        self.total_ops as f64 / self.elapsed.as_secs_f64()
    }

    fn p50_us(&self) -> f64 {
        percentile(&self.latencies, 50) as f64 / 1000.0
    }

    fn p99_us(&self) -> f64 {
        percentile(&self.latencies, 99) as f64 / 1000.0
    }

    fn avg_us(&self) -> f64 {
        if self.latencies.is_empty() {
            return 0.0;
        }
        let sum: u128 = self.latencies.iter().map(|d| d.as_nanos()).sum();
        (sum as f64 / self.latencies.len() as f64) / 1000.0
    }
}

fn percentile(sorted: &[Duration], pct: usize) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (sorted.len() * pct / 100).min(sorted.len() - 1);
    sorted[idx].as_nanos()
}

fn build_local_server(data_dir: &std::path::Path) -> Arc<dyn MetadataOps> {
    std::fs::create_dir_all(data_dir).expect("failed to create data dir");
    let db_path = data_dir.join("bench-meta.db");
    let db = open_rocks_db(&db_path).expect("failed to open RocksDB");
    let metadata = Arc::new(RocksMetadataStore::new(Arc::clone(&db)));
    let index = Arc::new(RocksDirectoryIndex::new(Arc::clone(&db)));
    let delta_store = Arc::new(RocksDeltaStore::new(Arc::clone(&db)));
    let storage_bundle = Arc::new(RocksStorageBundle::new(Arc::clone(&db)));
    Arc::new(MetadataServer::new(
        metadata,
        index,
        delta_store,
        DataLocation {
            server_id: "default".to_string(),
        },
        storage_bundle,
    ))
}

/// Benchmark create: create N files under root, each with a unique name.
async fn bench_create(
    meta: Arc<dyn MetadataOps>,
    n_threads: usize,
    ops_per_thread: u64,
) -> BenchResult {
    const ROOT: u64 = 1;

    // Create a unique parent directory for this run to avoid conflicts.
    let run_dir = meta
        .mkdir(ROOT, &format!("bench_create_{}", n_threads), 0o755, 0, 0)
        .await
        .expect("failed to create bench dir");
    let parent = run_dir.inode;

    let all_latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let start = Instant::now();
    let mut handles = Vec::new();

    for tid in 0..n_threads {
        let meta = meta.clone();
        let lats = all_latencies.clone();
        handles.push(tokio::spawn(async move {
            let mut local_lats = Vec::with_capacity(ops_per_thread as usize);
            for i in 0..ops_per_thread {
                let name = format!("f_{}_{}", tid, i);
                let t0 = Instant::now();
                meta.create(parent, &name, 0o644, 0, 0).await.expect("create failed");
                local_lats.push(t0.elapsed());
            }
            lats.lock().await.extend(local_lats);
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }
    let elapsed = start.elapsed();

    let mut latencies = Arc::try_unwrap(all_latencies)
        .expect("arc unwrap")
        .into_inner();
    latencies.sort();

    BenchResult {
        op: "create".to_string(),
        threads: n_threads,
        total_ops: n_threads as u64 * ops_per_thread,
        elapsed,
        latencies,
    }
}

/// Benchmark getattr: stat existing inodes.
async fn bench_stat(
    meta: Arc<dyn MetadataOps>,
    inodes: &[u64],
    n_threads: usize,
    ops_per_thread: u64,
) -> BenchResult {
    let inodes = Arc::new(inodes.to_vec());
    let all_latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let start = Instant::now();
    let mut handles = Vec::new();

    for _tid in 0..n_threads {
        let meta = meta.clone();
        let inodes = inodes.clone();
        let lats = all_latencies.clone();
        handles.push(tokio::spawn(async move {
            let mut local_lats = Vec::with_capacity(ops_per_thread as usize);
            for i in 0..ops_per_thread {
                let ino = inodes[i as usize % inodes.len()];
                let t0 = Instant::now();
                meta.getattr(ino).await.expect("getattr failed");
                local_lats.push(t0.elapsed());
            }
            lats.lock().await.extend(local_lats);
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }
    let elapsed = start.elapsed();

    let mut latencies = Arc::try_unwrap(all_latencies)
        .expect("arc unwrap")
        .into_inner();
    latencies.sort();

    BenchResult {
        op: "stat".to_string(),
        threads: n_threads,
        total_ops: n_threads as u64 * ops_per_thread,
        elapsed,
        latencies,
    }
}

/// Benchmark unlink: delete files then measure.
async fn bench_unlink(
    meta: Arc<dyn MetadataOps>,
    parent: u64,
    names: Vec<String>,
    n_threads: usize,
) -> BenchResult {
    let chunk_size = (names.len() + n_threads - 1) / n_threads;
    let chunks: Vec<Vec<String>> = names
        .chunks(chunk_size)
        .map(|c| c.to_vec())
        .collect();
    let all_latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let total_ops = names.len() as u64;

    let start = Instant::now();
    let mut handles = Vec::new();

    for chunk in chunks {
        let meta = meta.clone();
        let lats = all_latencies.clone();
        handles.push(tokio::spawn(async move {
            let mut local_lats = Vec::with_capacity(chunk.len());
            for name in &chunk {
                let t0 = Instant::now();
                meta.unlink(parent, name).await.expect("unlink failed");
                local_lats.push(t0.elapsed());
            }
            lats.lock().await.extend(local_lats);
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }
    let elapsed = start.elapsed();

    let mut latencies = Arc::try_unwrap(all_latencies)
        .expect("arc unwrap")
        .into_inner();
    latencies.sort();

    BenchResult {
        op: "unlink".to_string(),
        threads: n_threads,
        total_ops,
        elapsed,
        latencies,
    }
}

fn print_header() {
    println!(
        "{:<10} {:>7} {:>10} {:>12} {:>10} {:>10} {:>10}",
        "Op", "Threads", "Total", "ops/s", "Avg(us)", "P50(us)", "P99(us)"
    );
    println!("{}", "-".repeat(75));
}

fn print_result(r: &BenchResult) {
    println!(
        "{:<10} {:>7} {:>10} {:>12.0} {:>10.1} {:>10.1} {:>10.1}",
        r.op,
        r.threads,
        r.total_ops,
        r.ops_per_sec(),
        r.avg_us(),
        r.p50_us(),
        r.p99_us(),
    );
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let thread_counts: Vec<usize> = cli
        .threads
        .split(',')
        .map(|s| s.trim().parse().expect("invalid thread count"))
        .collect();

    let meta: Arc<dyn MetadataOps> = match cli.mode.as_str() {
        "local" => {
            let data_dir = cli.data_dir.unwrap_or_else(|| {
                let tmp = tempfile::tempdir().expect("failed to create temp dir");
                let path = tmp.path().to_path_buf();
                std::mem::forget(tmp);
                path
            });
            println!("Mode: LOCAL (in-process RocksDB)");
            println!("Data dir: {}", data_dir.display());
            build_local_server(&data_dir)
        }
        "grpc" => {
            let addr = cli
                .meta_addr
                .as_deref()
                .expect("--meta-addr required for grpc mode");
            println!("Mode: gRPC (remote MetadataServer)");
            println!("Server: {}", addr);
            let client = rucksfs_rpc::MetadataRpcClient::connect(addr.to_string())
                .await
                .expect("failed to connect to MetadataServer");
            Arc::new(client)
        }
        _ => {
            eprintln!("Unknown mode: {}. Use 'local' or 'grpc'.", cli.mode);
            std::process::exit(1);
        }
    };

    println!("Ops per thread: {}", cli.ops);
    println!();

    // --- Create benchmark ---
    println!("=== CREATE ===");
    print_header();
    for &nt in &thread_counts {
        let r = bench_create(meta.clone(), nt, cli.ops).await;
        print_result(&r);
    }
    println!();

    // Collect inodes for stat benchmark.
    let stat_dir = meta
        .mkdir(1, "bench_stat_prep", 0o755, 0, 0)
        .await
        .expect("mkdir for stat prep");
    let mut stat_inodes = Vec::new();
    for i in 0..1000u64 {
        let attr = meta
            .create(stat_dir.inode, &format!("s_{}", i), 0o644, 0, 0)
            .await
            .expect("create for stat prep");
        stat_inodes.push(attr.inode);
    }

    // --- Stat benchmark ---
    println!("=== STAT ===");
    print_header();
    for &nt in &thread_counts {
        let r = bench_stat(meta.clone(), &stat_inodes, nt, cli.ops).await;
        print_result(&r);
    }
    println!();

    // --- Unlink benchmark ---
    println!("=== UNLINK ===");
    print_header();
    for &nt in &thread_counts {
        let dir_name = format!("bench_unlink_{}", nt);
        let unlink_dir = meta
            .mkdir(1, &dir_name, 0o755, 0, 0)
            .await
            .expect("mkdir for unlink");
        let total = nt as u64 * cli.ops;
        let mut names = Vec::with_capacity(total as usize);
        for i in 0..total {
            let name = format!("u_{}", i);
            meta.create(unlink_dir.inode, &name, 0o644, 0, 0)
                .await
                .expect("create for unlink");
            names.push(name);
        }
        let r = bench_unlink(meta.clone(), unlink_dir.inode, names, nt).await;
        print_result(&r);
    }

    println!();
    println!("Done.");
}
