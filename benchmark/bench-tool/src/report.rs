use crate::runner::BenchResult;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Append one BenchResult row to a CSV file.
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
pub fn print_scaling(results: &[&BenchResult]) {
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
