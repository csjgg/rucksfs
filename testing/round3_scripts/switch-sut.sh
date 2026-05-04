#!/usr/bin/env bash
# switch-sut.sh — runs ON THE SERVER.
# Stops current SUT, cleans data, starts new SUT. Idempotent.
# Usage: switch-sut.sh <sut>
#   sut ∈ {rucksfs-delta, rucksfs-nodelta, nfs, juicefs-redis, juicefs-tikv, off}
set -uo pipefail

SUT="${1:?sut name required}"
SERVER_DATA=/data/server
mkdir -p "$SERVER_DATA"

log() { printf "[switch-sut %s] %s\n" "$(date +%H:%M:%S)" "$*"; }

stop_all() {
    log "stopping all known SUT processes"
    # RucksFS
    sudo pkill -f rucksfs-metaserver 2>/dev/null || true
    sudo pkill -f rucksfs-dataserver 2>/dev/null || true
    # NFS
    sudo systemctl stop nfs-kernel-server 2>/dev/null || true
    # JuiceFS (any mount)
    for m in $(mount | grep -E "juicefs|fuse" | awk '{print $3}'); do
        sudo umount -l "$m" 2>/dev/null || true
    done
    sudo pkill -9 -f juicefs 2>/dev/null || true
    # Redis
    sudo systemctl stop redis-server 2>/dev/null || true
    sudo pkill -f redis-server 2>/dev/null || true
    # TiKV / PD
    sudo pkill -f '^tikv-server' 2>/dev/null || true
    sudo pkill -f '^pd-server' 2>/dev/null || true
    sudo pkill -f 'tiup' 2>/dev/null || true
    sleep 2
    # Verify nothing left
    if pgrep -l -f "rucksfs-|nfsd|redis-server|tikv-server|pd-server|juicefs" >/dev/null 2>&1; then
        log "WARN: residual processes:"
        pgrep -a -f "rucksfs-|nfsd|redis-server|tikv-server|pd-server|juicefs"
    fi
}

clean_data() {
    log "cleaning $SERVER_DATA (except .keep)"
    sudo rm -rf "$SERVER_DATA"/rucksfs "$SERVER_DATA"/nfs-export "$SERVER_DATA"/juicefs-data \
                 "$SERVER_DATA"/juicefs-cache "$SERVER_DATA"/redis "$SERVER_DATA"/tikv-data \
                 "$SERVER_DATA"/pd-data 2>/dev/null || true
    sudo mkdir -p "$SERVER_DATA"
    sudo chown -R ubuntu:ubuntu "$SERVER_DATA"
    sync
    sudo bash -c 'echo 3 > /proc/sys/vm/drop_caches' 2>/dev/null || true
}

start_rucksfs() {
    local variant="$1"  # with-delta or no-delta
    log "starting rucksfs metaserver+dataserver ($variant)"
    mkdir -p "$SERVER_DATA/rucksfs/meta" "$SERVER_DATA/rucksfs/data"
    nohup /usr/local/bin/rucksfs-metaserver-${variant} \
        --listen 0.0.0.0:8001 --data-dir "$SERVER_DATA/rucksfs/meta" \
        > /data/server/meta-${variant}.log 2>&1 &
    nohup /usr/local/bin/rucksfs-dataserver \
        --listen 0.0.0.0:8002 --data-dir "$SERVER_DATA/rucksfs/data" \
        > /data/server/data.log 2>&1 &
    sleep 3
    # health: port 8001 and 8002 listening
    ss -tlnp | grep -qE ':8001\b' || { log "meta-server not listening"; return 1; }
    ss -tlnp | grep -qE ':8002\b' || { log "data-server not listening"; return 1; }
    log "rucksfs ($variant) ready"
}

start_nfs() {
    log "starting NFS"
    sudo mkdir -p "$SERVER_DATA/nfs-export"
    sudo chmod 777 "$SERVER_DATA/nfs-export"
    # Write exports file idempotently
    sudo bash -c "echo '$SERVER_DATA/nfs-export *(rw,sync,no_subtree_check,no_root_squash)' > /etc/exports"
    sudo exportfs -rav
    sudo systemctl start nfs-kernel-server
    sleep 2
    sudo rpc.nfsd 64 2>/dev/null || true
    log "NFS ready ($(cat /proc/fs/nfsd/threads 2>/dev/null) threads)"
}

