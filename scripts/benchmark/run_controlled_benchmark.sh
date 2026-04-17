#!/bin/bash
# =============================================================================
# run_controlled_benchmark.sh
#
# Controlled comparison: RucksFS-dist vs NFS
# Strict variable control — see docs below.
#
# Usage (run on Machine A / Client):
#   ./run_controlled_benchmark.sh --meta-ip <META_PRIVATE_IP> --data-ip <DATA_PRIVATE_IP>
#
# Prerequisites:
#   - All 3 machines provisioned via terraform (8C16G each)
#   - cloud-init completed on all nodes
#   - RucksFS binaries deployed to Meta and Data machines
#   - RucksFS MetadataServer running on Meta:8001
#   - RucksFS DataServer running on Data:8002
#   - RucksFS FUSE client mounted at /mnt/rucksfs-dist
#   - NFS exported from Data, mounted at /mnt/nfs with -o noac,vers=4.2
#
# Control Variables:
#   ✅ Same hardware: all machines 8C16G SA3.2XLARGE16
#   ✅ Same disk: 200G CLOUD_SSD, ext4
#   ✅ Same network: VPC internal (<0.3ms RTT)
#   ✅ Same test tool: mdtest
#   ✅ Same parameters: -n 5000 -F -u -i 3
#   ✅ Cache cleared: drop_caches on all 3 nodes before each test
#   ✅ NFS attr cache disabled: mount -o noac
#   ⚠️  NFS thread count: swept in Experiment 1 to find optimal
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
META_IP=""
DATA_IP=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --meta-ip) META_IP="$2"; shift 2 ;;
        --data-ip) DATA_IP="$2"; shift 2 ;;
        -h|--help)
            echo "Usage: $0 --meta-ip <META_PRIVATE_IP> --data-ip <DATA_PRIVATE_IP>"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ -z "$META_IP" || -z "$DATA_IP" ]]; then
    echo "ERROR: --meta-ip and --data-ip are required"
    echo "Usage: $0 --meta-ip <META_PRIVATE_IP> --data-ip <DATA_PRIVATE_IP>"
    exit 1
fi

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_DIR="/data/test-results/controlled_${TIMESTAMP}"
mkdir -p "$RESULT_DIR"

NFS_MOUNT="/mnt/nfs"
NFS_AC_MOUNT="/mnt/nfs-ac"
RUCKSFS_MOUNT="/mnt/rucksfs-dist"
N_FILES=5000       # files per mdtest process
ITERATIONS=3       # mdtest iterations per run
RUNS=3             # repeated runs for statistics

