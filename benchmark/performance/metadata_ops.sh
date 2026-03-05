#!/usr/bin/env bash
# =============================================================================
# benchmark/performance/metadata_ops.sh
# Metadata operation microbenchmarks for RucksFS
#
# Academic references:
#   - TableFS (ATC'13) §5.2: 1M file create/stat/delete in single directory
#   - SingularFS (ATC'23) §6: billion-scale create/stat, per-op latency
#   - LocoFS (SC'17) §6.3: readdir + stat pipeline (ls -l pattern)
#   - mdtest (IO500): MDEasy (private dirs) and MDHard (shared dir)
#   - BFO (TOS'20) §4: batch metadata operation overhead analysis
#
# Usage:
#   ./benchmark/performance/metadata_ops.sh --mountpoint <path> [options]
#
# Options:
#   --num-files N    Number of files per benchmark (default: 10000)
#   --num-dirs M     Number of directories for multi-dir tests (default: 100)
#   --depth D        Depth for deep-tree test (default: 50)
#
# Output:
#   benchmark/results/metadata_ops_<timestamp>.csv
# =============================================================================

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$PROJECT_ROOT/benchmark/results"
MOUNTPOINT=""
NUM_FILES=10000
NUM_DIRS=100
DEPTH=50
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

while [[ $# -gt 0 ]]; do
    case "$1" in
        --mountpoint)  MOUNTPOINT="$2"; shift 2 ;;
        --num-files)   NUM_FILES="$2"; shift 2 ;;
        --num-dirs)    NUM_DIRS="$2"; shift 2 ;;
        --depth)       DEPTH="$2"; shift 2 ;;
        -h|--help)
            echo "Usage: $0 --mountpoint <path> [--num-files N] [--num-dirs M] [--depth D]"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ -z "$MOUNTPOINT" ]]; then
    echo "ERROR: --mountpoint is required"; exit 1
fi
if [[ ! -d "$MOUNTPOINT" ]]; then
    echo "ERROR: Mountpoint does not exist: $MOUNTPOINT"; exit 1
fi

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

mkdir -p "$RESULTS_DIR"
CSV_FILE="$RESULTS_DIR/metadata_ops_${TIMESTAMP}.csv"
LOG_FILE="$RESULTS_DIR/metadata_ops_${TIMESTAMP}.log"
TEST_BASE="$MOUNTPOINT/.bench_meta_$$"

exec > >(tee -a "$LOG_FILE") 2>&1

echo "timestamp,benchmark,variant,num_files,num_dirs,depth,ops_total,duration_sec,ops_per_sec" > "$CSV_FILE"

cleanup() {
    rm -rf "$TEST_BASE" 2>/dev/null || true
}
trap cleanup EXIT
mkdir -p "$TEST_BASE"

# High-resolution timer (nanoseconds)
now_ns() {
    date +%s%N 2>/dev/null || python3 -c "import time; print(int(time.time()*1e9))"
}

# Compute ops/sec from ns duration
calc_ops_sec() {
    local ops=$1 duration_ns=$2
    if [[ "$duration_ns" -le 0 ]]; then echo "0"; return; fi
    python3 -c "print(f'{$ops / ($duration_ns / 1e9):.2f}')" 2>/dev/null || \
        echo "scale=2; $ops * 1000000000 / $duration_ns" | bc 2>/dev/null || echo "N/A"
}

ns_to_sec() {
    local ns=$1
    python3 -c "print(f'{$ns / 1e9:.4f}')" 2>/dev/null || \
        echo "scale=4; $ns / 1000000000" | bc 2>/dev/null || echo "N/A"
}

record() {
    local bench="$1" variant="$2" nf="$3" nd="$4" dp="$5" ops="$6" dur_ns="$7"
    local dur_sec opsec
    dur_sec=$(ns_to_sec "$dur_ns")
    opsec=$(calc_ops_sec "$ops" "$dur_ns")
    echo "${TIMESTAMP},${bench},${variant},${nf},${nd},${dp},${ops},${dur_sec},${opsec}" >> "$CSV_FILE"
    echo -e "  ${CYAN}→${NC} ${ops} ops in ${dur_sec}s = ${GREEN}${opsec} ops/sec${NC}"
}

# ==========================================================================
echo "╔══════════════════════════════════════════════════════╗"
echo "║  RucksFS — Metadata Operations Benchmark            ║"
echo "║  ref: TableFS, SingularFS, LocoFS, mdtest           ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
echo "  Mountpoint: $MOUNTPOINT"
echo "  Files:      $NUM_FILES"
echo "  Dirs:       $NUM_DIRS"
echo "  Depth:      $DEPTH"
echo ""

