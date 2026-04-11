#!/bin/bash
# cloud-init script for Machine C (Data Server)
# Installs: MinIO, NFS server, iperf3
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
  iperf3

# -------------------------------------------------------
# 3. MinIO (S3-compatible object storage for JuiceFS)
# -------------------------------------------------------
echo "=== Installing MinIO ==="
mkdir -p "$DATA_MNT/minio"

wget -q https://dl.min.io/server/minio/release/linux-amd64/minio -O /usr/local/bin/minio
chmod +x /usr/local/bin/minio

# Create systemd service
cat > /etc/systemd/system/minio.service <<'MINIO_SVC'
[Unit]
Description=MinIO Object Storage
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
Environment="MINIO_ROOT_USER=minioadmin"
Environment="MINIO_ROOT_PASSWORD=minioadmin"
ExecStart=/usr/local/bin/minio server /data/minio --address :9000 --console-address :9001
Restart=always
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
MINIO_SVC

systemctl daemon-reload
systemctl enable --now minio

# Wait for MinIO to start
for i in $(seq 1 30); do
  curl -sf http://127.0.0.1:9000/minio/health/live && break
  echo "Waiting for MinIO ($i/30)..."
  sleep 2
done

# Install mc (MinIO client) and create buckets
wget -q https://dl.min.io/client/mc/release/linux-amd64/mc -O /usr/local/bin/mc
chmod +x /usr/local/bin/mc
mc alias set local http://127.0.0.1:9000 minioadmin minioadmin
mc mb local/jfs-mysql --ignore-existing
mc mb local/jfs-redis --ignore-existing

echo "MinIO ready. Buckets: jfs-mysql, jfs-redis."

# -------------------------------------------------------
# 4. NFS Server (kernel nfsd + ext4)
# -------------------------------------------------------
echo "=== Installing NFS Server ==="
apt-get install -y nfs-kernel-server

mkdir -p "$DATA_MNT/nfs-export"
chmod 777 "$DATA_MNT/nfs-export"

# Export to all (within VPC, security group provides access control)
echo "/data/nfs-export *(rw,sync,no_subtree_check,no_root_squash)" >> /etc/exports
exportfs -rav
systemctl restart nfs-kernel-server

echo "NFS server ready. Exporting /data/nfs-export."

# -------------------------------------------------------
# 5. RucksFS DataServer data directory
# -------------------------------------------------------
mkdir -p "$DATA_MNT/rucksfs-data"

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
  echo "--- MinIO status ---"
  systemctl status minio --no-pager || true
  echo "--- NFS status ---"
  systemctl status nfs-kernel-server --no-pager || true
  echo "--- NFS exports ---"
  exportfs -v || true
} > "$DATA_MNT/env-info-data.txt"

# Disable apt auto-updates
systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true

echo "=== [$(date)] Machine C init complete ==="
