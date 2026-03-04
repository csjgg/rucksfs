#!/usr/bin/env bash
# =============================================================================
# benchmark/performance/concurrent_stress.sh
# Concurrency and scalability benchmarks for RucksFS
#
# Academic references:
#   - SingularFS (ATC'23) §6.1: concurrent file creation scaling
#   - SingularFS (ATC'23) §6.2: shared-directory contention
#   - SingularFS (ATC'23) §6.6: thread scaling factor
#   - LocoFS (SC'17) §6: concurrent metadata operations
#   - mdtest-hard: shared-directory concurrent create
#
# Usage:
#   ./benchmark/performance/concurrent_stress.sh --mountpoint <path> [options]
#
# Options:
#   --num-files N      Files per thread (default: 1000)
#   --max-threads T    Maximum thread count (default: 16)
#
# Output:
#   benchmark/results/concurrent_stress_<timestamp>.csv
# =============================================================================

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$PROJECT_ROOT/benchmark/results"
MOUNTPOINT=""
NUM_FILES=1000
MAX_THREADS=16
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

while [[ $# -gt 0 ]]; do
    case "$1" in
        --mountpoint)    MOUNTPOINT="$2"; shift 2 ;;
        --num-files)     NUM_FILES="$2"; shift 2 ;;
        --max-threads)   MAX_THREADS="$2"; shift 2 ;;
        -h|--help)
            echo "Usage: $0 --mountpoint <path> [--num-files N] [--max-threads T]"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ -z "$MOUNTPOINT" ]]; then echo "ERROR: --mountpoint required"; exit 1; fi
if [[ ! -d "$MOUNTPOINT" ]]; then echo "ERROR: $MOUNTPOINT not found"; exit 1; fi

# ---------------------------------------------------------------------------
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
NC='\033[0m'

mkdir -p "$RESULTS_DIR"
CSV_FILE="$RESULTS_DIR/concurrent_stress_${TIMESTAMP}.csv"
LOG_FILE="$RESULTS_DIR/concurrent_stress_${TIMESTAMP}.log"
TEST_BASE="$MOUNTPOINT/.bench_conc_$$"

exec > >(tee -a "$LOG_FILE") 2>&1

echo "timestamp,benchmark,variant,num_files_per_thread,num_threads,ops_total,duration_sec,agg_ops_per_sec" > "$CSV_FILE"

cleanup() { rm -rf "$TEST_BASE" 2>/dev/null || true; }
trap cleanup EXIT
mkdir -p "$TEST_BASE"

now_ns() { date +%s%N 2>/dev/null || python3 -c "import time; print(int(time.time()*1e9))"; }

record() {
    local bench="$1" variant="$2" nf="$3" nt="$4" ops="$5" dur_ns="$6"
    local dur_sec opsec
    dur_sec=$(python3 -c "print(f'{$dur_ns / 1e9:.4f}')" 2>/dev/null || echo "0")
    opsec=$(python3 -c "print(f'{$ops / ($dur_ns / 1e9):.2f}')" 2>/dev/null || echo "0")
    echo "${TIMESTAMP},${bench},${variant},${nf},${nt},${ops},${dur_sec},${opsec}" >> "$CSV_FILE"
    echo -e "  ${CYAN}→${NC} ${nt} threads × ${nf} files = ${ops} ops in ${dur_sec}s = ${GREEN}${opsec} agg ops/sec${NC}"
}

# Worker function: create N files in a directory
worker_create() {
    local dir="$1" count="$2" prefix="$3"
    for i in $(seq 1 "$count"); do
        touch "$dir/${prefix}_$i"
    done
}

# Worker function: stat N files
worker_stat() {
    local dir="$1" count="$2" prefix="$3"
    for i in $(seq 1 "$count"); do
        stat "$dir/${prefix}_$i" > /dev/null 2>&1
    done
}

# Worker function: delete N files
worker_delete() {
    local dir="$1" count="$2" prefix="$3"
    for i in $(seq 1 "$count"); do
        rm "$dir/${prefix}_$i" 2>/dev/null || true
    done
}

# Worker function: create+delete cycle
worker_create_delete() {
    local dir="$1" count="$2" prefix="$3"
    for i in $(seq 1 "$count"); do
        touch "$dir/${prefix}_$i"
        rm "$dir/${prefix}_$i"
    done
}

# Worker function: rename
worker_rename() {
    local dir="$1" count="$2" prefix="$3"
    for i in $(seq 1 "$count"); do
        mv "$dir/${prefix}_old_$i" "$dir/${prefix}_new_$i" 2>/dev/null || true
    done
}

# ==========================================================================
echo "╔══════════════════════════════════════════════════════╗"
echo "║  RucksFS — Concurrent Stress Benchmark              ║"
echo "║  ref: SingularFS, LocoFS, mdtest                    ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
echo "  Files/thread: $NUM_FILES"
echo "  Max threads:  $MAX_THREADS"
echo ""