# ==========================================================================
# B1: File Create — Single Directory (ref: TableFS §5.2, mdtest-hard)
#
# Measures raw metadata throughput for file creation in a shared namespace.
# This is the "hardest" case because all creates contend on the same
# directory's metadata lock.
# ==========================================================================
echo "── B1: File Create (single dir, N=$NUM_FILES) ──"
echo "   [mdtest-hard pattern: all files in one directory]"

B1_DIR="$TEST_BASE/b1_single"
mkdir -p "$B1_DIR"

start=$(now_ns)
for i in $(seq 1 "$NUM_FILES"); do
    touch "$B1_DIR/f_$i"
done
end=$(now_ns)
dur=$((end - start))

record "file_create" "single_dir" "$NUM_FILES" "1" "0" "$NUM_FILES" "$dur"

# Cleanup
rm -rf "$B1_DIR"
echo ""

# ==========================================================================
# B2: File Create — Multi Directory (ref: mdtest-easy)
#
# Distributes file creation across M directories. This represents the
# "easy" case where each directory has independent metadata, eliminating
# contention. The ratio B2/B1 reveals lock contention overhead.
# ==========================================================================
echo "── B2: File Create (multi-dir, N=$NUM_FILES across M=$NUM_DIRS dirs) ──"
echo "   [mdtest-easy pattern: files spread across private directories]"

B2_DIR="$TEST_BASE/b2_multi"
mkdir -p "$B2_DIR"

# Pre-create directories
for d in $(seq 1 "$NUM_DIRS"); do
    mkdir "$B2_DIR/d_$d"
done

files_per_dir=$((NUM_FILES / NUM_DIRS))

# Pre-generate the file list outside the timed section to exclude shell
# loop overhead from the measurement (only FUSE touch ops are timed).
B2_FILE_LIST=$(mktemp)
for d in $(seq 1 "$NUM_DIRS"); do
    for f in $(seq 1 "$files_per_dir"); do
        echo "$B2_DIR/d_$d/f_$f"
    done
done > "$B2_FILE_LIST"
total_created=$(wc -l < "$B2_FILE_LIST")

start=$(now_ns)
xargs touch < "$B2_FILE_LIST"
end=$(now_ns)
dur=$((end - start))

rm -f "$B2_FILE_LIST"
record "file_create" "multi_dir" "$total_created" "$NUM_DIRS" "0" "$total_created" "$dur"
rm -rf "$B2_DIR"
echo ""

# ==========================================================================
# B3: File Stat (ref: TableFS §5.2, SingularFS §6.1)
#
# Measures getattr throughput. In KV-backed systems, this involves:
# 1. Directory entry lookup (prefix scan or point query)
# 2. Inode value retrieval
# 3. Delta folding (if delta-based updates are used)
# ==========================================================================
echo "── B3: File Stat (N=$NUM_FILES) ──"
echo "   [Measures getattr/stat throughput on existing files]"

B3_DIR="$TEST_BASE/b3_stat"
mkdir -p "$B3_DIR"
for i in $(seq 1 "$NUM_FILES"); do
    touch "$B3_DIR/f_$i"
done

start=$(now_ns)
for i in $(seq 1 "$NUM_FILES"); do
    stat "$B3_DIR/f_$i" > /dev/null 2>&1
done
end=$(now_ns)
dur=$((end - start))

record "file_stat" "sequential" "$NUM_FILES" "1" "0" "$NUM_FILES" "$dur"
rm -rf "$B3_DIR"
echo ""

# ==========================================================================
# B4: File Delete (ref: TableFS §5.2)
#
# Measures unlink throughput. In KV-backed systems, deletion involves:
# 1. Directory entry removal
# 2. Inode metadata update (nlink decrement)
# 3. Potential data block deallocation
# ==========================================================================
echo "── B4: File Delete (N=$NUM_FILES) ──"
echo "   [Measures unlink throughput]"

B4_DIR="$TEST_BASE/b4_delete"
mkdir -p "$B4_DIR"
for i in $(seq 1 "$NUM_FILES"); do
    touch "$B4_DIR/f_$i"
done

start=$(now_ns)
for i in $(seq 1 "$NUM_FILES"); do
    rm "$B4_DIR/f_$i"
done
end=$(now_ns)
dur=$((end - start))

record "file_delete" "sequential" "$NUM_FILES" "1" "0" "$NUM_FILES" "$dur"
rmdir "$B4_DIR" 2>/dev/null || rm -rf "$B4_DIR"
echo ""

