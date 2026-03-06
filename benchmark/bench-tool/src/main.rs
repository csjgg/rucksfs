mod ops;
mod report;
mod runner;
mod setup;

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

impl From<CliMode> for runner::BenchMode {
    fn from(m: CliMode) -> Self {
        match m {
            CliMode::Easy => runner::BenchMode::Easy,
            CliMode::Hard => runner::BenchMode::Hard,
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
    println!(
        "rucksfs-bench: mountpoint={}, threads={:?}, num_files={}",
        cli.mountpoint.display(),
        cli.threads,
        cli.num_files
    );
}