start_juicefs_redis() {
    log "starting JuiceFS with Redis backend"
    sudo mkdir -p "$SERVER_DATA/juicefs-data" "$SERVER_DATA/redis"
    sudo chown -R ubuntu:ubuntu "$SERVER_DATA"
    # Start Redis, bind 0.0.0.0 so other local tools can reach; clients don't need it
    sudo bash -c "cat > /etc/redis/redis.conf <<EOF
bind 0.0.0.0
port 6379
dir $SERVER_DATA/redis
save \"\"
appendonly no
maxclients 10000
protected-mode no
EOF"
    sudo systemctl restart redis-server
    sleep 2
    redis-cli ping | grep -q PONG || { log "redis not responding"; return 1; }
    # Format + mount JuiceFS (server exports as NFS-like via its own FUSE; for our tests
    # the server mounts JuiceFS locally and re-exports via NFS to the clients).
    /usr/local/bin/juicefs format --storage file --bucket "$SERVER_DATA/juicefs-data/" \
        redis://localhost:6379/1 rucksfs-jfs-redis 2>&1 | tail -5
    mkdir -p "$SERVER_DATA/jfs-mnt"
    /usr/local/bin/juicefs mount -d redis://localhost:6379/1 "$SERVER_DATA/jfs-mnt" 2>&1 | tail -5
    sleep 2
    mountpoint -q "$SERVER_DATA/jfs-mnt" || { log "juicefs mount failed"; return 1; }
    # Re-export via NFS so clients access via NFS protocol (uniform access path across SUTs)
    sudo bash -c "echo '$SERVER_DATA/jfs-mnt *(rw,sync,no_subtree_check,no_root_squash)' > /etc/exports"
    sudo exportfs -rav
    sudo systemctl start nfs-kernel-server
    sudo rpc.nfsd 64 2>/dev/null || true
    log "JuiceFS+Redis ready"
}

start_juicefs_tikv() {
    log "starting JuiceFS with TiKV backend (single-node)"
    sudo mkdir -p "$SERVER_DATA/tikv-data" "$SERVER_DATA/pd-data" "$SERVER_DATA/juicefs-data"
    sudo chown -R ubuntu:ubuntu "$SERVER_DATA"
    local ip
    ip=$(hostname -I | awk '{print $1}')
    # Start PD and TiKV
    nohup tiup pd --name=pd1 \
        --data-dir="$SERVER_DATA/pd-data" \
        --client-urls="http://0.0.0.0:2379" --advertise-client-urls="http://${ip}:2379" \
        --peer-urls="http://0.0.0.0:2380" --advertise-peer-urls="http://${ip}:2380" \
        --initial-cluster="pd1=http://${ip}:2380" \
        --log-file=/data/server/pd.log > /dev/null 2>&1 &
    sleep 5
    nohup tiup tikv --addr=0.0.0.0:20160 --advertise-addr=${ip}:20160 \
        --data-dir="$SERVER_DATA/tikv-data" \
        --pd-endpoints="${ip}:2379" \
        --log-file=/data/server/tikv.log > /dev/null 2>&1 &
    sleep 15
    /usr/local/bin/juicefs format --storage file --bucket "$SERVER_DATA/juicefs-data/" \
        tikv://${ip}:2379 rucksfs-jfs-tikv 2>&1 | tail -5
    mkdir -p "$SERVER_DATA/jfs-mnt"
    /usr/local/bin/juicefs mount -d tikv://${ip}:2379 "$SERVER_DATA/jfs-mnt" 2>&1 | tail -5
    sleep 3
    mountpoint -q "$SERVER_DATA/jfs-mnt" || { log "juicefs mount failed"; return 1; }
    sudo bash -c "echo '$SERVER_DATA/jfs-mnt *(rw,sync,no_subtree_check,no_root_squash)' > /etc/exports"
    sudo exportfs -rav
    sudo systemctl start nfs-kernel-server
    sudo rpc.nfsd 64 2>/dev/null || true
    log "JuiceFS+TiKV ready"
}

# ---- main ----
stop_all
clean_data

case "$SUT" in
    rucksfs-delta)       start_rucksfs with-delta ;;
    rucksfs-nodelta)     start_rucksfs no-delta ;;
    nfs)                 start_nfs ;;
    juicefs-redis)       start_juicefs_redis ;;
    juicefs-tikv)        start_juicefs_tikv ;;
    off)                 log "all services stopped" ;;
    *)                   log "unknown SUT: $SUT"; exit 1 ;;
esac

log "switch-sut complete → $SUT"