# ==========================================================================
# B5: mkdir (ref: SingularFS §6.2)
#
# Directory creation is heavier than file creation because it involves:
# 1. New inode allocation
# 2. Directory entry in parent
# 3. Initialization of . and .. entries (POSIX)
# 4. Parent nlink increment
# ==========================================================================
echo "── B5: mkdir (N=$NUM_DIRS) ──"

B5_DIR="$TEST_BASE/b5_mkdir"
mkdir -p "$B5_DIR"

start=$(now_ns)
for i in $(seq 1 "$NUM_DIRS"); do
    mkdir "$B5_DIR/d_$i"
done
end=$(now_ns)
dur=$((end - start))

record "mkdir" "flat" "$NUM_DIRS" "$NUM_DIRS" "0" "$NUM_DIRS" "$dur"
rm -rf "$B5_DIR"
echo ""

# ==========================================================================
# B6: readdir (ref: LocoFS §6.3)
#
# Measures directory listing throughput for directories of varying sizes.
# In KV-backed systems, readdir is a prefix scan over dir_entries CF.
# ==========================================================================
echo "── B6: readdir ──"
echo "   [Measures directory listing performance at various sizes]"

for dir_size in 100 1000 5000; do
    if [[ $dir_size -gt $NUM_FILES ]]; then continue; fi

    B6_DIR="$TEST_BASE/b6_readdir_${dir_size}"
    mkdir -p "$B6_DIR"
    for i in $(seq 1 "$dir_size"); do
        touch "$B6_DIR/f_$i"
    done

    # Measure 10 readdir iterations
    iterations=10
    start=$(now_ns)
    for _ in $(seq 1 $iterations); do
        ls "$B6_DIR" > /dev/null
    done
    end=$(now_ns)
    dur=$((end - start))
    total_entries=$((dir_size * iterations))

    record "readdir" "entries_${dir_size}" "$dir_size" "1" "0" "$total_entries" "$dur"
    rm -rf "$B6_DIR"
done
echo ""

# ==========================================================================
# B7: readdir + stat (ls -l pattern) (ref: LocoFS §6.3, BFO §4)
#
# The most common real-world metadata access pattern. After listing a
# directory, the application (ls -l) calls stat() on each entry.
# This is where batch prefetch (BFO/eBPF) can dramatically help.
# The ratio B7 vs (B6 + B3) reveals whether the system optimizes this
# combined access pattern.
# ==========================================================================
echo "── B7: readdir + stat (ls -l pattern) ──"
echo "   [Simulates 'ls -l': readdir then stat each entry]"

for dir_size in 100 1000 5000; do
    if [[ $dir_size -gt $NUM_FILES ]]; then continue; fi

    B7_DIR="$TEST_BASE/b7_ls_${dir_size}"
    mkdir -p "$B7_DIR"
    for i in $(seq 1 "$dir_size"); do
        echo "content_$i" > "$B7_DIR/f_$i"
    done

    iterations=5
    start=$(now_ns)
    for _ in $(seq 1 $iterations); do
        ls -l "$B7_DIR" > /dev/null
    done
    end=$(now_ns)
    dur=$((end - start))
    total_ops=$((dir_size * iterations))

    record "readdir_stat" "ls_l_${dir_size}" "$dir_size" "1" "0" "$total_ops" "$dur"
    rm -rf "$B7_DIR"
done
echo ""

# ==========================================================================
# B8: Rename (ref: SingularFS §6.3)
#
# Rename is the most complex metadata operation:
# - Same-directory rename: update one dir entry
# - Cross-directory rename: remove from source, add to destination
# Both must be atomic (POSIX guarantee).
# ==========================================================================
echo "── B8: Rename (N=$NUM_FILES) ──"

B8_DIR="$TEST_BASE/b8_rename"
mkdir -p "$B8_DIR"
for i in $(seq 1 "$NUM_FILES"); do
    touch "$B8_DIR/old_$i"
done

start=$(now_ns)
for i in $(seq 1 "$NUM_FILES"); do
    mv "$B8_DIR/old_$i" "$B8_DIR/new_$i"
done
end=$(now_ns)
dur=$((end - start))

record "rename" "same_dir" "$NUM_FILES" "1" "0" "$NUM_FILES" "$dur"
rm -rf "$B8_DIR"
echo ""

# ==========================================================================
# B9: Mixed Workload (ref: IO500 composite score)
#
# Simulates a realistic mixed workload:
#   50% create + 30% stat + 20% delete
# This approximates the metadata operation distribution observed in
# production file system traces (Meta 2020: 60% write, 40% read).
# ==========================================================================
echo "── B9: Mixed Workload (50% create + 30% stat + 20% delete) ──"

