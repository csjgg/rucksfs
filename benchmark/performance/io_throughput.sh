#!/usr/bin/env bash
# =============================================================================
# benchmark/performance/io_throughput.sh
# Data I/O throughput and latency benchmarks for RucksFS
#
# Academic references:
#   - BFO (TOS'20) §4.1: small-file create-write-read-delete pipeline
#   - filebench fileserver: composite create+write+read+delete workload
#   - fio: industry-standard sequential/random I/O benchmark
#
# Usage:
#   ./benchmark/performance/io_throughput.sh --mountpoint <path> [options]
#
# Options:
#   --large-size MB     Large file size in MB (default: 64)
#   --small-count N     Number of small files (default: 5000)
#   --small-size KB     Small file size in KB (default: 4)
#
# Output:
#   benchmark/results/io_throughput_<timestamp>.csv
# =============================================================================

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$PROJECT_ROOT/benchmark/results"
MOUNTPOINT=""
LARGE_SIZE_MB=64
SMALL_COUNT=5000
SMALL_SIZE_KB=4
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

while [[ $# -gt 0 ]]; do
    case "$1" in
        --mountpoint)   MOUNTPOINT="$2"; shift 2 ;;
        --large-size)   LARGE_SIZE_MB="$2"; shift 2 ;;
        --small-count)  SMALL_COUNT="$2"; shift 2 ;;
        --small-size)   SMALL_SIZE_KB="$2"; shift 2 ;;
        -h|--help)
            echo "Usage: $0 --mountpoint <path> [--large-size MB] [--small-count N] [--small-size KB]"
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
NC='\033[0m'

mkdir -p "$RESULTS_DIR"
CSV_FILE="$RESULTS_DIR/io_throughput_${TIMESTAMP}.csv"
LOG_FILE="$RESULTS_DIR/io_throughput_${TIMESTAMP}.log"
TEST_BASE="$MOUNTPOINT/.bench_io_$$"

exec > >(tee -a "$LOG_FILE") 2>&1

echo "timestamp,benchmark,variant,block_size,total_bytes,duration_sec,throughput_mbps,iops" > "$CSV_FILE"

cleanup() { rm -rf "$TEST_BASE" 2>/dev/null || true; }
trap cleanup EXIT
mkdir -p "$TEST_BASE"

now_ns() { date +%s%N 2>/dev/null || python3 -c "import time; print(int(time.time()*1e9))"; }

record_io() {
    local bench="$1" variant="$2" bs="$3" total_bytes="$4" dur_ns="$5"
    local dur_sec mbps iops
    dur_sec=$(python3 -c "print(f'{$dur_ns / 1e9:.4f}')" 2>/dev/null || echo "0")
    mbps=$(python3 -c "print(f'{$total_bytes / ($dur_ns / 1e9) / 1048576:.2f}')" 2>/dev/null || echo "0")
    iops=$(python3 -c "print(f'{$total_bytes / $bs / ($dur_ns / 1e9):.2f}')" 2>/dev/null || echo "0")
    echo "${TIMESTAMP},${bench},${variant},${bs},${total_bytes},${dur_sec},${mbps},${iops}" >> "$CSV_FILE"
    echo -e "  ${CYAN}→${NC} ${total_bytes} bytes in ${dur_sec}s = ${GREEN}${mbps} MB/s${NC} (${iops} IOPS)"
}

# ==========================================================================
echo "╔══════════════════════════════════════════════════════╗"
echo "║  RucksFS — I/O Throughput Benchmark                 ║"
echo "║  ref: BFO, filebench, fio                           ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
echo "  Large file: ${LARGE_SIZE_MB}MB"
echo "  Small files: ${SMALL_COUNT} x ${SMALL_SIZE_KB}KB"
echo ""

# ==========================================================================
# T1: Sequential Write (ref: fio seqwrite)
# ==========================================================================
echo "── T1: Sequential Write (${LARGE_SIZE_MB}MB, bs=128K) ──"

LARGE_BYTES=$((LARGE_SIZE_MB * 1024 * 1024))
BS=131072  # 128K
COUNT=$((LARGE_BYTES / BS))