# ==========================================================================
# C1: Concurrent Create — Private Directories (ref: SingularFS §6.1)
#
# Each thread operates in its own directory (no contention).
# This measures the maximum achievable metadata throughput.
# Equivalent to mdtest-easy.
# ==========================================================================
echo "══ C1: Concurrent Create — Private Dirs (mdtest-easy) ══"

for nthreads in 1 2 4 8 16; do
    if [[ $nthreads -gt $MAX_THREADS ]]; then continue; fi

    C1_DIR="$TEST_BASE/c1_private_${nthreads}"
    mkdir -p "$C1_DIR"

    # Pre-create per-thread directories
    for t in $(seq 1 $nthreads); do
        mkdir -p "$C1_DIR/t$t"
    done

    start=$(now_ns)
    for t in $(seq 1 $nthreads); do
        worker_create "$C1_DIR/t$t" "$NUM_FILES" "f" &
    done
    wait
    end=$(now_ns)
    dur=$((end - start))
    total=$((nthreads * NUM_FILES))

    record "concurrent_create" "private_dirs" "$NUM_FILES" "$nthreads" "$total" "$dur"
    rm -rf "$C1_DIR"
done
echo ""

# ==========================================================================
# C2: Concurrent Create — Shared Directory (ref: mdtest-hard)
#
# All threads create files in the SAME directory.
# This stresses the directory-level lock (per-directory Mutex in RucksFS).
# The ratio C2/C1 reveals lock contention overhead.
# ==========================================================================
echo "══ C2: Concurrent Create — Shared Dir (mdtest-hard) ══"

for nthreads in 1 2 4 8 16; do
    if [[ $nthreads -gt $MAX_THREADS ]]; then continue; fi

    C2_DIR="$TEST_BASE/c2_shared_${nthreads}"
    mkdir -p "$C2_DIR"

    start=$(now_ns)
    for t in $(seq 1 $nthreads); do
        worker_create "$C2_DIR" "$NUM_FILES" "t${t}" &
    done
    wait
    end=$(now_ns)
    dur=$((end - start))
    total=$((nthreads * NUM_FILES))

    record "concurrent_create" "shared_dir" "$NUM_FILES" "$nthreads" "$total" "$dur"
    rm -rf "$C2_DIR"
done
echo ""

# ==========================================================================
# C3: Concurrent Read/Write Mix (ref: SingularFS §6.5)
#
# Half the threads write new files, the other half read existing files.
# This tests whether read operations are blocked by concurrent writes
# (important for delta-based update systems like RucksFS).
# ==========================================================================
echo "══ C3: Concurrent Read/Write Mix ══"

C3_DIR="$TEST_BASE/c3_rw"
mkdir -p "$C3_DIR"

# Pre-create files for readers
for i in $(seq 1 $NUM_FILES); do
    echo "read_data_$i" > "$C3_DIR/read_$i"
done

nthreads=8
if [[ $nthreads -gt $MAX_THREADS ]]; then nthreads=$MAX_THREADS; fi
writers=$((nthreads / 2))
readers=$((nthreads - writers))

start=$(now_ns)
# Writers
for t in $(seq 1 $writers); do
    worker_create "$C3_DIR" "$NUM_FILES" "w${t}" &
done
# Readers
for t in $(seq 1 $readers); do
    worker_stat "$C3_DIR" "$NUM_FILES" "read" &
done
wait
end=$(now_ns)
dur=$((end - start))
total=$((nthreads * NUM_FILES))

record "concurrent_rw" "half_write_half_read" "$NUM_FILES" "$nthreads" "$total" "$dur"
rm -rf "$C3_DIR"
echo ""

# ==========================================================================
# C4: Create-Delete Storm (ref: build system workload)
#
# Each thread creates a file and immediately deletes it.
# This is common in build systems (temp files) and measures the overhead
# of the full create→delete round-trip under contention.
# ==========================================================================
echo "══ C4: Create-Delete Storm ══"

for nthreads in 1 2 4 8; do
    if [[ $nthreads -gt $MAX_THREADS ]]; then continue; fi

    C4_DIR="$TEST_BASE/c4_storm_${nthreads}"
    mkdir -p "$C4_DIR"

    start=$(now_ns)
    for t in $(seq 1 $nthreads); do
        worker_create_delete "$C4_DIR" "$NUM_FILES" "s${t}" &
    done
    wait
    end=$(now_ns)
    dur=$((end - start))
    total=$((nthreads * NUM_FILES * 2))  # create + delete = 2 ops each

    record "create_delete_storm" "shared_dir" "$NUM_FILES" "$nthreads" "$total" "$dur"
    rm -rf "$C4_DIR"
done
echo ""

# ==========================================================================
# C5: Concurrent Rename (ref: SingularFS §6.3)
#
# Multiple threads rename different files simultaneously.
# Rename is the most complex metadata operation (potentially cross-dir
# with atomicity requirements), so concurrent rename tests are crucial.
# ==========================================================================
echo "══ C5: Concurrent Rename ══"

