#!/bin/bash
# cloud-init script for Machine A (Client / Test Driver)
# Installs: OpenMPI, mdtest, FUSE, NFS client
# Controlled benchmark setup: no JuiceFS, single client node.
set -euo pipefail
exec > /var/log/bench-init.log 2>&1

echo "=== [$(date)] Machine A init starting ==="

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
  openmpi-bin libopenmpi-dev \
  automake autoconf libtool \
  fuse3 libfuse3-dev \
  nfs-common \
  iperf3 sysstat \
  pkg-config libssl-dev \
  perl

# Enable FUSE allow_other
grep -q "^user_allow_other" /etc/fuse.conf 2>/dev/null || \
  echo "user_allow_other" >> /etc/fuse.conf

# -------------------------------------------------------
# 3. SSH key for MPI (single-node, but needed for mpirun)
# -------------------------------------------------------
echo "=== Setting up SSH ==="
for HOME_DIR in /home/ubuntu /root; do
  mkdir -p "$HOME_DIR/.ssh"

  if [ ! -f "$HOME_DIR/.ssh/id_ed25519" ]; then
    ssh-keygen -t ed25519 -f "$HOME_DIR/.ssh/id_ed25519" -N "" -q
  fi

  cat "$HOME_DIR/.ssh/id_ed25519.pub" >> "$HOME_DIR/.ssh/authorized_keys"

  cat > "$HOME_DIR/.ssh/config" <<'SSHCONF'
Host 10.0.*
  StrictHostKeyChecking no
  UserKnownHostsFile /dev/null
  LogLevel ERROR
SSHCONF

  chmod 700 "$HOME_DIR/.ssh"
  chmod 600 "$HOME_DIR/.ssh/id_ed25519" "$HOME_DIR/.ssh/config"
  chmod 644 "$HOME_DIR/.ssh/id_ed25519.pub"
done
chown -R ubuntu:ubuntu /home/ubuntu/.ssh

# -------------------------------------------------------
# 4. mdtest (IOR project, built with OpenMPI)
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
# 5. Create mount point directories
# -------------------------------------------------------
mkdir -p /mnt/rucksfs-dist
mkdir -p /mnt/nfs
mkdir -p /mnt/nfs-ac
mkdir -p "$DATA_MNT/test-results"

# -------------------------------------------------------
# 6. Record environment info
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
  echo "--- OpenMPI ---"
  mpirun --version || true
  echo "--- mdtest ---"
  mdtest -V 2>&1 | head -3 || true
} > "$DATA_MNT/env-info-client.txt"

# -------------------------------------------------------
# 7. Performance tuning
# -------------------------------------------------------
systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true

echo "=== [$(date)] Machine A init complete ==="