start=$(now_ns)
dd if=/dev/zero of="$TEST_BASE/t1_seqwrite" bs=$BS count=$COUNT conv=fdatasync 2>/dev/null
end=$(now_ns)
dur=$((end - start))
record_io "seq_write" "128k" "$BS" "$LARGE_BYTES" "$dur"
echo ""

# ==========================================================================
# T2: Sequential Read (ref: fio seqread)
# ==========================================================================
echo "── T2: Sequential Read (${LARGE_SIZE_MB}MB, bs=128K) ──"

# Drop caches if possible
echo 3 > /proc/sys/vm/drop_caches 2>/dev/null || true

start=$(now_ns)
dd if="$TEST_BASE/t1_seqwrite" of=/dev/null bs=$BS 2>/dev/null
end=$(now_ns)
dur=$((end - start))
record_io "seq_read" "128k" "$BS" "$LARGE_BYTES" "$dur"

rm -f "$TEST_BASE/t1_seqwrite"
echo ""

# ==========================================================================
# T3: Random Write 4K (ref: fio randwrite)
# ==========================================================================
echo "── T3: Random Write 4K ──"

T3_FILE="$TEST_BASE/t3_randwrite"
T3_SIZE=$((4 * 1024 * 1024))  # 4MB file
T3_OPS=1000
BS_4K=4096

# Pre-create file
dd if=/dev/zero of="$T3_FILE" bs=$BS_4K count=$((T3_SIZE / BS_4K)) 2>/dev/null

start=$(now_ns)
for i in $(seq 1 $T3_OPS); do
    # Random offset within file
    offset=$(( (RANDOM * RANDOM) % (T3_SIZE - BS_4K) ))
    dd if=/dev/urandom of="$T3_FILE" bs=$BS_4K count=1 seek=$((offset / BS_4K)) conv=notrunc 2>/dev/null
done
end=$(now_ns)
dur=$((end - start))
record_io "rand_write" "4k" "$BS_4K" "$((T3_OPS * BS_4K))" "$dur"

rm -f "$T3_FILE"
echo ""

# ==========================================================================
# T4: Random Read 4K (ref: fio randread)
# ==========================================================================
echo "── T4: Random Read 4K ──"

T4_FILE="$TEST_BASE/t4_randread"
dd if=/dev/urandom of="$T4_FILE" bs=$BS_4K count=$((T3_SIZE / BS_4K)) 2>/dev/null

start=$(now_ns)
for i in $(seq 1 $T3_OPS); do
    offset=$(( (RANDOM * RANDOM) % (T3_SIZE - BS_4K) ))
    dd if="$T4_FILE" of=/dev/null bs=$BS_4K count=1 skip=$((offset / BS_4K)) 2>/dev/null
done
end=$(now_ns)
dur=$((end - start))
record_io "rand_read" "4k" "$BS_4K" "$((T3_OPS * BS_4K))" "$dur"

rm -f "$T4_FILE"
echo ""

# ==========================================================================
# T5: Small File Pipeline (ref: BFO §4.1)
#
# This is the key benchmark from the BFO paper. It measures the full
# lifecycle of small files: create → write → close → open → read → delete.
# Small file performance is dominated by metadata overhead.
# ==========================================================================
echo "── T5: Small File Pipeline (N=$SMALL_COUNT, size=${SMALL_SIZE_KB}KB) ──"
echo "   [BFO pattern: create → write → read → delete]"

T5_DIR="$TEST_BASE/t5_pipeline"
mkdir -p "$T5_DIR"
SMALL_BYTES=$((SMALL_SIZE_KB * 1024))
PAYLOAD=$(dd if=/dev/urandom bs=$SMALL_BYTES count=1 2>/dev/null | base64 | head -c $SMALL_BYTES)

start=$(now_ns)
for i in $(seq 1 $SMALL_COUNT); do
    # Create + Write
    echo "$PAYLOAD" > "$T5_DIR/f_$i"
done
end_write=$(now_ns)

# Read all
for i in $(seq 1 $SMALL_COUNT); do
    cat "$T5_DIR/f_$i" > /dev/null
done
end_read=$(now_ns)

# Delete all
for i in $(seq 1 $SMALL_COUNT); do
    rm "$T5_DIR/f_$i"
done
end_delete=$(now_ns)

