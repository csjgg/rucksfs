#!/bin/bash
# cloud-init for Server-1 (RucksFS MDS + DS, dedicated machine)
# Only installs base tools. RucksFS binaries deployed manually.
set -euo pipefail
exec > /var/log/bench-init.log 2>&1

echo "=== [$(date)] Server-1 (RucksFS) init starting ==="

# Mount data disk
DATA_DEV="/dev/vdb"
DATA_MNT="/data"
for i in $(seq 1 30); do [ -b "$DATA_DEV" ] && break; sleep 2; done
if [ -b "$DATA_DEV" ]; then
  mkfs.ext4 -F "$DATA_DEV"
  mkdir -p "$DATA_MNT"
  mount "$DATA_DEV" "$DATA_MNT"
  echo "$DATA_DEV $DATA_MNT ext4 defaults,noatime 0 2" >> /etc/fstab
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y build-essential git curl wget iperf3 sysstat

# RucksFS data directories
mkdir -p "$DATA_MNT/rucksfs-meta"
mkdir -p "$DATA_MNT/rucksfs-data"

# Record environment
{
  echo "=== Server-1 (RucksFS) Environment ==="
  date; uname -a; lscpu; free -h; lsblk; df -h; ip -4 addr show
} > "$DATA_MNT/env-info.txt"

systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true
echo "=== [$(date)] Server-1 init complete ==="
