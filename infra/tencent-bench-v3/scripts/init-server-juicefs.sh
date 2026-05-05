#!/bin/bash
# cloud-init script for Server-JFS (JuiceFS + Redis, 8C16G)
# Redis for JuiceFS metadata, local disk for data backend.
set -euo pipefail
exec > /var/log/bench-init.log 2>&1

echo "=== [$(date)] Server-JFS init starting ==="

# -------------------------------------------------------
# 1. Mount data disk
# -------------------------------------------------------
DATA_DEV="/dev/vdb"
DATA_MNT="/data"

for i in $(seq 1 30); do
  [ -b "$DATA_DEV" ] && break
  echo "Waiting for $DATA_DEV ($i/30)..."
  sleep 2
done

if [ -b "$DATA_DEV" ]; then
  mkfs.ext4 -F "$DATA_DEV"
  mkdir -p "$DATA_MNT"
  mount "$DATA_DEV" "$DATA_MNT"
  echo "$DATA_DEV $DATA_MNT ext4 defaults,noatime 0 2" >> /etc/fstab
  echo "Data disk mounted at $DATA_MNT"
else
  echo "WARNING: $DATA_DEV not found, using root disk"
  mkdir -p "$DATA_MNT"
fi

# -------------------------------------------------------
# 2. Install Redis
# -------------------------------------------------------
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y redis-server iperf3 sysstat

# Configure Redis for metadata workload
cat > /etc/redis/redis.conf <<'REDISCONF'
bind 0.0.0.0
port 6379
protected-mode no
daemonize yes
pidfile /var/run/redis/redis-server.pid
logfile /var/log/redis/redis-server.log
dir /data/redis

# Memory
maxmemory 12gb
maxmemory-policy noeviction

# Persistence (AOF for durability)
appendonly yes
appendfsync everysec

# Performance
tcp-backlog 511
tcp-keepalive 300
databases 16
save ""
REDISCONF

mkdir -p /data/redis
chown redis:redis /data/redis

systemctl restart redis-server
systemctl enable redis-server

# Verify
redis-cli ping
echo "Redis running on port 6379"

# -------------------------------------------------------
# 3. Install JuiceFS
# -------------------------------------------------------
JFS_VERSION="1.2.3"
curl -sSL "https://github.com/juicedata/juicefs/releases/download/v${JFS_VERSION}/juicefs-${JFS_VERSION}-linux-amd64.tar.gz" \
    | tar -xz -C /usr/local/bin juicefs
chmod +x /usr/local/bin/juicefs
juicefs version
echo "JuiceFS installed"

# Create local data directory for JuiceFS data backend
mkdir -p /data/juicefs-data
mkdir -p /data/juicefs-cache

# -------------------------------------------------------
# 4. Performance tuning
# -------------------------------------------------------
systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true

# -------------------------------------------------------
# 5. Record environment info
# -------------------------------------------------------
{
  echo "=== Server-JFS Environment ==="
  date
  uname -a
  lscpu | head -15
  free -h
  df -h /data
  echo "--- Redis ---"
  redis-cli INFO server | head -10
  redis-cli INFO memory | head -5
  echo "--- JuiceFS ---"
  juicefs version
} > /data/env-info.txt

echo "=== [$(date)] Server-JFS init complete ==="
