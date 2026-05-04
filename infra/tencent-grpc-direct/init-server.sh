#!/bin/bash
# server init: mount data disk, install base tools (binaries scp'd later)
set -euo pipefail
exec > /var/log/init.log 2>&1
echo "=== [$(date)] server init ==="

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
apt-get install -y build-essential curl

mkdir -p "$DATA_MNT/rucksfs-meta"
chown -R ubuntu:ubuntu "$DATA_MNT"

{ date; uname -a; lscpu; free -h; lsblk; } > "$DATA_MNT/env-info.txt"

systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true
echo "=== [$(date)] server init done ==="
