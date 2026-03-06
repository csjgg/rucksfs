use crate::runner::{BenchConfig, BenchMode, BenchOp};
use std::fs;

/// Create the directory structure needed before a benchmark.
pub fn setup(config: &BenchConfig) {
    let bench_dir = config.bench_dir();
    fs::create_dir_all(&bench_dir).expect("failed to create bench dir");

    match config.mode {
        BenchMode::Easy => {
            for tid in 0..config.num_threads {
                fs::create_dir_all(config.thread_dir(tid))
                    .expect("failed to create thread dir");
            }
        }
        BenchMode::Hard => {
            fs::create_dir_all(config.thread_dir(0))
                .expect("failed to create shared dir");
        }
    }

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
        BenchOp::Create | BenchOp::Mkdir => {}
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
