#!/bin/bash
# cloud-init script for Server-TiKV (JuiceFS + TiKV, 8C16G)
# TiKV (single-node) for JuiceFS metadata, local disk for data backend.
set -euo pipefail
exec > /var/log/bench-init.log 2>&1

echo "=== [$(date)] Server-TiKV init starting ==="

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
# 2. Install TiKV (via TiUP — official TiDB toolchain)
# -------------------------------------------------------
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y curl iperf3 sysstat

# Install TiUP
curl --proto '=https' --tlsv1.2 -sSf https://tiup-mirrors.pingcap.com/install.sh | sh
export PATH="$PATH:/root/.tiup/bin"

# Install PD and TiKV components
tiup install pd tikv

# Create data directories
mkdir -p /data/tikv-data /data/pd-data

# Start PD (Placement Driver — required by TiKV)
nohup tiup pd \
    --data-dir=/data/pd-data \
    --client-urls=http://0.0.0.0:2379 \
    --peer-urls=http://0.0.0.0:2380 \
    --log-file=/var/log/pd.log \
    > /dev/null 2>&1 &
sleep 5
echo "PD started"

# Start TiKV
nohup tiup tikv \
    --pd-endpoints=http://127.0.0.1:2379 \
    --data-dir=/data/tikv-data \
    --addr=0.0.0.0:20160 \
    --status-addr=0.0.0.0:20180 \
    --log-file=/var/log/tikv.log \
    > /dev/null 2>&1 &
sleep 10
echo "TiKV started"

# -------------------------------------------------------
# 3. Install JuiceFS
# -------------------------------------------------------
JFS_VERSION="1.2.3"
curl -sSL "https://github.com/juicedata/juicefs/releases/download/v${JFS_VERSION}/juicefs-${JFS_VERSION}-linux-amd64.tar.gz" \
    | tar -xz -C /usr/local/bin juicefs
chmod +x /usr/local/bin/juicefs
juicefs version
echo "JuiceFS installed"

# -------------------------------------------------------
# 4. Performance tuning
# -------------------------------------------------------
systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true

# -------------------------------------------------------
# 5. Record environment info
# -------------------------------------------------------
{
  echo "=== Server-TiKV Environment ==="
  date
  uname -a
  lscpu | head -15
  free -h
  df -h /data
  echo "--- TiUP ---"
  /root/.tiup/bin/tiup --version 2>/dev/null || echo "tiup not in path"
  echo "--- JuiceFS ---"
  juicefs version
} > /data/env-info.txt

echo "=== [$(date)] Server-TiKV init complete ==="