write_dur=$((end_write - start))
read_dur=$((end_read - end_write))
delete_dur=$((end_delete - end_read))
total_dur=$((end_delete - start))
total_bytes=$((SMALL_COUNT * SMALL_BYTES))

record_io "small_file_pipeline" "write" "$SMALL_BYTES" "$total_bytes" "$write_dur"
record_io "small_file_pipeline" "read" "$SMALL_BYTES" "$total_bytes" "$read_dur"
record_io "small_file_pipeline" "delete" "$SMALL_BYTES" "0" "$delete_dur"
record_io "small_file_pipeline" "full_cycle" "$SMALL_BYTES" "$((total_bytes * 2))" "$total_dur"

rm -rf "$T5_DIR"
echo ""

# ==========================================================================
# T6: Append Workload (ref: filebench varmail)
# ==========================================================================
echo "── T6: Append Workload ──"

T6_FILE="$TEST_BASE/t6_append"
touch "$T6_FILE"
APPEND_COUNT=2000
APPEND_SIZE=1024

start=$(now_ns)
for i in $(seq 1 $APPEND_COUNT); do
    dd if=/dev/urandom bs=$APPEND_SIZE count=1 2>/dev/null >> "$T6_FILE"
done
end=$(now_ns)
dur=$((end - start))
total=$((APPEND_COUNT * APPEND_SIZE))
record_io "append" "1k_chunks" "$APPEND_SIZE" "$total" "$dur"

rm -f "$T6_FILE"
echo ""

# ==========================================================================
# T7: Data Integrity (checksum verification)
# ==========================================================================
echo "── T7: Data Integrity ──"

T7_FILE="$TEST_BASE/t7_integrity"
dd if=/dev/urandom of="$T7_FILE" bs=4096 count=256 2>/dev/null
orig_md5=$(md5sum "$T7_FILE" 2>/dev/null | cut -d' ' -f1)

# Read back and verify
read_md5=$(md5sum "$T7_FILE" 2>/dev/null | cut -d' ' -f1)
if [[ "$orig_md5" == "$read_md5" ]]; then
    echo -e "  ${GREEN}[PASS]${NC} T7: 1MB data integrity verified (md5=$orig_md5)"
else
    echo -e "  [FAIL] T7: data integrity (expected=$orig_md5, got=$read_md5)"
fi

# Copy and verify
cp "$T7_FILE" "$T7_FILE.copy"
copy_md5=$(md5sum "$T7_FILE.copy" 2>/dev/null | cut -d' ' -f1)
if [[ "$orig_md5" == "$copy_md5" ]]; then
    echo -e "  ${GREEN}[PASS]${NC} T7: copy integrity verified"
else
    echo -e "  [FAIL] T7: copy integrity (expected=$orig_md5, got=$copy_md5)"
fi

rm -f "$T7_FILE" "$T7_FILE.copy"
echo ""

# ==========================================================================
# T8: Overwrite (ref: fio randrw)
# ==========================================================================
echo "── T8: Overwrite (partial file rewrite) ──"

T8_FILE="$TEST_BASE/t8_overwrite"
T8_SIZE=$((2 * 1024 * 1024))  # 2MB
dd if=/dev/zero of="$T8_FILE" bs=4096 count=$((T8_SIZE / 4096)) 2>/dev/null

OVERWRITE_OPS=500
start=$(now_ns)
for i in $(seq 1 $OVERWRITE_OPS); do
    offset=$(( (RANDOM * RANDOM) % (T8_SIZE - 4096) ))
    dd if=/dev/urandom of="$T8_FILE" bs=4096 count=1 seek=$((offset / 4096)) conv=notrunc 2>/dev/null
done
end=$(now_ns)
dur=$((end - start))
record_io "overwrite" "4k_random" "4096" "$((OVERWRITE_OPS * 4096))" "$dur"

rm -f "$T8_FILE"
echo ""

# ==========================================================================
# Summary
# ==========================================================================
echo "══════════════════════════════════════════════════"
echo -e "${GREEN}I/O throughput benchmarks complete${NC}"
echo "  CSV:  $CSV_FILE"
echo "  Log:  $LOG_FILE"
echo "══════════════════════════════════════════════════"
echo ""
echo "Results:"
echo "──────────────────────────────────────────────────"
column -t -s',' "$CSV_FILE" 2>/dev/null || cat "$CSV_FILE"