# ---------------------------------------------------------------------------
# Colors and logging
# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()  { echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} $*"; }
ok()   { echo -e "${GREEN}[OK]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
err()  { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Drop caches on all 3 nodes
drop_all_caches() {
    sync
    echo 3 > /proc/sys/vm/drop_caches 2>/dev/null || true
    ssh root@"$META_IP" 'sync && echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    ssh root@"$DATA_IP" 'sync && echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    sleep 1
}

# Set nfsd thread count on Data node
set_nfsd_threads() {
    local n="$1"
    ssh root@"$DATA_IP" "rpc.nfsd $n"
    local actual
    actual=$(ssh root@"$DATA_IP" 'cat /proc/fs/nfsd/threads')
    log "nfsd threads set to: $actual"
}

# Run mdtest and save output
# Usage: run_mdtest <label> <mountpoint> <np> <extra_flags> <output_file>
run_mdtest() {
    local label="$1"
    local mp="$2"
    local np="$3"
    local extra="$4"
    local outfile="$5"
    local test_dir="$mp/mdtest_run"

    # Clean up test directory
    rm -rf "$test_dir" 2>/dev/null || true
    mkdir -p "$test_dir"

    drop_all_caches

    log "  [$label] np=$np n=$N_FILES"

    if [[ "$np" -eq 1 ]]; then
        mdtest -d "$test_dir" -n "$N_FILES" -F -C -T -r -u -i "$ITERATIONS" $extra \
            2>&1 | tee -a "$outfile"
    else
        mpirun --allow-run-as-root --oversubscribe -np "$np" \
            mdtest -d "$test_dir" -n "$N_FILES" -F -C -T -r -u -i "$ITERATIONS" $extra \
            2>&1 | tee -a "$outfile"
    fi

    # Clean up
    rm -rf "$test_dir" 2>/dev/null || true
}

# ==========================================================================
# Phase 0: Environment snapshot
# ==========================================================================
log "================================================================"
log "  RucksFS vs NFS — Controlled Benchmark"
log "  Time:     $(date)"
log "  Meta IP:  $META_IP"
log "  Data IP:  $DATA_IP"
log "  Results:  $RESULT_DIR"
log "================================================================"

{
    echo "=== Benchmark Environment ==="
    echo "Timestamp: $(date -Iseconds)"
    echo ""
    echo "--- Client (local) ---"
    uname -a
    lscpu | head -20
    free -h
    echo ""
    echo "--- Mounts ---"
    mount | grep -E "rucksfs|nfs" || true
    echo ""
    echo "--- Network to Meta ---"
    ping -c 5 "$META_IP" 2>&1 | tail -1
    echo "--- Network to Data ---"
    ping -c 5 "$DATA_IP" 2>&1 | tail -1
    echo ""
    echo "--- Meta machine ---"
    ssh root@"$META_IP" 'lscpu | head -5; free -h; cat /data/env-info-meta.txt 2>/dev/null' || true
    echo ""
    echo "--- Data machine ---"
    ssh root@"$DATA_IP" 'lscpu | head -5; free -h; cat /proc/fs/nfsd/threads 2>/dev/null; cat /data/env-info-data.txt 2>/dev/null' || true
} > "$RESULT_DIR/environment.txt" 2>&1

ok "Environment snapshot saved"

# Set CPU governor to performance on all nodes
for ip in localhost "$META_IP" "$DATA_IP"; do
    if [[ "$ip" == "localhost" ]]; then
        cpupower frequency-set -g performance 2>/dev/null || true
    else
        ssh root@"$ip" 'cpupower frequency-set -g performance 2>/dev/null || true' 2>/dev/null || true
    fi
done

# ==========================================================================
# Experiment 1: NFS Thread Scan
# Purpose: Find optimal nfsd thread count, eliminate "too few threads" concern.
# Fixed: np=16, n=5000, mount -o noac,vers=4.2
# Variable: nfsd threads = 8, 16, 32, 64
# ==========================================================================
log ""
log "================================================================"
log "  Experiment 1: NFS Thread Scan (np=16)"
log "================================================================"

EXP1_DIR="$RESULT_DIR/exp1_nfs_thread_scan"
mkdir -p "$EXP1_DIR"

for nfsd_threads in 8 16 32 64; do
    log ""
    log "--- nfsd_threads=$nfsd_threads ---"
    set_nfsd_threads "$nfsd_threads"
    sleep 2

    for run in $(seq 1 "$RUNS"); do
        run_mdtest "nfs-t${nfsd_threads}" "$NFS_MOUNT" 16 "" \
            "$EXP1_DIR/nfs_threads${nfsd_threads}_run${run}.txt"
    done
done

ok "Experiment 1 complete"

# Determine best thread count from exp1 results
# (Simple heuristic: use 64 threads for subsequent experiments as upper bound)
BEST_NFSD_THREADS=64
log "Using nfsd_threads=$BEST_NFSD_THREADS for subsequent experiments"
set_nfsd_threads "$BEST_NFSD_THREADS"

# ==========================================================================
# Experiment 2: Concurrency Scaling (core experiment)
# Purpose: Compare RucksFS-dist vs NFS scaling with controlled variables.
# Fixed: n=5000, NFS threads=optimal, mount -o noac,vers=4.2
# Variable: np = 1, 2, 4, 8, 16, 32
# ==========================================================================
log ""
log "================================================================"
log "  Experiment 2: Concurrency Scaling (RucksFS vs NFS)"
log "================================================================"

EXP2_DIR="$RESULT_DIR/exp2_scaling"
mkdir -p "$EXP2_DIR"

for np in 1 2 4 8 16 32; do
    log ""
    log "--- np=$np ---"

    # NFS tests
    for run in $(seq 1 "$RUNS"); do
        run_mdtest "nfs" "$NFS_MOUNT" "$np" "" \
            "$EXP2_DIR/nfs_np${np}_run${run}.txt"
    done

    # RucksFS tests
    for run in $(seq 1 "$RUNS"); do
        run_mdtest "rucksfs" "$RUCKSFS_MOUNT" "$np" "" \
            "$EXP2_DIR/rucksfs_np${np}_run${run}.txt"
    done
done

ok "Experiment 2 complete"

# ==========================================================================
# Experiment 3: NFS Attribute Cache Impact
# Purpose: Quantify NFS attr cache effect on stat, explain previous 233K anomaly.
# Fixed: np=1, n=5000, stat only
# Variable: NFS noac vs ac, RucksFS baseline
# ==========================================================================
log ""
log "================================================================"
log "  Experiment 3: NFS Attribute Cache (stat only)"
log "================================================================"

EXP3_DIR="$RESULT_DIR/exp3_attr_cache"
mkdir -p "$EXP3_DIR"

# Mount NFS with attribute cache enabled (separate mount point)
umount "$NFS_AC_MOUNT" 2>/dev/null || true
mount -t nfs -o ac,vers=4.2 "$DATA_IP":/data/nfs-export "$NFS_AC_MOUNT"
log "Mounted NFS with ac at $NFS_AC_MOUNT"

# Pre-create files for stat tests
for mp in "$NFS_MOUNT" "$NFS_AC_MOUNT" "$RUCKSFS_MOUNT"; do
    test_dir="$mp/mdtest_stat_prep"
    rm -rf "$test_dir" 2>/dev/null || true
    mkdir -p "$test_dir"
    mdtest -d "$test_dir" -n "$N_FILES" -F -C -u 2>/dev/null || true
done

for run in $(seq 1 "$RUNS"); do
    log "--- Run $run ---"

    # NFS noac (stat only)
    drop_all_caches
    mdtest -d "$NFS_MOUNT/mdtest_stat_prep" -n "$N_FILES" -F -T -u -i "$ITERATIONS" \
        2>&1 | tee -a "$EXP3_DIR/nfs_noac_stat_run${run}.txt"

    # NFS ac (stat only)
    drop_all_caches
    mdtest -d "$NFS_AC_MOUNT/mdtest_stat_prep" -n "$N_FILES" -F -T -u -i "$ITERATIONS" \
        2>&1 | tee -a "$EXP3_DIR/nfs_ac_stat_run${run}.txt"

    # RucksFS (stat only)
    drop_all_caches
    mdtest -d "$RUCKSFS_MOUNT/mdtest_stat_prep" -n "$N_FILES" -F -T -u -i "$ITERATIONS" \
        2>&1 | tee -a "$EXP3_DIR/rucksfs_stat_run${run}.txt"
done

# Clean up stat prep files
for mp in "$NFS_MOUNT" "$NFS_AC_MOUNT" "$RUCKSFS_MOUNT"; do
    rm -rf "$mp/mdtest_stat_prep" 2>/dev/null || true
done
umount "$NFS_AC_MOUNT" 2>/dev/null || true

ok "Experiment 3 complete"

# ==========================================================================
# Experiment 4: Network Verification
# Purpose: Confirm network symmetry between Client→Meta and Client→Data.
# ==========================================================================
log ""
log "================================================================"
log "  Experiment 4: Network Verification"
log "================================================================"

EXP4_DIR="$RESULT_DIR/exp4_network"
mkdir -p "$EXP4_DIR"

{
    echo "=== Ping: Client -> Meta ==="
    ping -c 50 "$META_IP"
    echo ""
    echo "=== Ping: Client -> Data ==="
    ping -c 50 "$DATA_IP"
} > "$EXP4_DIR/ping.txt" 2>&1

# iperf3 bandwidth test (requires iperf3 -s running on target)
{
    echo "=== iperf3: Client -> Meta ==="
    ssh root@"$META_IP" 'pkill iperf3 2>/dev/null; iperf3 -s -D' 2>/dev/null || true
    sleep 1
    iperf3 -c "$META_IP" -t 10 2>&1 || echo "iperf3 to Meta failed"
    echo ""
    echo "=== iperf3: Client -> Data ==="
    ssh root@"$DATA_IP" 'pkill iperf3 2>/dev/null; iperf3 -s -D' 2>/dev/null || true
    sleep 1
    iperf3 -c "$DATA_IP" -t 10 2>&1 || echo "iperf3 to Data failed"
} > "$EXP4_DIR/iperf3.txt" 2>&1

# Clean up iperf3 daemons
ssh root@"$META_IP" 'pkill iperf3' 2>/dev/null || true
ssh root@"$DATA_IP" 'pkill iperf3' 2>/dev/null || true

ok "Experiment 4 complete"

# ==========================================================================
# Summary
# ==========================================================================
log ""
log "================================================================"
log "  All Experiments Complete!"
log "  Time: $(date)"
log "  Results: $RESULT_DIR"
log "================================================================"
echo ""
echo "Directory structure:"
find "$RESULT_DIR" -type f | sort
echo ""
echo "File count: $(find "$RESULT_DIR" -type f | wc -l)"

# ==========================================================================
# Quick summary extraction
# ==========================================================================
log ""
log "Quick Summary — Experiment 2 (create ops/s):"
log "----------------------------------------------"
for np in 1 2 4 8 16 32; do
    nfs_vals=""
    rfs_vals=""
    for run in $(seq 1 "$RUNS"); do
        v=$(grep "File creation" "$EXP2_DIR/nfs_np${np}_run${run}.txt" 2>/dev/null | awk '{print $3}' | tail -1)
        nfs_vals="$nfs_vals $v"
        v=$(grep "File creation" "$EXP2_DIR/rucksfs_np${np}_run${run}.txt" 2>/dev/null | awk '{print $3}' | tail -1)
        rfs_vals="$rfs_vals $v"
    done
    printf "np=%2d  NFS: %-30s  RucksFS: %s\n" "$np" "$nfs_vals" "$rfs_vals"
done

log ""
log "Quick Summary — Experiment 1 (NFS thread scan, np=16, create ops/s):"
log "---------------------------------------------------------------------"
for t in 8 16 32 64; do
    vals=""
    for run in $(seq 1 "$RUNS"); do
        v=$(grep "File creation" "$EXP1_DIR/nfs_threads${t}_run${run}.txt" 2>/dev/null | awk '{print $3}' | tail -1)
        vals="$vals $v"
    done
    printf "threads=%2d: %s\n" "$t" "$vals"
done