for nthreads in 1 2 4 8; do
    if [[ $nthreads -gt $MAX_THREADS ]]; then continue; fi

    C5_DIR="$TEST_BASE/c5_rename_${nthreads}"
    mkdir -p "$C5_DIR"

    # Pre-create files for renaming
    for t in $(seq 1 $nthreads); do
        for i in $(seq 1 "$NUM_FILES"); do
            touch "$C5_DIR/t${t}_old_$i"
        done
    done

    start=$(now_ns)
    for t in $(seq 1 $nthreads); do
        worker_rename "$C5_DIR" "$NUM_FILES" "t${t}" &
    done
    wait
    end=$(now_ns)
    dur=$((end - start))
    total=$((nthreads * NUM_FILES))

    record "concurrent_rename" "same_dir" "$NUM_FILES" "$nthreads" "$total" "$dur"
    rm -rf "$C5_DIR"
done
echo ""

# ==========================================================================
# C6: Thread Scaling (ref: SingularFS §6.6)
#
# Run the same benchmark (file create in private dirs) with increasing
# thread counts and compute the scaling factor.
# Ideal scaling = N× for N threads.
# ==========================================================================
echo "══ C6: Thread Scaling Analysis ══"
echo "   [Measures how throughput scales with thread count]"

baseline_opsec=""
for nthreads in 1 2 4 8 16; do
    if [[ $nthreads -gt $MAX_THREADS ]]; then continue; fi

    C6_DIR="$TEST_BASE/c6_scale_${nthreads}"
    mkdir -p "$C6_DIR"
    for t in $(seq 1 $nthreads); do
        mkdir -p "$C6_DIR/t$t"
    done

    start=$(now_ns)
    for t in $(seq 1 $nthreads); do
        worker_create "$C6_DIR/t$t" "$NUM_FILES" "f" &
    done
    wait
    end=$(now_ns)
    dur=$((end - start))
    total=$((nthreads * NUM_FILES))
    dur_sec=$(python3 -c "print(f'{$dur / 1e9:.4f}')" 2>/dev/null || echo "0")
    opsec=$(python3 -c "print(f'{$total / ($dur / 1e9):.2f}')" 2>/dev/null || echo "0")

    if [[ -z "$baseline_opsec" ]]; then
        baseline_opsec="$opsec"
    fi

    scaling=$(python3 -c "print(f'{float($opsec) / float($baseline_opsec):.2f}')" 2>/dev/null || echo "N/A")
    echo -e "  ${nthreads} threads: ${GREEN}${opsec} ops/sec${NC}  (scaling: ${YELLOW}${scaling}x${NC} vs single-thread)"

    echo "${TIMESTAMP},thread_scaling,${nthreads}_threads,${NUM_FILES},${nthreads},${total},${dur_sec},${opsec}" >> "$CSV_FILE"
    rm -rf "$C6_DIR"
done
echo ""

# ==========================================================================
# C7: Lock Contention — All threads on same directory (ref: SingularFS §6.2)
#
# This is the worst-case scenario: all threads perform interleaved
# create/stat/delete on the same directory. Measures how well the
# per-directory lock handles high contention.
# ==========================================================================
echo "══ C7: Lock Contention — Same Dir Mixed Ops ══"

for nthreads in 1 2 4 8; do
    if [[ $nthreads -gt $MAX_THREADS ]]; then continue; fi

    C7_DIR="$TEST_BASE/c7_lock_${nthreads}"
    mkdir -p "$C7_DIR"

    # Each thread does create, stat, then delete (3 ops per file)
    mixed_worker() {
        local dir="$1" count="$2" prefix="$3"
        for i in $(seq 1 "$count"); do
            touch "$dir/${prefix}_$i"
            stat "$dir/${prefix}_$i" > /dev/null 2>&1
            rm "$dir/${prefix}_$i" 2>/dev/null || true
        done
    }

    # Export for subshells
    export -f mixed_worker 2>/dev/null || true

    start=$(now_ns)
    for t in $(seq 1 $nthreads); do
        (
            for i in $(seq 1 "$NUM_FILES"); do
                touch "$C7_DIR/t${t}_$i"
                stat "$C7_DIR/t${t}_$i" > /dev/null 2>&1
                rm "$C7_DIR/t${t}_$i" 2>/dev/null || true
            done
        ) &
    done
    wait
    end=$(now_ns)
    dur=$((end - start))
    total=$((nthreads * NUM_FILES * 3))  # 3 ops per file

    record "lock_contention" "mixed_same_dir" "$NUM_FILES" "$nthreads" "$total" "$dur"
    rm -rf "$C7_DIR"
done
echo ""

# ==========================================================================
# Summary
# ==========================================================================
echo "══════════════════════════════════════════════════"
echo -e "${GREEN}Concurrent stress benchmarks complete${NC}"
echo "  CSV:  $CSV_FILE"
echo "  Log:  $LOG_FILE"
echo "══════════════════════════════════════════════════"
echo ""
echo "Results:"
echo "──────────────────────────────────────────────────"
column -t -s',' "$CSV_FILE" 2>/dev/null || cat "$CSV_FILE"
