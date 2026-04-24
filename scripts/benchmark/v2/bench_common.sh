#!/bin/bash
# =============================================================================
# Common functions for benchmark v2 scripts
# Source this file: source bench_common.sh
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration (override before sourcing if needed)
# ---------------------------------------------------------------------------
SERVER1_IP="${SERVER1_IP:-10.0.1.4}"   # RucksFS MDS+DS
SERVER2_IP="${SERVER2_IP:-10.0.1.8}"   # NFS
NFS_MOUNT="${NFS_MOUNT:-/mnt/nfs}"
RUCKSFS_MOUNT="${RUCKSFS_MOUNT:-/mnt/rucksfs-dist}"
N_FILES="${N_FILES:-5000}"
ITERATIONS="${ITERATIONS:-3}"
RUNS="${RUNS:-3}"
RESULT_BASE="${RESULT_BASE:-/data/test-results/controlled_v2}"

# Create result directory (shared across all scripts in a single run)
mkdir -p "$RESULT_BASE"

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------
CYAN='\033[0;36m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
log()  { echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} $*"; }
ok()   { echo -e "${GREEN}[OK]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
err()  { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# ---------------------------------------------------------------------------
# Cache management
# ---------------------------------------------------------------------------
drop_all_caches() {
    log "  Dropping caches on all nodes..."
    sync
    echo 3 > /proc/sys/vm/drop_caches 2>/dev/null || true
    ssh root@"$SERVER1_IP" 'sync && echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    ssh root@"$SERVER2_IP" 'sync && echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    sleep 2
}

# ---------------------------------------------------------------------------
# NFS thread management
# ---------------------------------------------------------------------------
set_nfsd_threads() {
    local n="$1"
    ssh root@"$SERVER2_IP" "rpc.nfsd $n"
    local actual
    actual=$(ssh root@"$SERVER2_IP" 'cat /proc/fs/nfsd/threads')
    log "  nfsd threads set to $actual"
}

# ---------------------------------------------------------------------------
# Warmup: run a throwaway mdtest to prime the mount/VFS path
# ---------------------------------------------------------------------------
warmup_mount() {
    local label="$1" mp="$2"
    local warmup_dir="$mp/warmup_$$"
    log "  Warmup: $label ($mp)"
    rm -rf "$warmup_dir" 2>/dev/null || true
    mkdir -p "$warmup_dir"
    # Small warmup: 100 files, single process, no output saved
    mdtest -d "$warmup_dir" -n 100 -F -C -T -r -u -i 1 > /dev/null 2>&1 || true
    rm -rf "$warmup_dir" 2>/dev/null || true
    log "  Warmup done"
}

# ---------------------------------------------------------------------------
# Cleanup test directory between runs (includes server-side cleanup for NFS)
# ---------------------------------------------------------------------------
cleanup_test_dir() {
    local mp="$1"
    local test_dir="$mp/mdtest_run"
    rm -rf "$test_dir" 2>/dev/null || true
    # Also clean any straggler files
    find "$mp" -mindepth 1 -maxdepth 1 -exec rm -rf {} + 2>/dev/null || true
    # If it's NFS mount, also clean server-side to avoid stale handles
    if mount | grep -q "nfs.*$mp"; then
        ssh root@"$SERVER2_IP" "find /data/nfs-export -mindepth 1 -delete 2>/dev/null" || true
    fi
    sleep 1
}

# ---------------------------------------------------------------------------
# Run mdtest
# ---------------------------------------------------------------------------
run_mdtest() {
    local label="$1" mp="$2" np="$3" outfile="$4"
    local test_dir="$mp/mdtest_run"

    # Pre-run cleanup
    cleanup_test_dir "$mp"
    mkdir -p "$test_dir"

    # Drop caches for cold-start measurement
    drop_all_caches

    log "  [$label] np=$np n=$N_FILES iter=$ITERATIONS"
    if [[ "$np" -eq 1 ]]; then
        mdtest -d "$test_dir" -n "$N_FILES" -F -C -T -r -u -i "$ITERATIONS" \
            2>&1 | tee "$outfile"
    else
        mpirun --allow-run-as-root --oversubscribe -np "$np" \
            mdtest -d "$test_dir" -n "$N_FILES" -F -C -T -r -u -i "$ITERATIONS" \
            2>&1 | tee "$outfile"
    fi

    # Post-run cleanup
    cleanup_test_dir "$mp"
}

# ---------------------------------------------------------------------------
# Extract ops/s from mdtest output (max across iterations)
# ---------------------------------------------------------------------------
extract_create_ops() {
    grep "File creation" "$1" 2>/dev/null | awk '{print $3}' | sort -n | tail -1
}

extract_stat_ops() {
    grep "File stat" "$1" 2>/dev/null | awk '{print $3}' | sort -n | tail -1
}

extract_remove_ops() {
    grep "File removal" "$1" 2>/dev/null | awk '{print $3}' | sort -n | tail -1
}

# ---------------------------------------------------------------------------
# CPU governor
# ---------------------------------------------------------------------------
set_performance_governor() {
    log "Setting CPU governor to performance on all nodes..."
    cpupower frequency-set -g performance 2>/dev/null || true
    ssh root@"$SERVER1_IP" 'cpupower frequency-set -g performance' 2>/dev/null || true
    ssh root@"$SERVER2_IP" 'cpupower frequency-set -g performance' 2>/dev/null || true
}

log "bench_common.sh loaded (Server1=$SERVER1_IP Server2=$SERVER2_IP)"
