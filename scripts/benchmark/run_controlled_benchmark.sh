#!/bin/bash
# =============================================================================
# Controlled RucksFS vs NFS Benchmark — v2 (symmetric 2-server topology)
#
# Topology:
#   Client (8C16G) ── Server-1 (8C16G, RucksFS MDS+DS, dedicated)
#                  ── Server-2 (8C16G, NFS nfsd+ext4, dedicated)
#
# Key improvements over v1:
#   - Each filesystem has its own dedicated server (no resource sharing)
#   - RucksFS MDS+DS on same machine (symmetric with NFS all-in-one)
#   - Tests run SERIALLY (NFS first, then RucksFS) to avoid cross-interference
#
# Usage:
#   ./run_controlled_benchmark.sh --server1-ip <RucksFS_IP> --server2-ip <NFS_IP>
# =============================================================================

set -uo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
SERVER1_IP=""  # RucksFS (MDS+DS)
SERVER2_IP=""  # NFS

while [[ $# -gt 0 ]]; do
    case "$1" in
        --server1-ip) SERVER1_IP="$2"; shift 2 ;;
        --server2-ip) SERVER2_IP="$2"; shift 2 ;;
        -h|--help) echo "Usage: $0 --server1-ip <RucksFS_IP> --server2-ip <NFS_IP>"; exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ -z "$SERVER1_IP" || -z "$SERVER2_IP" ]]; then
    echo "ERROR: --server1-ip and --server2-ip required"
    exit 1
fi

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_DIR="/data/test-results/controlled_v2_${TIMESTAMP}"
mkdir -p "$RESULT_DIR"

NFS_MOUNT="/mnt/nfs"
RUCKSFS_MOUNT="/mnt/rucksfs-dist"
N_FILES=5000
ITERATIONS=3
RUNS=3
THREAD_SCAN_NP=16                # np used during NFS thread scan
THREAD_CANDIDATES="8 16 32 64"   # nfsd thread counts to evaluate

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
CYAN='\033[0;36m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
log()  { echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} $*"; }
ok()   { echo -e "${GREEN}[OK]${NC} $*"; }
err()  { echo -e "${RED}[ERROR]${NC} $*" >&2; }

set_nfsd_threads() {
    local n="$1"
    ssh root@"$SERVER2_IP" "rpc.nfsd $n"
    local actual
    actual=$(ssh root@"$SERVER2_IP" 'cat /proc/fs/nfsd/threads')
    log "  nfsd threads set to $actual"
}

# Extract the max "File creation" ops/s from an mdtest output file
extract_create_ops() {
    grep "File creation" "$1" 2>/dev/null | awk '{print $3}' | sort -n | tail -1
}

drop_all_caches() {
    sync
    echo 3 > /proc/sys/vm/drop_caches 2>/dev/null || true
    ssh root@"$SERVER1_IP" 'sync && echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    ssh root@"$SERVER2_IP" 'sync && echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    sleep 1
}

clean_test_dir() {
    local mp="$1"
    local test_dir="$mp/mdtest_run"
    # Clean client-side
    rm -rf "$test_dir" 2>/dev/null || true
    # If NFS, also clean server-side to handle stale handles
    if [[ "$mp" == "$NFS_MOUNT" ]]; then
        ssh root@"$SERVER2_IP" 'find /data/nfs-export/mdtest_run -type f -delete 2>/dev/null; find /data/nfs-export/mdtest_run -depth -type d -delete 2>/dev/null; rm -rf /data/nfs-export/mdtest_run 2>/dev/null' || true
    fi
    if [[ "$mp" == "$RUCKSFS_MOUNT" ]]; then
        ssh root@"$SERVER1_IP" 'rm -rf /data/rucksfs-meta/mdtest_run 2>/dev/null' || true
    fi
}

run_mdtest() {
    local label="$1" mp="$2" np="$3" outfile="$4"
    local test_dir="$mp/mdtest_run"

    clean_test_dir "$mp"
    mkdir -p "$test_dir"
    drop_all_caches

    log "  [$label] np=$np n=$N_FILES"
    if [[ "$np" -eq 1 ]]; then
        mdtest -d "$test_dir" -n "$N_FILES" -F -C -T -r -u -i "$ITERATIONS" \
            2>&1 | tee -a "$outfile"
    else
        mpirun --allow-run-as-root --oversubscribe -np "$np" \
            mdtest -d "$test_dir" -n "$N_FILES" -F -C -T -r -u -i "$ITERATIONS" \
            2>&1 | tee -a "$outfile"
    fi

    clean_test_dir "$mp"
}

# ==========================================================================
# Environment snapshot
# ==========================================================================
log "================================================================"
log "  RucksFS vs NFS — Controlled Benchmark v2"
log "  Topology: symmetric 2-server (dedicated machines)"
log "  Server-1 (RucksFS): $SERVER1_IP"
log "  Server-2 (NFS):     $SERVER2_IP"
log "  Results:            $RESULT_DIR"
log "================================================================"

