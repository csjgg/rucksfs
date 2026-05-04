#!/usr/bin/env python3
"""Parse mdtest outputs from round3 runs into a CSV.

Usage: parse-results.py <results_dir> <output_csv>

Filename pattern: <sut>_<mode>_np<N>_run<R>.txt
SUT values: rucksfs-delta, rucksfs-nodelta, nfs, juicefs-redis, juicefs-tikv
mode: hard | easy

mdtest output lines of interest:
   File creation        : <max> <min> <mean> <stddev>    (ops/sec row)
   File stat            : ...
   File removal         : ...
"""
import os
import re
import sys
import csv
import glob
from statistics import median


def parse_file(path):
    """Return dict with keys: create, stat, remove (ops/s as floats) if found."""
    out = {}
    with open(path) as f:
        text = f.read()
    # mdtest prints a summary table:
    # File creation     :     12345.678      234.5     4567.0   200.0
    # We take the "mean" column (3rd numeric) as the ops/sec.
    for key, label in [("create", "File creation"),
                       ("stat", "File stat"),
                       ("remove", "File removal")]:
        m = re.search(rf"^\s*{label}\s*:\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)",
                      text, re.MULTILINE)
        if m:
            # take the MAX (field 1) – mdtest's max is the best observed
            # across the -i iterations. For i=1 this is just the value.
            out[key] = float(m.group(1))
    return out


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        sys.exit(1)
    results_dir, csv_out = sys.argv[1], sys.argv[2]

    rows = []
    for path in sorted(glob.glob(os.path.join(results_dir, "*.txt"))):
        fn = os.path.basename(path)
        m = re.match(r"(?P<sut>[\w-]+?)_(?P<mode>hard|easy)_np(?P<N>\d+)_run(?P<run>\d+)\.txt$", fn)
        if not m:
            continue
        g = m.groupdict()
        metrics = parse_file(path)
        if not metrics:
            continue
        rows.append({
            "sut": g["sut"],
            "mode": g["mode"],
            "N": int(g["N"]),
            "run": int(g["run"]),
            **metrics,
            "file": fn,
        })

    # Write raw CSV
    with open(csv_out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=["sut", "mode", "N", "run", "create", "stat", "remove", "file"])
        w.writeheader()
        for r in rows:
            w.writerow(r)

    # Aggregate by (sut, mode, N): median over runs
    from collections import defaultdict
    agg = defaultdict(list)
    for r in rows:
        agg[(r["sut"], r["mode"], r["N"])].append(r)
    summary_path = csv_out.replace(".csv", "_summary.csv")
    with open(summary_path, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["sut", "mode", "N", "n_runs", "create_median", "stat_median", "remove_median"])
        for key, group in sorted(agg.items()):
            sut, mode, N = key
            def med(field):
                vals = [r[field] for r in group if field in r]
                return f"{median(vals):.2f}" if vals else ""
            w.writerow([sut, mode, N, len(group), med("create"), med("stat"), med("remove")])

    print(f"raw: {csv_out}  ({len(rows)} rows)")
    print(f"summary: {summary_path}  ({len(agg)} groups)")


if __name__ == "__main__":
    main()
