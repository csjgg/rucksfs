#!/usr/bin/env bash
set -euo pipefail

# ── defaults ──────────────────────────────────────────────────────────
# bench-v3: 使用独立的 Terraform 目录 tencent-bench-v3/ 提供的 SSH 凭证
SSH_KEY="${SSH_KEY:-/data/workspace/rucksfs/infra/tencent-bench-v3/shunjiecuitest.pem}"
SSH_OPTS="-i $SSH_KEY -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=3"
SERVER_PUB="${SERVER_PUB:-}"
SERVER_PRIV="${SERVER_PRIV:-}"
CLIENT_PUBS="${CLIENT_PUBS:-}"
CLIENT_PRIVS="${CLIENT_PRIVS:-}"
# bench-v3: SUT 列表按并发档位动态调整——N≤32 跑四家，N≥64 只跑两个 RucksFS 变体
# 默认列表用于中低并发档位。高并发档位请用 --suts rucksfs-delta,rucksfs-nodelta 覆盖。
SUTS="rucksfs-delta,rucksfs-nodelta,nfs,juicefs-tikv"
MODES="hard,easy"
RESULTS_DIR="./results-v3"
SSH_USER="ubuntu"
# bench-v3 Option B: 每个 client 起多少个 mdtest rank
# 2C client 上 RANKS_PER_CLIENT=4 意味着 2× oversubscribe（FUSE/mdtest 大部分时间等 RPC，可承受）
# 默认 1 保持向后兼容；高并发 Delta 对比建议 2 或 4
RANKS_PER_CLIENT="${RANKS_PER_CLIENT:-1}"
# bench-v3 NFS 挂载选项：默认 noac（关闭所有客户端 AC，和 RucksFS TTL=0 / JuiceFS cache=0 对齐）
# 对照实验可改为 "vers=4.2" 即启用 AC，用于展示"NFS 开缓存后 stat 会飞，但 create/remove 仍塌"
NFS_MOUNT_OPTS="${NFS_MOUNT_OPTS:-vers=4.2,noac}"

# ── parse args ────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --server-pub)   SERVER_PUB="$2";   shift 2;;
        --server-priv)  SERVER_PRIV="$2";  shift 2;;
        --client-pubs)  CLIENT_PUBS="$2";  shift 2;;
        --client-privs) CLIENT_PRIVS="$2"; shift 2;;
        --suts)         SUTS="$2";         shift 2;;
        --modes)        MODES="$2";        shift 2;;
        --results-dir)  RESULTS_DIR="$2";  shift 2;;
        --ssh-user)     SSH_USER="$2";     shift 2;;
        --ranks-per-client) RANKS_PER_CLIENT="$2"; shift 2;;
        --nfs-mount-opts)   NFS_MOUNT_OPTS="$2";   shift 2;;
        *) echo "Unknown option: $1"; exit 1;;
    esac
done

IFS=',' read -ra SUT_LIST   <<< "$SUTS"
IFS=',' read -ra MODE_LIST  <<< "$MODES"
IFS=',' read -ra PUB_LIST   <<< "$CLIENT_PUBS"
IFS=',' read -ra PRIV_LIST  <<< "$CLIENT_PRIVS"