{
    echo "=== Benchmark Environment (v2) ==="
    echo "Timestamp: $(date -Iseconds)"
    echo "Topology: Client + Server-1(RucksFS) + Server-2(NFS)"
    echo ""
    echo "--- Client ---"
    uname -a; lscpu | head -15; free -h
    echo ""
    echo "--- Mounts ---"
    mount | grep -E "rucksfs|nfs" || true
    echo ""
    echo "--- Server-1 (RucksFS, $SERVER1_IP) ---"
    ssh root@"$SERVER1_IP" 'lscpu | head -5; free -h; cat /data/env-info.txt 2>/dev/null' || true
    echo ""
    echo "--- Server-2 (NFS, $SERVER2_IP) ---"
    ssh root@"$SERVER2_IP" 'lscpu | head -5; free -h; cat /data/env-info.txt 2>/dev/null' || true
} > "$RESULT_DIR/environment.txt" 2>&1

ok "Environment saved"

# CPU governor
for ip in localhost "$SERVER1_IP" "$SERVER2_IP"; do
    if [[ "$ip" == "localhost" ]]; then
        cpupower frequency-set -g performance 2>/dev/null || true
    else
        ssh root@"$ip" 'cpupower frequency-set -g performance' 2>/dev/null || true
    fi
done

# ==========================================================================
# Experiment 1: Network Symmetry Verification
# ==========================================================================
log ""
log "================================================================"
log "  Experiment 1: Network Symmetry"
log "================================================================"

EXP1_DIR="$RESULT_DIR/exp1_network"
mkdir -p "$EXP1_DIR"

{
    echo "=== Ping: Client -> Server-1 (RucksFS) ==="
    ping -c 50 "$SERVER1_IP"
    echo ""
    echo "=== Ping: Client -> Server-2 (NFS) ==="
    ping -c 50 "$SERVER2_IP"
} > "$EXP1_DIR/ping.txt" 2>&1

{
    echo "=== iperf3: Client -> Server-1 ==="
    ssh root@"$SERVER1_IP" 'pkill iperf3 2>/dev/null; iperf3 -s -D -1' 2>/dev/null || true
    sleep 1
    iperf3 -c "$SERVER1_IP" -t 10 2>&1 || echo "failed"
    echo ""
    echo "=== iperf3: Client -> Server-2 ==="
    ssh root@"$SERVER2_IP" 'pkill iperf3 2>/dev/null; iperf3 -s -D -1' 2>/dev/null || true
    sleep 1
    iperf3 -c "$SERVER2_IP" -t 10 2>&1 || echo "failed"
} > "$EXP1_DIR/iperf3.txt" 2>&1

ok "Network verification complete"
grep "rtt" "$EXP1_DIR/ping.txt"

# ==========================================================================
# Experiment 1.5: NFS Thread Scan (find optimal nfsd thread count)
# ==========================================================================
log ""
log "================================================================"
log "  Experiment 1.5: NFS Thread Scan"
log "  Candidates: $THREAD_CANDIDATES  (np=$THREAD_SCAN_NP, $RUNS runs each)"
log "================================================================"

EXP15_DIR="$RESULT_DIR/exp1.5_thread_scan"
mkdir -p "$EXP15_DIR"

BEST_THREADS=16   # fallback
BEST_CREATE_OPS=0

for threads in $THREAD_CANDIDATES; do
    log "--- nfsd threads=$threads ---"
    set_nfsd_threads "$threads"
    sleep 2  # let kernel settle

    sum_create=0
    for run in $(seq 1 "$RUNS"); do
        outfile="$EXP15_DIR/nfsd_t${threads}_run${run}.txt"
        run_mdtest "nfs-t$threads" "$NFS_MOUNT" "$THREAD_SCAN_NP" "$outfile"
        create_ops=$(extract_create_ops "$outfile")
        log "    run $run: create=$create_ops ops/s"
        sum_create=$(echo "$sum_create + ${create_ops:-0}" | bc)
    done

    avg_create=$(echo "scale=1; $sum_create / $RUNS" | bc)
    log "  => threads=$threads  avg create=$avg_create ops/s"

    if (( $(echo "$avg_create > $BEST_CREATE_OPS" | bc -l) )); then
        BEST_CREATE_OPS="$avg_create"
        BEST_THREADS="$threads"
    fi
done

log ""
log "  *** Best nfsd thread count: $BEST_THREADS (avg create $BEST_CREATE_OPS ops/s) ***"
log ""

# Apply the optimal thread count for the main experiment
set_nfsd_threads "$BEST_THREADS"

