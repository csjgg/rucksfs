#!/usr/bin/env bash
# run-mdtest.sh — runs ON CLIENT-0.
# Mounts the SUT, then runs mpirun mdtest across all clients.
# Usage: run-mdtest.sh <sut> <server_ip> <hostfile_path> <results_dir>
set -uo pipefail

SUT="${1:?sut name}"
SERVER_IP="${2:?server private ip}"
HOSTFILE="${3:?hostfile}"
RESULTS_DIR="${4:?results dir}"

MNT=/mnt/sut
mkdir -p "$RESULTS_DIR"
sudo mkdir -p "$MNT"

log() { printf "[run-mdtest %s] %s\n" "$(date +%H:%M:%S)" "$*"; }

# ------------------------------------------------------------
# drop caches on all clients + server
# ------------------------------------------------------------
drop_all_caches() {
    sync
    sudo bash -c 'echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
    # propagate to other clients via SSH (from hostfile)
    while read -r host _; do
        [ -z "$host" ] && continue
        ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 "ubuntu@$host" \
            "sync && sudo bash -c 'echo 3 > /proc/sys/vm/drop_caches'" 2>/dev/null || true
    done < "$HOSTFILE"
    # server too
    ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 "ubuntu@$SERVER_IP" \
        "sync && sudo bash -c 'echo 3 > /proc/sys/vm/drop_caches'" 2>/dev/null || true
    sleep 1
}

# ------------------------------------------------------------
# Mount SUT on all clients (parallel via SSH)
# ------------------------------------------------------------
mount_sut_on_all() {
    local cmd=""
    case "$SUT" in
        rucksfs-delta|rucksfs-nodelta)
            cmd="
                sudo mkdir -p $MNT
                fusermount3 -u $MNT 2>/dev/null || true
                sleep 1
                export RUCKSFS_CLIENT_POOL_SIZE=4
                nohup /usr/local/bin/rucksfs-remote-client \
                    --mount $MNT \
                    --meta-addr http://$SERVER_IP:8001 \
                    --data-addr http://$SERVER_IP:8002 \
                    > /tmp/rucksfs-client.log 2>&1 &
                sleep 3
                mountpoint -q $MNT || { echo 'FUSE mount failed'; tail /tmp/rucksfs-client.log; exit 1; }
            "
            ;;
        nfs|juicefs-redis|juicefs-tikv)
            # All non-rucksfs SUTs are accessed via NFS (the server re-exports juicefs over NFS)
            local export_path
            case "$SUT" in
                nfs)                export_path=/data/server/nfs-export ;;
                juicefs-redis|juicefs-tikv) export_path=/data/server/jfs-mnt ;;
            esac
            cmd="
                sudo mkdir -p $MNT
                sudo umount -l $MNT 2>/dev/null || true
                sleep 1
                sudo mount -t nfs -o vers=3,hard,timeo=600,retrans=2 $SERVER_IP:$export_path $MNT
                mountpoint -q $MNT || { echo 'NFS mount failed'; exit 1; }
            "
            ;;
    esac

    # Run on localhost (client-0)
    bash -c "$cmd"

    # Run on all other clients in parallel
    local pids=()
    while read -r host _; do
        [ -z "$host" ] && continue
        [ "$host" = "$(hostname -I | awk '{print $1}')" ] && continue
        ssh -o StrictHostKeyChecking=no "ubuntu@$host" "$cmd" &
        pids+=($!)
    done < "$HOSTFILE"
    for pid in "${pids[@]}"; do wait "$pid" || true; done
}

unmount_sut_on_all() {
    local cmd=""
    case "$SUT" in
        rucksfs-delta|rucksfs-nodelta)
            cmd="fusermount3 -u $MNT 2>/dev/null; sleep 1; true"
            ;;
        *)
            cmd="sudo umount -l $MNT 2>/dev/null; sleep 1; true"
            ;;
    esac
    bash -c "$cmd"
    while read -r host _; do
        [ -z "$host" ] && continue
        [ "$host" = "$(hostname -I | awk '{print $1}')" ] && continue
        ssh -o StrictHostKeyChecking=no "ubuntu@$host" "$cmd" 2>/dev/null &
    done < "$HOSTFILE"
    wait 2>/dev/null || true
}

# ------------------------------------------------------------
# Run mdtest at a specific N (total rank count)
# ------------------------------------------------------------
run_one() {
    local N="$1"; local files_per_rank="$2"; local run="$3"; local mode="$4"
    local mode_flag=""
    case "$mode" in
        hard) mode_flag="" ;;    # shared parent dir
        easy) mode_flag="-u" ;;  # unique per-rank subdir
    esac
    local out="$RESULTS_DIR/${SUT}_${mode}_np${N}_run${run}.txt"
    drop_all_caches
    # Clean test dir on SUT mount
    sudo rm -rf "$MNT/test_${mode}_${N}" 2>/dev/null || true
    sudo mkdir -p "$MNT/test_${mode}_${N}"
    sudo chmod 777 "$MNT/test_${mode}_${N}"
    log "N=$N mode=$mode run=$run files/rank=$files_per_rank"
    mpirun --allow-run-as-root --oversubscribe --hostfile "$HOSTFILE" -np "$N" \
        mdtest -d "$MNT/test_${mode}_${N}" -n "$files_per_rank" -F -C -T -r $mode_flag -i 1 \
        > "$out" 2>&1
    # Print the summary line for progress
    grep -E "File creation|File stat|File removal" "$out" | head -5 | sed 's/^/    /'
}

# ------------------------------------------------------------
# Main: scale over concurrency levels
# ------------------------------------------------------------
log "mounting $SUT on all clients"
mount_sut_on_all || { log "mount failed"; exit 1; }

# Verify mount on client-0
ls "$MNT" >/dev/null 2>&1 || { log "mount verification failed"; exit 1; }
log "mount ok"

# Hard mode matrix: N ∈ {8, 16, 32, 64, 96, 128, 192}
# Reduce files/rank for N≥96 to cap runtime at ~3 min per run
declare -A FILES_HARD=( [8]=2000 [16]=2000 [32]=2000 [64]=1500 [96]=1000 [128]=1000 [192]=800 )

for N in 8 16 32 64 96 128 192; do
    for run in 1 2 3; do
        run_one "$N" "${FILES_HARD[$N]}" "$run" "hard" || log "  run failed, continuing"
    done
done

log "mdtest hard matrix done, unmounting"
unmount_sut_on_all
log "run-mdtest.sh finished for SUT=$SUT"