NUM_CLIENTS=${#PUB_LIST[@]}
if [[ $NUM_CLIENTS -eq 0 ]]; then
    echo "FATAL: --client-pubs is required"; exit 1
fi
if [[ ${#PRIV_LIST[@]} -ne $NUM_CLIENTS ]]; then
    echo "FATAL: client-pubs count ($NUM_CLIENTS) != client-privs count (${#PRIV_LIST[@]})"; exit 1
fi

mkdir -p "$RESULTS_DIR"

# ── helpers ───────────────────────────────────────────────────────────
log()  { echo "[$(date '+%H:%M:%S')] $*"; }
die()  { log "FATAL: $*"; exit 1; }

ssh_server() { ssh $SSH_OPTS "${SSH_USER}@${SERVER_PUB}" "$@"; }
ssh_client() { local idx=$1; shift; ssh $SSH_OPTS "${SSH_USER}@${PUB_LIST[$idx]}" "$@"; }

# run on server, detached (for daemons)
ssh_server_bg() { ssh -f $SSH_OPTS "${SSH_USER}@${SERVER_PUB}" "$@"; }

# run on all clients in parallel, wait for all (with per-client timeout)
on_all_clients() {
    local cmd="$1"
    local pids=()
    for i in $(seq 0 $((NUM_CLIENTS - 1))); do
        # Wrap each ssh in timeout so a hung client can't block the whole sweep.
        # 90s is enough for teardown/drop_caches; real mdtest runs have their own path.
        ( timeout 90 ssh $SSH_OPTS "${SSH_USER}@${PUB_LIST[$i]}" "$cmd" ) &
        pids+=($!)
    done
    local failed=0
    for pid in "${pids[@]}"; do
        wait "$pid" || failed=$((failed+1))
    done
    return $failed
}

files_per_rank() {
    local n=$1
    # bench-v3: 统一 2000/rank，使每个 mdtest 阶段执行时间足够长以降低噪声。
    # 相比 v2 的递减方案（N 增大时递减以限制总时长），v3 优先保证测量稳定。
    echo 2000
}

# wait for tcp port on server (called via ssh)
wait_port() {
    local port=$1 max=${2:-30}
    for ((i=0; i<max; i++)); do
        if ss -tln | grep -q ":${port} "; then return 0; fi
        sleep 1
    done
    return 1
}

wait_server_ports() {
    local ports="$1"  # space-separated
    log "  waiting for server ports: $ports"
    ssh_server "
        for port in $ports; do
            for i in \$(seq 1 30); do
                ss -tln | grep -q \":\${port} \" && break
                sleep 1
            done
            ss -tln | grep -q \":\${port} \" || { echo \"port \$port not up\"; exit 1; }
        done
    " || die "server ports not ready"
}

# ── teardown ──────────────────────────────────────────────────────────
teardown_all() {
    log "teardown_all: cleaning server + $NUM_CLIENTS clients"

    # server cleanup
    ssh_server "
        sudo systemctl stop nfs-kernel-server 2>/dev/null || true
        sudo fuser -k 8001/tcp 8002/tcp 2379/tcp 2380/tcp 20160/tcp 2>/dev/null || true
        pkill -9 rucksfs-metaserver 2>/dev/null || true
        pkill -9 rucksfs-metaserver-nodelta 2>/dev/null || true
        pkill -9 rucksfs-dataserver 2>/dev/null || true
        pkill -9 tikv-server 2>/dev/null || true
        pkill -9 pd-server 2>/dev/null || true
        pkill -9 juicefs 2>/dev/null || true
        sleep 3
    " || true

    # client cleanup (parallel)
    on_all_clients "
        pkill -9 rucksfs-remote-client 2>/dev/null || true
        pkill -9 juicefs 2>/dev/null || true
        sudo umount -l /mnt/sut 2>/dev/null || true
        for conn in /sys/fs/fuse/connections/*/abort; do
            [ -e \"\$conn\" ] && sudo bash -c \"echo 1 > \$conn\"
        done
        sudo fusermount3 -z -u /mnt/sut 2>/dev/null || true
        sudo rm -rf /mnt/sut
        sudo mkdir -p /mnt/sut
        sudo chown ${SSH_USER}:${SSH_USER} /mnt/sut
    " || true

    sleep 2
    log "teardown_all: done"
}

# ── SUT start functions ───────────────────────────────────────────────
start_rucksfs_delta() {
    log "  starting rucksfs-delta on server"
    ssh_server "
        sudo rm -rf /data/mds-bench /data/ds-bench
        sudo mkdir -p /data/mds-bench /data/ds-bench
        sudo chown ${SSH_USER}:${SSH_USER} /data/mds-bench /data/ds-bench
    "
    ssh_server_bg "nohup /tmp/rucksfs-metaserver --listen 0.0.0.0:8001 --data-dir /data/mds-bench > /tmp/mds.log 2>&1 &"
    ssh_server_bg "nohup /tmp/rucksfs-dataserver --listen 0.0.0.0:8002 --data-dir /data/ds-bench > /tmp/ds.log 2>&1 &"
    wait_server_ports "8001 8002"
}

start_rucksfs_nodelta() {
    log "  starting rucksfs-nodelta on server"
    ssh_server "
        sudo rm -rf /data/mds-bench /data/ds-bench
        sudo mkdir -p /data/mds-bench /data/ds-bench
        sudo chown ${SSH_USER}:${SSH_USER} /data/mds-bench /data/ds-bench
    "
    ssh_server_bg "nohup /tmp/rucksfs-metaserver-nodelta --listen 0.0.0.0:8001 --data-dir /data/mds-bench > /tmp/mds.log 2>&1 &"
    ssh_server_bg "nohup /tmp/rucksfs-dataserver --listen 0.0.0.0:8002 --data-dir /data/ds-bench > /tmp/ds.log 2>&1 &"
    wait_server_ports "8001 8002"
}

start_nfs() {
    log "  starting nfs on server"
    ssh_server "
        sudo rm -rf /data/nfs-export/*
        sudo systemctl start nfs-kernel-server
    " || die "nfs start failed"
}

start_juicefs_tikv() {
    log "  starting juicefs-tikv on server"
    ssh_server "
        sudo rm -rf /data/tikv-data /data/pd-data /data/jfs-data
        sudo mkdir -p /data/tikv-data /data/pd-data /data/jfs-data
        sudo chown ${SSH_USER}:${SSH_USER} /data/tikv-data /data/pd-data /data/jfs-data
    "
    ssh_server_bg "nohup \$HOME/.tiup/bin/tiup pd --data-dir=/data/pd-data --client-urls=http://0.0.0.0:2379 --advertise-client-urls=http://${SERVER_PRIV}:2379 --peer-urls=http://0.0.0.0:2380 --advertise-peer-urls=http://${SERVER_PRIV}:2380 > /tmp/pd.log 2>&1 &"
    sleep 5
    ssh_server_bg "nohup \$HOME/.tiup/bin/tiup tikv --pd-endpoints=http://127.0.0.1:2379 --data-dir=/data/tikv-data --addr=${SERVER_PRIV}:20160 --advertise-addr=${SERVER_PRIV}:20160 > /tmp/tikv.log 2>&1 &"
    sleep 10
    wait_server_ports "2379 20160"
    ssh_server "juicefs format 'tikv://${SERVER_PRIV}:2379/bench' bench --storage file --bucket /data/jfs-data" \
        || die "juicefs format failed"
}

start_sut() {
    local sut=$1
    case "$sut" in
        rucksfs-delta)   start_rucksfs_delta;;
        rucksfs-nodelta) start_rucksfs_nodelta;;
        nfs)             start_nfs;;
        juicefs-tikv)    start_juicefs_tikv;;
        *) die "unknown SUT: $sut";;
    esac
}

# ── mount on all clients ──────────────────────────────────────────────
mount_client() {
    local sut=$1 idx=$2
    case "$sut" in
        rucksfs-delta|rucksfs-nodelta)
            ssh_client "$idx" "
                RUCKSFS_CLIENT_POOL_SIZE=4 nohup /tmp/rucksfs-remote-client \
                    --mount /mnt/sut \
                    --meta-addr http://${SERVER_PRIV}:8001 \
                    --data-addr http://${SERVER_PRIV}:8002 > /tmp/rucksfs-client.log 2>&1 &
                for i in \$(seq 1 60); do
                    mountpoint -q /mnt/sut && exit 0
                    sleep 1
                done
                echo 'mount timeout'; exit 1
            ";;
        nfs)
            ssh_client "$idx" "
                sudo mount -t nfs -o ${NFS_MOUNT_OPTS} ${SERVER_PRIV}:/data/nfs-export /mnt/sut
            ";;
        juicefs-tikv)
            ssh_client "$idx" "
                juicefs mount 'tikv://${SERVER_PRIV}:2379/bench' /mnt/sut -d \
                    --attr-cache=0 --entry-cache=0 --dir-entry-cache=0 --open-cache=0
                for i in \$(seq 1 30); do
                    mountpoint -q /mnt/sut && exit 0
                    sleep 1
                done
                echo 'mount timeout'; exit 1
            ";;
    esac
}

mount_on_all_clients() {
    local sut=$1
    log "  mounting $sut on $NUM_CLIENTS clients"
    local pids=()
    for i in $(seq 0 $((NUM_CLIENTS - 1))); do
        mount_client "$sut" "$i" &
        pids+=($!)
    done
    local failed=0
    for pid in "${pids[@]}"; do
        wait "$pid" || failed=$((failed+1))
    done
    # Retry up to 3 times for any clients that failed
    local max_retries=3
    for ((attempt=1; attempt<=max_retries && failed>0; attempt++)); do
        log "  $failed client(s) failed mount (attempt $attempt/$max_retries), retrying..."
        sleep 10
        local retry_pids=()
        for i in $(seq 0 $((NUM_CLIENTS - 1))); do
            ssh_client "$i" "mountpoint -q /mnt/sut" 2>/dev/null || {
                mount_client "$sut" "$i" &
                retry_pids+=($!)
            }
        done
        failed=0
        for pid in "${retry_pids[@]}"; do
            wait "$pid" || failed=$((failed+1))
        done
    done
    if (( failed > 0 )); then
        die "$failed client(s) still failed to mount after $max_retries retries"
    fi
}

verify_mount_all_clients() {
    log "  verifying mount on all clients"
    local failed=0
    local failed_indices=()
    for i in $(seq 0 $((NUM_CLIENTS - 1))); do
        ssh_client "$i" "mountpoint -q /mnt/sut" 2>/dev/null || {
            log "  WARN: client $i (${PUB_LIST[$i]}) not mounted, will retry"
            failed_indices+=($i)
            failed=$((failed+1))
        }
    done
    if (( failed > 0 )); then
        log "  $failed client(s) failed verification, attempting remediation..."
        sleep 5
        local sut_name
        sut_name=$(ssh_client 0 "ps aux | grep rucksfs-remote-client | grep -v grep" 2>/dev/null && echo "rucksfs" || echo "other")
        for idx in "${failed_indices[@]}"; do
            # The SUT is passed from the main loop context
            mount_client "${CURRENT_SUT}" "$idx" || true
        done
        sleep 10
        local still_failed=0
        for idx in "${failed_indices[@]}"; do
            ssh_client "$idx" "mountpoint -q /mnt/sut" 2>/dev/null || {
                log "  ERROR: client $idx still not mounted after remediation"
                still_failed=$((still_failed+1))
            }
        done
        (( still_failed > 0 )) && die "$still_failed client(s) mount verification failed after remediation"
    fi
    log "  all $NUM_CLIENTS clients mounted OK"
}

# ── caches & prep ─────────────────────────────────────────────────────
drop_caches_all() {
    log "  dropping caches on server + all clients"
    ssh_server "echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null" || true
    on_all_clients "echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null" || true
}

# ── generate hostfile ─────────────────────────────────────────────────
generate_hostfile() {
    local tmpfile
    tmpfile=$(mktemp /tmp/hostfile.XXXXXX)
    for priv in "${PRIV_LIST[@]}"; do
        echo "$priv slots=1 max-slots=1" >> "$tmpfile"
    done
    echo "$tmpfile"
}

# ── run mdtest ────────────────────────────────────────────────────────
run_mdtest() {
    local sut=$1 mode=$2
    local np=$(( NUM_CLIENTS * RANKS_PER_CLIENT ))
    local fpr
    fpr=$(files_per_rank "$np")

    log "  mdtest: sut=$sut mode=$mode np=$np (clients=$NUM_CLIENTS × ranks_per_client=$RANKS_PER_CLIENT) files_per_rank=$fpr"

    # create bench dir from client-0
    ssh_client 0 "mkdir -p /mnt/sut/bench" || die "cannot create /mnt/sut/bench"

    # build hostfile on the SERVER (mpirun runs from server in bench-v3:
    # client-0 is 2C2G and its SSH fanout to 96 clients was unreliable).
    local hf_content=""
    for priv in "${PRIV_LIST[@]}"; do
        hf_content+="$priv slots=${RANKS_PER_CLIENT} max-slots=${RANKS_PER_CLIENT}\n"
    done
    ssh_server "printf '${hf_content}' > /tmp/mdtest_hostfile"

    local mode_flag=""
    if [[ "$mode" == "easy" ]]; then
        mode_flag="-u"
    fi

    local outfile="${RESULTS_DIR}/${sut}_${mode}_np${np}.txt"
    log "  output -> $outfile"

    # Run mpirun from the server node. mdtest processes are spawned on clients
    # via SSH per the hostfile; the server itself is not in the hostfile, so
    # the only work done on the server is orted coordination.
    # ulimit -n raised: 96 concurrent SSH connections + MPI internal FDs
    # easily blow past the default 1024 and cause Permission-denied-like
    # failures during orted launch.
    ssh_server "
        ulimit -n 65536
        mpirun --allow-run-as-root \
            --mca plm_rsh_num_concurrent 8 \
            --mca plm_rsh_no_tree_spawn 1 \
            --hostfile /tmp/mdtest_hostfile \
            -np $np \
            mdtest -d /mnt/sut/bench -n $fpr -F -C -T -r $mode_flag -i 3
    " 2>&1 | tee "$outfile"

    local rc=${PIPESTATUS[0]}
    if [[ $rc -ne 0 ]]; then
        log "  WARNING: mdtest exited with code $rc"
    fi
}

# ── extract results ───────────────────────────────────────────────────
# mdtest -i 3 的 SUMMARY 段格式：
#   Operation                     Max            Min           Mean        Std Dev
#   File creation               68588.080      68588.080      68588.080          0.000
# 字段序：第 3=Max、第 4=Min、第 5=Mean、第 6=Std Dev。
# bench-v3 以 Mean 为主指标；同时记录 Std Dev 以便发现异常迭代。
extract_mean() {
    local file=$1 metric=$2
    grep "$metric" "$file" | tail -1 | awk '{print $5}'
}

extract_stddev() {
    local file=$1 metric=$2
    grep "$metric" "$file" | tail -1 | awk '{print $6}'
}

write_summary_csv() {
    local csv="${RESULTS_DIR}/summary.csv"
    echo "sut,mode,np,file_creation,file_creation_std,file_stat,file_stat_std,file_removal,file_removal_std" > "$csv"

    for f in "${RESULTS_DIR}"/*.txt; do
        [[ -f "$f" ]] || continue
        local base
        base=$(basename "$f" .txt)
        local sut mode np
        np=$(echo "$base" | grep -oP 'np\K[0-9]+')
        mode=$(echo "$base" | grep -oP '_\K(hard|easy)(?=_np)')
        sut=$(echo "$base" | sed "s/_${mode}_np${np}//")

        local c_mean c_std s_mean s_std r_mean r_std
        c_mean=$(extract_mean   "$f" "File creation")
        c_std=$(extract_stddev  "$f" "File creation")
        s_mean=$(extract_mean   "$f" "File stat")
        s_std=$(extract_stddev  "$f" "File stat")
        r_mean=$(extract_mean   "$f" "File removal")
        r_std=$(extract_stddev  "$f" "File removal")

        echo "${sut},${mode},${np},${c_mean},${c_std},${s_mean},${s_std},${r_mean},${r_std}" >> "$csv"
    done

    log "summary written to $csv"
    cat "$csv"
}

# ── main loop ─────────────────────────────────────────────────────────
main() {
    log "========================================"
    log "Benchmark Orchestrator v3 (MDS-path, client caches disabled)"
    log "Server:  $SERVER_PUB ($SERVER_PRIV)"
    log "Clients: $NUM_CLIENTS (ranks_per_client=$RANKS_PER_CLIENT, total_np=$(( NUM_CLIENTS * RANKS_PER_CLIENT )))"
    log "SUTs:    ${SUT_LIST[*]}"
    log "Modes:   ${MODE_LIST[*]}"
    log "Results: $RESULTS_DIR"
    log "NFS opts: $NFS_MOUNT_OPTS"
    log "========================================"

    for sut in "${SUT_LIST[@]}"; do
        log "===== SUT: $sut ====="
        CURRENT_SUT="$sut"

        teardown_all
        start_sut "$sut"
        mount_on_all_clients "$sut"
        verify_mount_all_clients

        for mode in "${MODE_LIST[@]}"; do
            log "--- $sut / $mode ---"
            drop_caches_all
            run_mdtest "$sut" "$mode"
        done

        teardown_all
    done

    write_summary_csv
    log "ALL DONE"
}

main
