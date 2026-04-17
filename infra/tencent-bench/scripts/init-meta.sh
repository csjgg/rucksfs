#!/bin/bash
# cloud-init script for Machine B (Metadata Server)
# Installs: iperf3 (RucksFS binary deployed manually after cloud-init)
# Controlled benchmark setup: no MySQL/TiKV, dedicated to RucksFS MetadataServer.
set -euo pipefail
exec > /var/log/bench-init.log 2>&1

echo "=== [$(date)] Machine B init starting ==="

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
# 2. System packages
# -------------------------------------------------------
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y \
  build-essential git curl wget \
  iperf3 sysstat

# -------------------------------------------------------
# 3. RucksFS MetadataServer data directory
# -------------------------------------------------------
mkdir -p "$DATA_MNT/rucksfs-meta"

# -------------------------------------------------------
# 4. Record environment info
# -------------------------------------------------------
{
  echo "=== Machine B Environment ==="
  date
  uname -a
  lscpu
  free -h
  lsblk
  df -h
  ip -4 addr show
} > "$DATA_MNT/env-info-meta.txt"

# Disable apt auto-updates
systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true

echo "=== [$(date)] Machine B init complete ==="
