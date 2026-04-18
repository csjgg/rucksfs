#!/bin/bash
# =============================================================================
# Experiment 1: Network Symmetry Verification
# Verifies Client->Server1 and Client->Server2 have equal latency/bandwidth
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/bench_common.sh"

EXP_DIR="$RESULT_BASE/exp1_network"
mkdir -p "$EXP_DIR"

log "================================================================"
log "  Experiment 1: Network Symmetry Verification"
log "  Server-1 (RucksFS): $SERVER1_IP"
log "  Server-2 (NFS):     $SERVER2_IP"
log "================================================================"

# --- Ping ---
log "Running ping tests (50 packets each)..."
{
    echo "=== Ping: Client -> Server-1 (RucksFS, $SERVER1_IP) ==="
    ping -c 50 "$SERVER1_IP"
    echo ""
    echo "=== Ping: Client -> Server-2 (NFS, $SERVER2_IP) ==="
    ping -c 50 "$SERVER2_IP"
} > "$EXP_DIR/ping.txt" 2>&1

log "Ping results:"
grep "rtt" "$EXP_DIR/ping.txt"

# --- iperf3 ---
log "Running iperf3 bandwidth tests (10s each)..."
{
    echo "=== iperf3: Client -> Server-1 (RucksFS) ==="
    ssh root@"$SERVER1_IP" 'pkill iperf3 2>/dev/null; iperf3 -s -D -1' 2>/dev/null || true
    sleep 1
    iperf3 -c "$SERVER1_IP" -t 10 2>&1 || echo "iperf3 to server1 failed"
    echo ""
    echo "=== iperf3: Client -> Server-2 (NFS) ==="
    ssh root@"$SERVER2_IP" 'pkill iperf3 2>/dev/null; iperf3 -s -D -1' 2>/dev/null || true
    sleep 1
    iperf3 -c "$SERVER2_IP" -t 10 2>&1 || echo "iperf3 to server2 failed"
} > "$EXP_DIR/iperf3.txt" 2>&1

log "iperf3 results saved"

# --- Environment snapshot ---
log "Saving environment info..."
{
    echo "=== Benchmark Environment (v2 symmetric) ==="
    echo "Timestamp: $(date -Iseconds)"
    echo "Topology: Client + Server-1(RucksFS MDS+DS) + Server-2(NFS nfsd+ext4)"
    echo ""
    echo "--- Client ---"
    uname -a; lscpu | head -15; free -h
    echo ""
    echo "--- Mounts ---"
    mount | grep -E "rucksfs|nfs" || true
    echo ""
    echo "--- Server-1 (RucksFS, $SERVER1_IP) ---"
    ssh root@"$SERVER1_IP" 'uname -a; lscpu | head -8; free -h; df -h /data' 2>/dev/null || true
    echo ""
    echo "--- Server-2 (NFS, $SERVER2_IP) ---"
    ssh root@"$SERVER2_IP" 'uname -a; lscpu | head -8; free -h; df -h /data; cat /proc/fs/nfsd/threads' 2>/dev/null || true
} > "$RESULT_BASE/environment.txt" 2>&1

ok "Experiment 1 complete. Results: $EXP_DIR"
