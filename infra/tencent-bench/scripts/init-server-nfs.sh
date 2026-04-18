#!/bin/bash
# cloud-init for Server-2 (NFS only, dedicated machine)
# No RucksFS processes on this machine.
set -euo pipefail
exec > /var/log/bench-init.log 2>&1

echo "=== [$(date)] Server-2 (NFS) init starting ==="

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
apt-get install -y build-essential git curl wget iperf3 sysstat nfs-kernel-server

# NFS export
mkdir -p "$DATA_MNT/nfs-export"
chmod 777 "$DATA_MNT/nfs-export"
echo "/data/nfs-export *(rw,sync,no_subtree_check,no_root_squash)" >> /etc/exports
exportfs -rav

# 64 nfsd threads (validated optimal in previous experiment)
NFSD_THREADS=64
sed -i "s/^RPCNFSDCOUNT=.*/RPCNFSDCOUNT=$NFSD_THREADS/" /etc/default/nfs-kernel-server 2>/dev/null || \
  echo "RPCNFSDCOUNT=$NFSD_THREADS" >> /etc/default/nfs-kernel-server
systemctl restart nfs-kernel-server

# Helper to adjust threads at runtime
cat > /usr/local/bin/set-nfsd-threads <<'HELPER'
#!/bin/bash
if [ -z "$1" ]; then echo "Current: $(cat /proc/fs/nfsd/threads)"; exit 0; fi
rpc.nfsd "$1"; echo "Set to: $(cat /proc/fs/nfsd/threads)"
HELPER
chmod +x /usr/local/bin/set-nfsd-threads

# Record environment
{
  echo "=== Server-2 (NFS) Environment ==="
  date; uname -a; lscpu; free -h; lsblk; df -h; ip -4 addr show
  echo "--- NFS ---"
  systemctl status nfs-kernel-server --no-pager || true
  exportfs -v || true
  echo "nfsd threads: $(cat /proc/fs/nfsd/threads 2>/dev/null || echo unknown)"
} > "$DATA_MNT/env-info.txt"

systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true
echo "=== [$(date)] Server-2 init complete ==="
