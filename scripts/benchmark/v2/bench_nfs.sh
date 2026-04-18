#!/bin/bash
# =============================================================================
# Experiment 2A: NFS Concurrency Scaling
# Runs NFS mdtest at np=1,2,4,8,16,32 with optimal nfsd thread count
# Server-1 (RucksFS) is idle during this phase
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/bench_common.sh"

EXP_DIR="$RESULT_BASE/exp2_scaling"
mkdir -p "$EXP_DIR"

# Read optimal thread count from thread scan
OPTIMAL_THREADS_FILE="$RESULT_BASE/optimal_nfsd_threads.txt"
if [[ -f "$OPTIMAL_THREADS_FILE" ]]; then
    OPTIMAL_THREADS=$(cat "$OPTIMAL_THREADS_FILE")
    log "Using optimal nfsd threads from scan: $OPTIMAL_THREADS"
else
    OPTIMAL_THREADS=16
    warn "No thread scan result found, using default: $OPTIMAL_THREADS"
fi

log "================================================================"
log "  Experiment 2A: NFS Concurrency Scaling"
log "  nfsd threads: $OPTIMAL_THREADS (from thread scan)"
log "  np: 1 2 4 8 16 32"
log "  $RUNS runs × $ITERATIONS iterations, n=$N_FILES"
log "================================================================"

set_performance_governor

# Apply optimal thread count
set_nfsd_threads "$OPTIMAL_THREADS"
sleep 2

# Warmup
warmup_mount "nfs" "$NFS_MOUNT"

for np in 1 2 4 8 16 32; do
    log ""
    log "--- NFS np=$np ---"
    for run in $(seq 1 "$RUNS"); do
        outfile="$EXP_DIR/nfs_np${np}_run${run}.txt"
        run_mdtest "nfs" "$NFS_MOUNT" "$np" "$outfile"

        create=$(extract_create_ops "$outfile")
        stat=$(extract_stat_ops "$outfile")
        remove=$(extract_remove_ops "$outfile")
        log "    run $run: create=$create stat=$stat remove=$remove"
    done
done

# Summary
log ""
log "=== NFS Results Summary (nfsd=$OPTIMAL_THREADS threads) ==="
printf "%-6s  %-15s  %-15s  %-15s\n" "np" "create" "stat" "remove"
printf "%-6s  %-15s  %-15s  %-15s\n" "---" "------" "----" "------"
for np in 1 2 4 8 16 32; do
    # Average across runs
    sum_c=0; sum_s=0; sum_r=0
    for run in $(seq 1 "$RUNS"); do
        f="$EXP_DIR/nfs_np${np}_run${run}.txt"
        c=$(extract_create_ops "$f"); sum_c=$(echo "$sum_c + ${c:-0}" | bc)
        s=$(extract_stat_ops "$f");   sum_s=$(echo "$sum_s + ${s:-0}" | bc)
        r=$(extract_remove_ops "$f"); sum_r=$(echo "$sum_r + ${r:-0}" | bc)
    done
    avg_c=$(echo "scale=1; $sum_c / $RUNS" | bc)
    avg_s=$(echo "scale=1; $sum_s / $RUNS" | bc)
    avg_r=$(echo "scale=1; $sum_r / $RUNS" | bc)
    printf "%-6s  %-15s  %-15s  %-15s\n" "$np" "$avg_c" "$avg_s" "$avg_r"
done

ok "Experiment 2A (NFS) complete. Results: $EXP_DIR/nfs_*.txt"