B9_DIR="$TEST_BASE/b9_mixed"
mkdir -p "$B9_DIR"

# Pre-create some files for stat/delete operations
pre_create=$((NUM_FILES / 2))
for i in $(seq 1 "$pre_create"); do
    touch "$B9_DIR/pre_$i"
done

total_ops=$NUM_FILES
creates=$((total_ops * 50 / 100))
stats=$((total_ops * 30 / 100))
deletes=$((total_ops * 20 / 100))

start=$(now_ns)
# Creates
for i in $(seq 1 "$creates"); do
    touch "$B9_DIR/new_$i"
done
# Stats (cycle through existing files)
for i in $(seq 1 "$stats"); do
    idx=$(( (i % pre_create) + 1 ))
    stat "$B9_DIR/pre_$idx" > /dev/null 2>&1
done
# Deletes
for i in $(seq 1 "$deletes"); do
    rm "$B9_DIR/pre_$i" 2>/dev/null || true
done
end=$(now_ns)
dur=$((end - start))
actual_ops=$((creates + stats + deletes))

record "mixed" "50c_30s_20d" "$NUM_FILES" "1" "0" "$actual_ops" "$dur"
rm -rf "$B9_DIR"
echo ""

# ==========================================================================
# B10: Deep Tree Traversal (ref: SingularFS §6.4)
#
# Creates a deeply nested directory tree and measures path resolution
# latency. In KV-backed systems, each path component requires a
# directory entry lookup. Deep trees test the overhead of multi-hop
# lookups (which caching can mitigate).
# ==========================================================================
echo "── B10: Deep Tree (depth=$DEPTH) ──"
echo "   [Measures path resolution cost for deep hierarchies]"

B10_DIR="$TEST_BASE/b10_deep"
mkdir -p "$B10_DIR"

# Create deep tree
start=$(now_ns)
current="$B10_DIR"
for i in $(seq 1 "$DEPTH"); do
    current="$current/d$i"
    mkdir "$current"
done
end=$(now_ns)
dur=$((end - start))
record "deep_tree" "create_depth_${DEPTH}" "0" "$DEPTH" "$DEPTH" "$DEPTH" "$dur"

# Create a file at the deepest level
echo "deep" > "$current/leaf.txt"

# Measure access to deepest file
iterations=100
start=$(now_ns)
for _ in $(seq 1 $iterations); do
    stat "$current/leaf.txt" > /dev/null
done
end=$(now_ns)
dur=$((end - start))
record "deep_tree" "stat_depth_${DEPTH}" "1" "0" "$DEPTH" "$iterations" "$dur"

# Measure full path traversal
start=$(now_ns)
for _ in $(seq 1 $iterations); do
    cat "$current/leaf.txt" > /dev/null
done
end=$(now_ns)
dur=$((end - start))
record "deep_tree" "read_depth_${DEPTH}" "1" "0" "$DEPTH" "$iterations" "$dur"

rm -rf "$B10_DIR"
echo ""

# ==========================================================================
# B11: Create + Delete cycle (ref: TableFS create-then-remove throughput)
#
# Measures the throughput of the full create→delete lifecycle.
# This is important because many workloads (temp files, build systems)
# create and immediately remove files.
# ==========================================================================
echo "── B11: Create-Delete Cycle (N=$NUM_FILES) ──"

B11_DIR="$TEST_BASE/b11_cycle"
mkdir -p "$B11_DIR"

start=$(now_ns)
for i in $(seq 1 "$NUM_FILES"); do
    touch "$B11_DIR/f_$i"
    rm "$B11_DIR/f_$i"
done
end=$(now_ns)
dur=$((end - start))
# Each iteration = 1 create + 1 delete = 2 ops
total_ops=$((NUM_FILES * 2))

record "create_delete_cycle" "sequential" "$NUM_FILES" "1" "0" "$total_ops" "$dur"
rm -rf "$B11_DIR"
echo ""

# ==========================================================================
# Summary
# ==========================================================================
echo "══════════════════════════════════════════════════"
echo -e "${GREEN}Metadata benchmarks complete${NC}"
echo "  CSV:  $CSV_FILE"
echo "  Log:  $LOG_FILE"
echo "══════════════════════════════════════════════════"
echo ""
echo "Benchmark summary (from CSV):"
echo "──────────────────────────────────────────────────"
column -t -s',' "$CSV_FILE" 2>/dev/null || cat "$CSV_FILE"
