#!/bin/bash
# cloud-init script for Machine A (Client / Test Driver)
# Installs: mdtest, pjdfstest, FUSE, JuiceFS, NFS client, Rust, iperf3
set -euo pipefail
exec > /var/log/bench-init.log 2>&1

echo "=== [$(date)] Machine A init starting ==="

# -------------------------------------------------------
# 1. Mount data disk
# -------------------------------------------------------
DATA_DEV="/dev/vdb"
DATA_MNT="/data"

# Wait for device
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
  mpich automake autoconf libtool \
  fuse3 libfuse3-dev \
  nfs-common \
  iperf3 \
  pkg-config libssl-dev \
  linux-tools-common linux-tools-generic \
  perl

# Enable FUSE allow_other
grep -q "^user_allow_other" /etc/fuse.conf 2>/dev/null || \
  echo "user_allow_other" >> /etc/fuse.conf

# -------------------------------------------------------
# 3. mdtest (IOR project)
# -------------------------------------------------------
echo "=== Installing mdtest ==="
cd /opt
git clone https://github.com/hpc/ior.git
cd ior
./bootstrap
./configure --prefix=/usr/local
make -j$(nproc)
make install
echo "mdtest version: $(mdtest -V 2>&1 | head -1 || true)"

# -------------------------------------------------------
# 4. pjdfstest
# -------------------------------------------------------
echo "=== Installing pjdfstest ==="
cd /opt
git clone https://github.com/pjd/pjdfstest.git
cd pjdfstest
autoreconf -ifs
./configure
make
ln -sf /opt/pjdfstest/pjdfstest /usr/local/bin/pjdfstest

# -------------------------------------------------------
# 5. JuiceFS client
# -------------------------------------------------------
echo "=== Installing JuiceFS ==="
curl -sSL https://d.juicefs.com/install | sh -

# -------------------------------------------------------
# 6. Rust toolchain (for compiling RucksFS)
# -------------------------------------------------------
echo "=== Installing Rust ==="
su - ubuntu -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y'

# -------------------------------------------------------
# 7. Create mount point directories
# -------------------------------------------------------
mkdir -p /mnt/rucksfs-embedded
mkdir -p /mnt/rucksfs-dist
mkdir -p /mnt/juicefs-mysql
mkdir -p /mnt/juicefs-redis
mkdir -p /mnt/nfs
mkdir -p "$DATA_MNT/ext4-bench"
mkdir -p "$DATA_MNT/rucksfs-local"
mkdir -p "$DATA_MNT/test-results"

# -------------------------------------------------------
# 8. Record environment info
# -------------------------------------------------------
{
  echo "=== Machine A Environment ==="
  date
  uname -a
  lscpu
  free -h
  lsblk
  df -h
  ip -4 addr show
} > "$DATA_MNT/env-info-client.txt"

# -------------------------------------------------------
# 9. Performance tuning
# -------------------------------------------------------
# Disable apt auto-updates during benchmark
systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true

echo "=== [$(date)] Machine A init complete ==="
