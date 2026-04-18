#!/bin/bash
# =============================================================================
# Experiment 2B: RucksFS Concurrency Scaling
# Runs RucksFS mdtest at np=1,2,4,8,16,32
# Server-2 (NFS) is idle during this phase
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/bench_common.sh"

EXP_DIR="$RESULT_BASE/exp2_scaling"
mkdir -p "$EXP_DIR"

log "================================================================"
log "  Experiment 2B: RucksFS Concurrency Scaling"
log "  np: 1 2 4 8 16 32"
log "  $RUNS runs × $ITERATIONS iterations, n=$N_FILES"
log "================================================================"

set_performance_governor

# Warmup
warmup_mount "rucksfs" "$RUCKSFS_MOUNT"

for np in 1 2 4 8 16 32; do
    log ""
    log "--- RucksFS np=$np ---"
    for run in $(seq 1 "$RUNS"); do
        outfile="$EXP_DIR/rucksfs_np${np}_run${run}.txt"
        run_mdtest "rucksfs" "$RUCKSFS_MOUNT" "$np" "$outfile"

        create=$(extract_create_ops "$outfile")
        stat=$(extract_stat_ops "$outfile")
        remove=$(extract_remove_ops "$outfile")
        log "    run $run: create=$create stat=$stat remove=$remove"
    done
done

# Summary
log ""
log "=== RucksFS Results Summary ==="
printf "%-6s  %-15s  %-15s  %-15s\n" "np" "create" "stat" "remove"
printf "%-6s  %-15s  %-15s  %-15s\n" "---" "------" "----" "------"
for np in 1 2 4 8 16 32; do
    sum_c=0; sum_s=0; sum_r=0
    for run in $(seq 1 "$RUNS"); do
        f="$EXP_DIR/rucksfs_np${np}_run${run}.txt"
        c=$(extract_create_ops "$f"); sum_c=$(echo "$sum_c + ${c:-0}" | bc)
        s=$(extract_stat_ops "$f");   sum_s=$(echo "$sum_s + ${s:-0}" | bc)
        r=$(extract_remove_ops "$f"); sum_r=$(echo "$sum_r + ${r:-0}" | bc)
    done
    avg_c=$(echo "scale=1; $sum_c / $RUNS" | bc)
    avg_s=$(echo "scale=1; $sum_s / $RUNS" | bc)
    avg_r=$(echo "scale=1; $sum_r / $RUNS" | bc)
    printf "%-6s  %-15s  %-15s  %-15s\n" "$np" "$avg_c" "$avg_s" "$avg_r"
done

ok "Experiment 2B (RucksFS) complete. Results: $EXP_DIR/rucksfs_*.txt"
