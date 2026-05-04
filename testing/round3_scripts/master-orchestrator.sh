#!/usr/bin/env bash
# master-orchestrator.sh — runs LOCALLY. End-to-end Round 3 pipeline.
#
# Steps:
#   0. Prechecks (binaries exist)
#   1. terraform apply (6 clients + 1 server)
#   2. Wait for cloud-init on all nodes
#   3. Upload binaries + scripts
#   4. Setup SSH mesh (client-0 → client-N via MPI)
#   5. Generate hostfile (private IPs)
#   6. Phase 1: RucksFS delta  (  → fuse breakout check)
#   7. Phase 1: RucksFS nodelta
#   8. Parse Phase 1 results; check breakout criterion
#   9. Phase 2: NFS                    [skipped if Phase 1 breakout triggered]
#  10. Phase 3: JuiceFS+Redis
#  11. Phase 4: JuiceFS+TiKV
#  12. Collect all results
#  13. terraform destroy
#
# Env vars:
#   SKIP_TERRAFORM_APPLY=1    — assume instances already running (set CLIENT_IPS/SERVER_IP manually)
#   SKIP_TERRAFORM_DESTROY=1  — don't destroy at end (DEBUG ONLY; costs money!)
#   SKIP_SUTS="nfs juicefs-tikv"   — space-separated list of SUTs to skip in Phase 2-4
#   FORCE_CONTINUE=1          — ignore breakout criterion (run full matrix regardless)

set -uo pipefail

REPO="/data/workspace/rucksfs"
INFRA="$REPO/infra/tencent-bench"
SCRIPTS="$REPO/testing/round3_scripts"
SSH_KEY="$INFRA/shunjiecuitest.pem"
SSH_OPTS="-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ServerAliveInterval=30 -o ConnectTimeout=10 -i $SSH_KEY"

RUN_ID="round3_$(date +%Y%m%d_%H%M%S)"
RESULTS_LOCAL="$REPO/testing/results/$RUN_ID"
mkdir -p "$RESULTS_LOCAL"

log() {
    printf "\n\033[1;36m[%s]\033[0m %s\n" "$(date +%H:%M:%S)" "$*"
    printf "[%s] %s\n" "$(date +%H:%M:%S)" "$*" >> "$RESULTS_LOCAL/orchestrator.log"
}

# -------------------- STEP 0: precheck binaries --------------------
log "Step 0: precheck binaries"

# Build both variants of metaserver
if [ ! -x /tmp/rucksfs-metaserver-with-delta ] || [ "$REPO/server/src/lib.rs" -nt /tmp/rucksfs-metaserver-with-delta ]; then
    log "building rucksfs-metaserver-with-delta"
    (cd "$REPO" && touch server/src/lib.rs && cargo build --release --workspace 2>&1 | tail -5)
    cp "$REPO/target/release/rucksfs-metaserver" /tmp/rucksfs-metaserver-with-delta
fi
if [ ! -x /tmp/rucksfs-metaserver-no-delta ] || [ "$REPO/server/src/lib.rs" -nt /tmp/rucksfs-metaserver-no-delta ]; then
    log "building rucksfs-metaserver-no-delta"
    (cd "$REPO" && touch server/src/lib.rs && cargo build --release --workspace --features rucksfs-server/no_delta 2>&1 | tail -5)
    cp "$REPO/target/release/rucksfs-metaserver" /tmp/rucksfs-metaserver-no-delta
fi

# Rebuild with-delta as the final state (so dataserver/client match delta code)
(cd "$REPO" && touch server/src/lib.rs && cargo build --release --workspace 2>&1 | tail -3)

BINS_SERVER=(
    "/tmp/rucksfs-metaserver-with-delta"
    "/tmp/rucksfs-metaserver-no-delta"
    "$REPO/target/release/rucksfs-dataserver"
)
BINS_CLIENT=(
    "$REPO/target/release/rucksfs-remote-client"
    "$REPO/benchmark/bench-tool/target/release/rucksfs-bench"
)
for b in "${BINS_SERVER[@]}" "${BINS_CLIENT[@]}"; do
    [ -x "$b" ] || { log "MISSING: $b"; exit 2; }
