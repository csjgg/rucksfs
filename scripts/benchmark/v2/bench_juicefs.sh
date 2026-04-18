#!/bin/bash
# =============================================================================
# Experiment: JuiceFS+Redis Concurrency Scaling
# Runs JuiceFS mdtest at np=1,2,4,8,16,32
# Comparable to bench_nfs.sh and bench_rucksfs.sh
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/bench_common.sh"

JUICEFS_MOUNT="${JUICEFS_MOUNT:-/mnt/juicefs}"
SERVER_JFS_IP="${SERVER_JFS_IP:-$SERVER1_IP}"  # Redis server IP

EXP_DIR="$RESULT_BASE/exp2_scaling"
mkdir -p "$EXP_DIR"

log "================================================================"
log "  Experiment: JuiceFS+Redis Concurrency Scaling"
log "  Redis server: $SERVER_JFS_IP"
log "  Mount: $JUICEFS_MOUNT"
log "  np: 1 2 4 8 16 32"
log "  $RUNS runs x $ITERATIONS iterations, n=$N_FILES"
log "================================================================"

# Verify mount
if ! mount | grep -q "$JUICEFS_MOUNT"; then
    err "JuiceFS not mounted at $JUICEFS_MOUNT"
    exit 1
fi

set_performance_governor

# Warmup
warmup_mount "juicefs" "$JUICEFS_MOUNT"

for np in 1 2 4 8 16 32; do
    log ""
    log "--- JuiceFS np=$np ---"
    for run in $(seq 1 "$RUNS"); do
        outfile="$EXP_DIR/juicefs_np${np}_run${run}.txt"
        run_mdtest "juicefs" "$JUICEFS_MOUNT" "$np" "$outfile"

        create=$(extract_create_ops "$outfile")
        stat=$(extract_stat_ops "$outfile")
        remove=$(extract_remove_ops "$outfile")
        log "    run $run: create=$create stat=$stat remove=$remove"
    done
done

# Summary
log ""
log "=== JuiceFS+Redis Results Summary ==="
printf "%-6s  %-15s  %-15s  %-15s\n" "np" "create" "stat" "remove"
printf "%-6s  %-15s  %-15s  %-15s\n" "---" "------" "----" "------"
for np in 1 2 4 8 16 32; do
    sum_c=0; sum_s=0; sum_r=0
    for run in $(seq 1 "$RUNS"); do
        f="$EXP_DIR/juicefs_np${np}_run${run}.txt"
        c=$(extract_create_ops "$f"); sum_c=$(echo "$sum_c + ${c:-0}" | bc)
        s=$(extract_stat_ops "$f");   sum_s=$(echo "$sum_s + ${s:-0}" | bc)
        r=$(extract_remove_ops "$f"); sum_r=$(echo "$sum_r + ${r:-0}" | bc)
    done
    avg_c=$(echo "scale=1; $sum_c / $RUNS" | bc)
    avg_s=$(echo "scale=1; $sum_s / $RUNS" | bc)
    avg_r=$(echo "scale=1; $sum_r / $RUNS" | bc)
    printf "%-6s  %-15s  %-15s  %-15s\n" "$np" "$avg_c" "$avg_s" "$avg_r"
done

ok "JuiceFS+Redis benchmark complete. Results: $EXP_DIR/juicefs_*.txt"
