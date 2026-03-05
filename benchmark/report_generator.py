#!/usr/bin/env python3
"""
RucksFS Benchmark Report Generator

Aggregates CSV data from benchmark runs into structured JSON and
human-readable Markdown reports.

Usage:
    python3 benchmark/report_generator.py [--results-dir DIR]

Default results directory: benchmark/results/
"""

import argparse
import csv
import json
import os
import platform
import sys
from collections import defaultdict
from datetime import datetime
from pathlib import Path


def find_latest_csv(results_dir, prefix):
    """Find the most recent CSV file matching prefix."""
    candidates = sorted(
        Path(results_dir).glob(f"{prefix}_*.csv"),
        key=lambda p: p.stat().st_mtime,
        reverse=True,
    )
    return candidates[0] if candidates else None


def parse_metadata_ops(csv_path):
    """Parse metadata_ops CSV into per-operation stats."""
    if not csv_path or not csv_path.exists():
        return None
    ops = defaultdict(list)
    with open(csv_path, newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            benchmark = row.get("benchmark", "")
            variant = row.get("variant", "")
            ops_per_sec = float(row.get("ops_per_sec", 0))
            key = f"{benchmark}/{variant}" if variant else benchmark
            ops[key].append(ops_per_sec)
    # Compute stats per operation.
    results = {}
    for key, values in ops.items():
        values.sort()
        n = len(values)
        results[key] = {
            "samples": n,
            "avg": sum(values) / n if n else 0,
            "min": values[0] if n else 0,
            "max": values[-1] if n else 0,
            "p50": values[n // 2] if n else 0,
            "p99": values[int(n * 0.99)] if n else 0,
        }
    return results


def parse_io_throughput(csv_path):
    """Parse io_throughput CSV into per-variant stats."""
    if not csv_path or not csv_path.exists():
        return None
    rows = []
    with open(csv_path, newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            rows.append({
                "benchmark": row.get("benchmark", ""),
                "variant": row.get("variant", ""),
                "block_size": row.get("block_size", ""),
                "total_bytes": int(row.get("total_bytes", 0)),
                "duration_sec": float(row.get("duration_sec", 0)),
                "throughput_mbps": float(row.get("throughput_mbps", 0)),
                "iops": float(row.get("iops", 0)),
            })
    return rows


def parse_concurrent_stress(csv_path):
    """Parse concurrent_stress CSV into scaling data."""
    if not csv_path or not csv_path.exists():
        return None
    rows = []
    with open(csv_path, newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            rows.append({
                "benchmark": row.get("benchmark", ""),
                "variant": row.get("variant", ""),
                "num_threads": int(row.get("num_threads", 0)),
                "ops_total": int(row.get("ops_total", 0)),
                "duration_sec": float(row.get("duration_sec", 0)),
                "agg_ops_per_sec": float(row.get("agg_ops_per_sec", 0)),
            })
    return rows


def parse_posix_conformance(results_dir):
    """Parse the latest posix_conformance log file for pass/fail counts."""
    logs = sorted(
        Path(results_dir).glob("posix_conformance_*.log"),
        key=lambda p: p.stat().st_mtime,
        reverse=True,
    )
    if not logs:
        return None
    with open(logs[0]) as f:
        content = f.read()
    # Try structured TOTAL line first: "TOTAL: PASS=X  FAIL=Y  SKIP=Z"
    for line in content.split("\n"):
        if "TOTAL:" in line and "PASS=" in line:
            parts = line.strip().split()
            result = {}
            for part in parts:
                if "=" in part:
                    k, v = part.split("=")
                    result[k] = int(v)
            return result
    # Fallback: count [PASS], [FAIL], [SKIP] markers in output.
    pass_count = content.count("[PASS]")
    fail_count = content.count("[FAIL]")
    skip_count = content.count("[SKIP]")
    if pass_count + fail_count + skip_count > 0:
        return {"PASS": pass_count, "FAIL": fail_count, "SKIP": skip_count}
    return None


def generate_json_report(data):
    """Generate structured JSON report."""
    return json.dumps(data, indent=2, default=str)


def generate_markdown_report(data):
    """Generate human-readable Markdown report."""
    lines = []
    lines.append("# RucksFS Benchmark Report")
    lines.append("")
    lines.append(f"**Generated:** {data['timestamp']}")
    lines.append(f"**System:** {data['system']['os']} / {data['system']['arch']}")
    lines.append("")

    # POSIX Conformance
    if data.get("posix_conformance"):
        pc = data["posix_conformance"]
        total = pc.get("PASS", 0) + pc.get("FAIL", 0) + pc.get("SKIP", 0)
        lines.append("## POSIX Conformance")
        lines.append("")
        lines.append("| Metric | Value |")
        lines.append("|--------|-------|")
        lines.append(f"| Passed | {pc.get('PASS', 0)} |")
        lines.append(f"| Failed | {pc.get('FAIL', 0)} |")
        lines.append(f"| Skipped | {pc.get('SKIP', 0)} |")
        lines.append(f"| Total | {total} |")
        if total:
            lines.append(f"| Pass Rate | {pc.get('PASS', 0) * 100 / total:.1f}% |")
        lines.append("")

    # Metadata Performance
    if data.get("metadata_ops"):
        lines.append("## Metadata Performance")
        lines.append("")
        lines.append("| Operation | Avg ops/sec | P50 | P99 | Samples |")
        lines.append("|-----------|-------------|-----|-----|---------|")
        for op, stats in sorted(data["metadata_ops"].items()):
            lines.append(
                f"| {op} | {stats['avg']:.0f} | {stats['p50']:.0f} | {stats['p99']:.0f} | {stats['samples']} |"
            )
        lines.append("")

    # I/O Throughput
    if data.get("io_throughput"):
        lines.append("## I/O Throughput")
        lines.append("")
        lines.append("| Benchmark | Variant | Block Size | Throughput (MB/s) | IOPS |")
        lines.append("|-----------|---------|------------|-------------------|------|")
        for row in data["io_throughput"]:
            lines.append(
                f"| {row['benchmark']} | {row['variant']} | {row['block_size']} | "
                f"{row['throughput_mbps']:.2f} | {row['iops']:.0f} |"
            )
        lines.append("")

    # Concurrency Scaling
    if data.get("concurrent_stress"):
        lines.append("## Concurrency Scaling")
        lines.append("")
        lines.append("| Benchmark | Variant | Threads | Agg ops/sec |")
        lines.append("|-----------|---------|---------|-------------|")
        for row in data["concurrent_stress"]:
            lines.append(
                f"| {row['benchmark']} | {row['variant']} | {row['num_threads']} | "
                f"{row['agg_ops_per_sec']:.0f} |"
            )
        lines.append("")

    # Summary
    lines.append("---")
    lines.append("")
    lines.append("*Generated by `benchmark/report_generator.py`*")

    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description="RucksFS Benchmark Report Generator")
    parser.add_argument(
        "--results-dir",
        default="benchmark/results",
        help="Directory containing benchmark CSV/log files",
    )
    parser.add_argument(
        "--output-dir",
        default=None,
        help="Output directory for reports (default: same as results-dir)",
    )
    args = parser.parse_args()

    results_dir = Path(args.results_dir)
    output_dir = Path(args.output_dir) if args.output_dir else results_dir

    if not results_dir.exists():
        print(f"Error: results directory '{results_dir}' does not exist", file=sys.stderr)
        sys.exit(1)

    output_dir.mkdir(parents=True, exist_ok=True)

    # Collect data.
    data = {
        "timestamp": datetime.now().isoformat(),
        "system": {
            "os": platform.system(),
            "arch": platform.machine(),
            "python": platform.python_version(),
        },
    }

    # Parse available benchmark data.
    metadata_csv = find_latest_csv(results_dir, "metadata_ops")
    if metadata_csv:
        data["metadata_ops"] = parse_metadata_ops(metadata_csv)
        print(f"  Parsed: {metadata_csv.name}")

    io_csv = find_latest_csv(results_dir, "io_throughput")
    if io_csv:
        data["io_throughput"] = parse_io_throughput(io_csv)
        print(f"  Parsed: {io_csv.name}")

    concurrent_csv = find_latest_csv(results_dir, "concurrent_stress")
    if concurrent_csv:
        data["concurrent_stress"] = parse_concurrent_stress(concurrent_csv)
        print(f"  Parsed: {concurrent_csv.name}")

    posix = parse_posix_conformance(results_dir)
    if posix:
        data["posix_conformance"] = posix
        print("  Parsed: posix conformance log")

    if len(data) <= 2:
        print("No benchmark data found in results directory.", file=sys.stderr)
        sys.exit(1)

    # Generate reports.
    json_path = output_dir / "report.json"
    md_path = output_dir / "report.md"

    with open(json_path, "w") as f:
        f.write(generate_json_report(data))
    print(f"  JSON report: {json_path}")

    with open(md_path, "w") as f:
        f.write(generate_markdown_report(data))
    print(f"  Markdown report: {md_path}")

    # Console summary.
    print()
    print("=== Summary ===")
    if posix:
        print(f"  POSIX: {posix.get('PASS', 0)} pass, {posix.get('FAIL', 0)} fail")
    if data.get("metadata_ops"):
        ops = data["metadata_ops"]
        total_avg = sum(s["avg"] for s in ops.values()) / len(ops) if ops else 0
        print(f"  Metadata: {len(ops)} benchmarks, avg {total_avg:.0f} ops/sec")
    if data.get("io_throughput"):
        print(f"  I/O: {len(data['io_throughput'])} data points")
    if data.get("concurrent_stress"):
        print(f"  Concurrency: {len(data['concurrent_stress'])} data points")


if __name__ == "__main__":
    main()
