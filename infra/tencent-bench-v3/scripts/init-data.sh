#!/bin/bash
# cloud-init script for Machine C (Data / NFS Server)
# Installs: NFS server, iperf3
# Controlled benchmark setup: no MinIO, configurable nfsd threads.
set -euo pipefail
exec > /var/log/bench-init.log 2>&1

echo "=== [$(date)] Machine C init starting ==="

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
# 3. NFS Server (kernel nfsd + ext4)
# -------------------------------------------------------
echo "=== Installing NFS Server ==="
apt-get install -y nfs-kernel-server

mkdir -p "$DATA_MNT/nfs-export"
chmod 777 "$DATA_MNT/nfs-export"

# Export to all (within VPC, security group provides access control)
echo "/data/nfs-export *(rw,sync,no_subtree_check,no_root_squash)" >> /etc/exports
exportfs -rav

# Configure nfsd thread count: start with 64 (max for thread scan experiment).
# Can be adjusted at runtime with: rpc.nfsd <N>
NFSD_THREADS=64
sed -i "s/^RPCNFSDCOUNT=.*/RPCNFSDCOUNT=$NFSD_THREADS/" /etc/default/nfs-kernel-server 2>/dev/null || \
  echo "RPCNFSDCOUNT=$NFSD_THREADS" >> /etc/default/nfs-kernel-server
systemctl restart nfs-kernel-server

echo "NFS server ready. Exporting /data/nfs-export. nfsd threads: $NFSD_THREADS"

# Verify thread count
echo "Actual nfsd threads: $(cat /proc/fs/nfsd/threads 2>/dev/null || echo 'unknown')"

# -------------------------------------------------------
# 4. RucksFS DataServer data directory
# -------------------------------------------------------
mkdir -p "$DATA_MNT/rucksfs-data"

# -------------------------------------------------------
# 5. Helper script to change nfsd thread count at runtime
# -------------------------------------------------------
cat > /usr/local/bin/set-nfsd-threads <<'HELPER'
#!/bin/bash
# Usage: set-nfsd-threads <N>
# Changes nfsd thread count without restarting the NFS server.
if [ -z "$1" ]; then
  echo "Current threads: $(cat /proc/fs/nfsd/threads)"
  echo "Usage: set-nfsd-threads <N>"
  exit 1
fi
rpc.nfsd "$1"
echo "nfsd threads set to: $(cat /proc/fs/nfsd/threads)"
HELPER
chmod +x /usr/local/bin/set-nfsd-threads

# -------------------------------------------------------
# 6. Record environment info
# -------------------------------------------------------
{
  echo "=== Machine C Environment ==="
  date
  uname -a
  lscpu
  free -h
  lsblk
  df -h
  ip -4 addr show
  echo "--- NFS status ---"
  systemctl status nfs-kernel-server --no-pager || true
  echo "--- NFS exports ---"
  exportfs -v || true
  echo "--- nfsd threads ---"
  cat /proc/fs/nfsd/threads || true
} > "$DATA_MNT/env-info-data.txt"

# Disable apt auto-updates
systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true

echo "=== [$(date)] Machine C init complete ==="