# Save thread scan summary
{
    echo "=== NFS Thread Scan Summary ==="
    echo "Candidates: $THREAD_CANDIDATES"
    echo "Test: np=$THREAD_SCAN_NP, n=$N_FILES, $RUNS runs, $ITERATIONS mdtest iterations"
    echo ""
    for threads in $THREAD_CANDIDATES; do
        echo "--- threads=$threads ---"
        for run in $(seq 1 "$RUNS"); do
            outfile="$EXP15_DIR/nfsd_t${threads}_run${run}.txt"
            create=$(extract_create_ops "$outfile")
            stat=$(grep "File stat" "$outfile" 2>/dev/null | awk '{print $3}' | sort -n | tail -1)
            remove=$(grep "File removal" "$outfile" 2>/dev/null | awk '{print $3}' | sort -n | tail -1)
            echo "  run $run: create=$create stat=$stat remove=$remove"
        done
    done
    echo ""
    echo "BEST: threads=$BEST_THREADS (avg create=$BEST_CREATE_OPS ops/s)"
} > "$EXP15_DIR/summary.txt"

ok "Thread scan complete — using $BEST_THREADS nfsd threads"

# ==========================================================================
# Experiment 2: Concurrency Scaling — SERIAL EXECUTION
# NFS runs FIRST (all np), then RucksFS (all np).
# ==========================================================================
log ""
log "================================================================"
log "  Experiment 2: Concurrency Scaling (serial execution)"
log "  Phase A: NFS (all np) → Phase B: RucksFS (all np)"
log "================================================================"

EXP2_DIR="$RESULT_DIR/exp2_scaling"
mkdir -p "$EXP2_DIR"

# --- Phase A: NFS (Server-2 active, Server-1 idle) ---
log ""
log "──── Phase A: NFS tests (Server-2) ────"
for np in 1 2 4 8 16 32; do
    log "--- NFS np=$np ---"
    for run in $(seq 1 "$RUNS"); do
        run_mdtest "nfs" "$NFS_MOUNT" "$np" \
            "$EXP2_DIR/nfs_np${np}_run${run}.txt"
    done
done
ok "Phase A (NFS) complete"

# --- Phase B: RucksFS (Server-1 active, Server-2 idle) ---
log ""
log "──── Phase B: RucksFS tests (Server-1) ────"
for np in 1 2 4 8 16 32; do
    log "--- RucksFS np=$np ---"
    for run in $(seq 1 "$RUNS"); do
        run_mdtest "rucksfs" "$RUCKSFS_MOUNT" "$np" \
            "$EXP2_DIR/rucksfs_np${np}_run${run}.txt"
    done
done
ok "Phase B (RucksFS) complete"

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

# Quick summary
log "Quick Summary — NFS Thread Scan (Experiment 1.5):"
log "---------------------------------------------------"
log "Optimal nfsd threads: $BEST_THREADS (avg create $BEST_CREATE_OPS ops/s)"
log ""
log "Quick Summary — Experiment 2 (create ops/s, max of 3 mdtest iterations):"
log "------------------------------------------------------------------------"
for np in 1 2 4 8 16 32; do
    nfs_vals="" rfs_vals=""
    for run in $(seq 1 "$RUNS"); do
        v=$(grep "File creation" "$EXP2_DIR/nfs_np${np}_run${run}.txt" 2>/dev/null | awk '{print $3}' | tail -1)
        nfs_vals="$nfs_vals $v"
        v=$(grep "File creation" "$EXP2_DIR/rucksfs_np${np}_run${run}.txt" 2>/dev/null | awk '{print $3}' | tail -1)
        rfs_vals="$rfs_vals $v"
    done
    printf "np=%2d  NFS: %-30s  RucksFS: %s\n" "$np" "$nfs_vals" "$rfs_vals"
done

log ""
log "Quick Summary — stat ops/s:"
log "----------------------------"
for np in 1 2 4 8 16 32; do
    nfs_vals="" rfs_vals=""
    for run in $(seq 1 "$RUNS"); do
        v=$(grep "File stat" "$EXP2_DIR/nfs_np${np}_run${run}.txt" 2>/dev/null | awk '{print $3}' | tail -1)
        nfs_vals="$nfs_vals $v"
        v=$(grep "File stat" "$EXP2_DIR/rucksfs_np${np}_run${run}.txt" 2>/dev/null | awk '{print $3}' | tail -1)
        rfs_vals="$rfs_vals $v"
    done
    printf "np=%2d  NFS: %-30s  RucksFS: %s\n" "$np" "$nfs_vals" "$rfs_vals"
done

log ""
log "Quick Summary — remove ops/s:"
log "-------------------------------"
for np in 1 2 4 8 16 32; do
    nfs_vals="" rfs_vals=""
    for run in $(seq 1 "$RUNS"); do
        v=$(grep "File removal" "$EXP2_DIR/nfs_np${np}_run${run}.txt" 2>/dev/null | awk '{print $3}' | tail -1)
        nfs_vals="$nfs_vals $v"
        v=$(grep "File removal" "$EXP2_DIR/rucksfs_np${np}_run${run}.txt" 2>/dev/null | awk '{print $3}' | tail -1)
        rfs_vals="$rfs_vals $v"
    done
    printf "np=%2d  NFS: %-30s  RucksFS: %s\n" "$np" "$nfs_vals" "$rfs_vals"
done
