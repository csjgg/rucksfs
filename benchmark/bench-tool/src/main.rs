mod ops;
mod report;
mod runner;
mod setup;

use clap::{Parser, Subcommand, ValueEnum};
use runner::{BenchConfig, BenchMode, BenchOp, BenchResult};
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

            let result = runner::run_bench(
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
                report::print_scaling(&group);
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