done

# -------------------- STEP 1: terraform apply --------------------
if [ -z "${SKIP_TERRAFORM_APPLY:-}" ]; then
    log "Step 1: terraform apply (6 clients + 1 server)"
    cd "$INFRA"
    terraform init -upgrade 2>&1 | tail -5
    terraform apply -auto-approve 2>&1 | tail -10
    CLIENT_IPS=$(terraform output -json client_public_ips | python3 -c 'import json,sys; print(",".join(json.load(sys.stdin)))')
    CLIENT_PRIV_IPS=$(terraform output -json client_private_ips | python3 -c 'import json,sys; print(",".join(json.load(sys.stdin)))')
    SERVER_IP=$(terraform output -raw server_rucksfs_public_ip)
    SERVER_INTERNAL_IP=$(terraform output -raw server_rucksfs_private_ip)
    cd - >/dev/null
fi
: "${CLIENT_IPS:?need CLIENT_IPS}" "${SERVER_IP:?need SERVER_IP}"
log "Server:  $SERVER_IP (internal $SERVER_INTERNAL_IP)"
log "Clients: $CLIENT_IPS"
log "         (internal: $CLIENT_PRIV_IPS)"
IFS=',' read -r -a C_PUB_ARR <<< "$CLIENT_IPS"
IFS=',' read -r -a C_PRIV_ARR <<< "$CLIENT_PRIV_IPS"
NUM_CLIENTS=${#C_PUB_ARR[@]}

# -------------------- STEP 2: wait for cloud-init --------------------
log "Step 2: wait for cloud-init on all $((NUM_CLIENTS + 1)) nodes (up to 10 min)"
for target_spec in "$SERVER_IP:server" "${C_PUB_ARR[@]/%/:client}"; do
    ip="${target_spec%:*}"; name="${target_spec##*:}"
    for i in $(seq 1 120); do
        if ssh $SSH_OPTS "ubuntu@$ip" "grep -q 'init complete' /var/log/bench-init.log" 2>/dev/null; then
            log "  $ip ($name) ready"
            break
        fi
        sleep 5
    done
done

# -------------------- STEP 3: upload binaries & scripts --------------------
log "Step 3: upload binaries to server"
scp $SSH_OPTS "${BINS_SERVER[@]}" "ubuntu@$SERVER_IP:/tmp/" >/dev/null
scp $SSH_OPTS "$SCRIPTS/switch-sut.sh" "ubuntu@$SERVER_IP:/tmp/" >/dev/null
ssh $SSH_OPTS "ubuntu@$SERVER_IP" '
    sudo cp /tmp/rucksfs-metaserver-with-delta /tmp/rucksfs-metaserver-no-delta /usr/local/bin/
    sudo cp /tmp/rucksfs-dataserver /usr/local/bin/
    sudo chmod +x /usr/local/bin/rucksfs-metaserver-with-delta /usr/local/bin/rucksfs-metaserver-no-delta /usr/local/bin/rucksfs-dataserver
    sudo cp /tmp/switch-sut.sh /usr/local/bin/
    sudo chmod +x /usr/local/bin/switch-sut.sh
    sudo mkdir -p /data/server
    sudo chown -R ubuntu:ubuntu /data/server
' 2>&1 | tail -3

log "Step 3b: upload binaries to all clients (parallel)"
UPLOAD_PIDS=()
for cip in "${C_PUB_ARR[@]}"; do
    (
        scp $SSH_OPTS "${BINS_CLIENT[@]}" "ubuntu@$cip:/tmp/" >/dev/null
        scp $SSH_OPTS "$SCRIPTS/run-mdtest.sh" "ubuntu@$cip:/tmp/" >/dev/null
        ssh $SSH_OPTS "ubuntu@$cip" '
            sudo cp /tmp/rucksfs-remote-client /tmp/rucksfs-bench /usr/local/bin/
            sudo chmod +x /usr/local/bin/rucksfs-remote-client /usr/local/bin/rucksfs-bench
            sudo cp /tmp/run-mdtest.sh /usr/local/bin/
            sudo chmod +x /usr/local/bin/run-mdtest.sh
            sudo mkdir -p /mnt/sut
            sudo chown ubuntu:ubuntu /mnt/sut
        '
    ) &
    UPLOAD_PIDS+=($!)
done
for pid in "${UPLOAD_PIDS[@]}"; do wait "$pid"; done

# -------------------- STEP 4: setup SSH mesh --------------------
log "Step 4: setup SSH mesh for MPI"
bash "$SCRIPTS/setup-ssh-mesh.sh" "$SSH_KEY" "$CLIENT_IPS" "$CLIENT_PRIV_IPS"

# -------------------- STEP 5: generate hostfile --------------------
log "Step 5: generate MPI hostfile"
HOSTFILE_LOCAL="$RESULTS_LOCAL/hostfile"
{
    for priv in "${C_PRIV_ARR[@]}"; do
        echo "$priv slots=8"
    done
} > "$HOSTFILE_LOCAL"
cat "$HOSTFILE_LOCAL" | tee -a "$RESULTS_LOCAL/orchestrator.log"
scp $SSH_OPTS "$HOSTFILE_LOCAL" "ubuntu@${C_PUB_ARR[0]}:/tmp/hostfile" >/dev/null

# -------------------- Helper: run one SUT --------------------
CLIENT0_PUB="${C_PUB_ARR[0]}"
RESULTS_REMOTE_PREFIX="/data/results_round3"

run_sut() {
    local sut="$1"
    local phase_name="$2"
    log ">>> $phase_name: running SUT=$sut"
    # Server side: switch SUT
    ssh $SSH_OPTS "ubuntu@$SERVER_IP" "sudo /usr/local/bin/switch-sut.sh $sut" 2>&1 | tee -a "$RESULTS_LOCAL/switch-sut-$sut.log" | tail -10
    # Client-0 side: run mdtest
    ssh $SSH_OPTS "ubuntu@$CLIENT0_PUB" "
        sudo mkdir -p $RESULTS_REMOTE_PREFIX/$sut
        sudo chown ubuntu:ubuntu $RESULTS_REMOTE_PREFIX/$sut
        /usr/local/bin/run-mdtest.sh $sut $SERVER_INTERNAL_IP /tmp/hostfile $RESULTS_REMOTE_PREFIX/$sut
    " 2>&1 | tee -a "$RESULTS_LOCAL/run-$sut.log"
}

pull_results() {
    local sut="$1"
    log "pulling results for $sut"
    mkdir -p "$RESULTS_LOCAL/raw/$sut"
    scp -r $SSH_OPTS "ubuntu@$CLIENT0_PUB:$RESULTS_REMOTE_PREFIX/$sut/" "$RESULTS_LOCAL/raw/" 2>&1 | tail -3
}

# -------------------- STEP 6-7: Phase 1 RucksFS --------------------
run_sut rucksfs-delta    "Phase 1a"
pull_results rucksfs-delta

run_sut rucksfs-nodelta  "Phase 1b"
pull_results rucksfs-nodelta

# -------------------- STEP 8: parse + breakout check --------------------
log "Step 8: parse Phase 1 results + breakout check"
python3 "$SCRIPTS/parse-results.py" "$RESULTS_LOCAL/raw/rucksfs-delta"   "$RESULTS_LOCAL/phase1_delta.csv"
python3 "$SCRIPTS/parse-results.py" "$RESULTS_LOCAL/raw/rucksfs-nodelta" "$RESULTS_LOCAL/phase1_nodelta.csv"

# Compute delta/nodelta ratio at N=64 and N=128 for create
BREAKOUT=$(python3 <<EOF
import csv
from statistics import median
def load(path):
    with open(path) as f:
        return list(csv.DictReader(f))
def med_create(rows, N):
    vals = [float(r['create']) for r in rows if int(r['N']) == N and r['mode'] == 'hard' and r.get('create')]
    return median(vals) if vals else None
d  = load("$RESULTS_LOCAL/phase1_delta.csv")
nd = load("$RESULTS_LOCAL/phase1_nodelta.csv")
ratios = {}
for N in (64, 128):
    cd  = med_create(d,  N)
    cnd = med_create(nd, N)
    if cd and cnd:
        ratios[N] = cd / cnd
print("RATIOS:", ratios)
ok64  = ratios.get(64,  0) >= 1.5
ok128 = ratios.get(128, 0) >= 2.0
print("BREAKOUT_OK" if (ok64 and ok128) else "BREAKOUT_FAIL")
EOF
)
echo "$BREAKOUT" | tee -a "$RESULTS_LOCAL/orchestrator.log"

BREAKOUT_OK=false
if echo "$BREAKOUT" | grep -q BREAKOUT_OK; then
    BREAKOUT_OK=true
fi

if [ "$BREAKOUT_OK" = false ] && [ -z "${FORCE_CONTINUE:-}" ]; then
    log "*** BREAKOUT CRITERION FAILED ***"
    log "Delta/no-delta ratio at N=64 or N=128 is below the required threshold."
    log "Skipping Phase 2-4 (NFS, JuiceFS). Destroying cluster to save cost."
    ssh $SSH_OPTS "ubuntu@$SERVER_IP" "sudo /usr/local/bin/switch-sut.sh off" 2>&1 | tail -3
    if [ -z "${SKIP_TERRAFORM_DESTROY:-}" ]; then
        cd "$INFRA" && terraform destroy -auto-approve 2>&1 | tail -5
    fi
    exit 3
fi

# -------------------- STEP 9-11: Phase 2-4 baseline systems --------------------
SKIP_SUTS_LIST="${SKIP_SUTS:-}"

maybe_run() {
    local sut="$1"; local phase="$2"
    if echo " $SKIP_SUTS_LIST " | grep -q " $sut "; then
        log "SKIPPING $sut (listed in SKIP_SUTS)"
        return
    fi
    run_sut "$sut" "$phase"
    pull_results "$sut"
    python3 "$SCRIPTS/parse-results.py" "$RESULTS_LOCAL/raw/$sut" "$RESULTS_LOCAL/${sut}.csv"
}

maybe_run nfs            "Phase 2"
maybe_run juicefs-redis  "Phase 3"
maybe_run juicefs-tikv   "Phase 4"

# -------------------- STEP 12: aggregate + plot --------------------
log "Step 12: aggregate results"
# Merge all raw/*/* into a single CSV
python3 <<EOF
import os, csv, glob
from statistics import median
rows = []
for sut_dir in sorted(glob.glob("$RESULTS_LOCAL/raw/*/")):
    sut = os.path.basename(sut_dir.rstrip("/"))
    # Use per-SUT CSV if it exists, otherwise parse on the fly
    pass
EOF
python3 "$SCRIPTS/parse-results.py" "$RESULTS_LOCAL/raw" "$RESULTS_LOCAL/all.csv" 2>&1 || true
# parse-results.py doesn't recurse; do it per-sut
for d in "$RESULTS_LOCAL/raw"/*/; do
    sut=$(basename "$d")
    python3 "$SCRIPTS/parse-results.py" "$d" "$RESULTS_LOCAL/${sut}.csv" 2>&1 || true
done

# -------------------- STEP 13: terraform destroy --------------------
if [ -z "${SKIP_TERRAFORM_DESTROY:-}" ]; then
    log "Step 13: terraform destroy"
    ssh $SSH_OPTS "ubuntu@$SERVER_IP" "sudo /usr/local/bin/switch-sut.sh off" 2>&1 | tail -3 || true
    cd "$INFRA" && terraform destroy -auto-approve 2>&1 | tail -5
else
    log "SKIP_TERRAFORM_DESTROY=1 set — cluster is still running!"
fi

log "DONE. Results in $RESULTS_LOCAL"
echo "$RESULTS_LOCAL" > /tmp/round3_latest_results.txt
