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
