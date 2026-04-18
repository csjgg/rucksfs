#!/bin/bash
# =============================================================================
# Experiment 1.5: NFS Thread Scan
# Find optimal nfsd thread count on dedicated Server-2
# Tests: 8, 16, 32, 64 threads at np=16 with warmup + 3 cold-start runs
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/bench_common.sh"

THREAD_CANDIDATES="${THREAD_CANDIDATES:-8 16 32 64}"
SCAN_NP="${SCAN_NP:-16}"

EXP_DIR="$RESULT_BASE/exp1.5_thread_scan"
mkdir -p "$EXP_DIR"

log "================================================================"
log "  Experiment 1.5: NFS Thread Scan"
log "  Candidates: $THREAD_CANDIDATES"
log "  np=$SCAN_NP, $RUNS cold-start runs, $ITERATIONS mdtest iterations"
log "================================================================"

set_performance_governor

BEST_THREADS=16   # fallback
BEST_CREATE_OPS=0

for threads in $THREAD_CANDIDATES; do
    log ""
    log "--- nfsd threads=$threads ---"
    set_nfsd_threads "$threads"
    sleep 3  # let kernel settle after thread change

    # Warmup: prime the NFS mount path with a throwaway run
    warmup_mount "nfs-warmup-t$threads" "$NFS_MOUNT"

    sum_create=0
    sum_stat=0
    sum_remove=0

    for run in $(seq 1 "$RUNS"); do
        outfile="$EXP_DIR/nfsd_t${threads}_run${run}.txt"
        run_mdtest "nfs-t$threads" "$NFS_MOUNT" "$SCAN_NP" "$outfile"

        create_ops=$(extract_create_ops "$outfile")
        stat_ops=$(extract_stat_ops "$outfile")
        remove_ops=$(extract_remove_ops "$outfile")

        log "    run $run: create=${create_ops:-0} stat=${stat_ops:-0} remove=${remove_ops:-0} ops/s"
        sum_create=$(echo "$sum_create + ${create_ops:-0}" | bc)
        sum_stat=$(echo "$sum_stat + ${stat_ops:-0}" | bc)
        sum_remove=$(echo "$sum_remove + ${remove_ops:-0}" | bc)
    done

    avg_create=$(echo "scale=1; $sum_create / $RUNS" | bc)
    avg_stat=$(echo "scale=1; $sum_stat / $RUNS" | bc)
    avg_remove=$(echo "scale=1; $sum_remove / $RUNS" | bc)
    log "  => threads=$threads  avg: create=$avg_create stat=$avg_stat remove=$avg_remove ops/s"

    # Select best based on create ops (primary metric)
    if (( $(echo "$avg_create > $BEST_CREATE_OPS" | bc -l) )); then
        BEST_CREATE_OPS="$avg_create"
        BEST_THREADS="$threads"
    fi
done

log ""
log "  *** Best nfsd thread count: $BEST_THREADS (avg create $BEST_CREATE_OPS ops/s) ***"
log ""

# Save summary
{
    echo "=== NFS Thread Scan Summary ==="
    echo "Candidates: $THREAD_CANDIDATES"
    echo "Test: np=$SCAN_NP, n=$N_FILES, $RUNS cold-start runs, $ITERATIONS mdtest iterations"
    echo "Warmup: 100 files single-process before each thread count"
    echo ""
    for threads in $THREAD_CANDIDATES; do
        echo "--- threads=$threads ---"
        for run in $(seq 1 "$RUNS"); do
            outfile="$EXP_DIR/nfsd_t${threads}_run${run}.txt"
            create=$(extract_create_ops "$outfile")
            stat=$(extract_stat_ops "$outfile")
            remove=$(extract_remove_ops "$outfile")
            echo "  run $run: create=$create stat=$stat remove=$remove"
        done
    done
    echo ""
    echo "BEST: threads=$BEST_THREADS (avg create=$BEST_CREATE_OPS ops/s)"
} > "$EXP_DIR/summary.txt"

# Write optimal value for downstream scripts
echo "$BEST_THREADS" > "$RESULT_BASE/optimal_nfsd_threads.txt"
log "Optimal thread count written to $RESULT_BASE/optimal_nfsd_threads.txt"

ok "Thread scan complete — optimal: $BEST_THREADS threads"
