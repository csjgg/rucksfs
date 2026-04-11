#!/bin/bash
# cloud-init script for Machine B (Metadata Server)
# Installs: MySQL 8.0, TiKV (single-node PD+TiKV), iperf3
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
  iperf3

# -------------------------------------------------------
# 3. MySQL 8.0
# -------------------------------------------------------
echo "=== Installing MySQL 8.0 ==="
apt-get install -y mysql-server

systemctl stop mysql
mkdir -p "$DATA_MNT/mysql"
rsync -av /var/lib/mysql/ "$DATA_MNT/mysql/"
chown -R mysql:mysql "$DATA_MNT/mysql"

sed -i 's/^bind-address.*127.0.0.1/# bind-address = 127.0.0.1/' /etc/mysql/mysql.conf.d/mysqld.cnf
sed -i 's/^mysqlx-bind-address.*127.0.0.1/# mysqlx-bind-address = 127.0.0.1/' /etc/mysql/mysql.conf.d/mysqld.cnf

cat > /etc/mysql/mysql.conf.d/bench.cnf <<'MYCNF'
[mysqld]
bind-address = 0.0.0.0
datadir = /data/mysql
innodb_flush_log_at_trx_commit = 1
innodb_buffer_pool_size = 8G
innodb_log_file_size = 512M
innodb_flush_method = O_DIRECT
max_connections = 500
MYCNF

if [ -f /etc/apparmor.d/usr.sbin.mysqld ]; then
  sed -i 's|/var/lib/mysql/|/data/mysql/|g' /etc/apparmor.d/usr.sbin.mysqld
  apparmor_parser -r /etc/apparmor.d/usr.sbin.mysqld 2>/dev/null || true
fi

systemctl start mysql

mysql -e "CREATE DATABASE IF NOT EXISTS juicefs;"
mysql -e "CREATE USER IF NOT EXISTS 'juicefs'@'%' IDENTIFIED BY 'juicefs_bench';"
mysql -e "GRANT ALL PRIVILEGES ON juicefs.* TO 'juicefs'@'%';"
mysql -e "FLUSH PRIVILEGES;"

echo "MySQL ready. JuiceFS DB/user created."

# -------------------------------------------------------
# 4. TiKV (single-node: PD + TiKV via tiup)
# -------------------------------------------------------
echo "=== Installing TiKV ==="
mkdir -p "$DATA_MNT/tikv"
export HOME="/root"

# Install tiup (TiDB's component manager)
curl --proto '=https' --tlsv1.2 -sSf https://tiup-mirrors.pingcap.com/install.sh | sh
export PATH="/root/.tiup/bin:$PATH"

# Install PD and TiKV components
tiup install pd tikv

# Create data directories
mkdir -p "$DATA_MNT/tikv/pd" "$DATA_MNT/tikv/kv"

# Create systemd service for PD (Placement Driver)
cat > /etc/systemd/system/tikv-pd.service <<'PDSVC'
[Unit]
Description=TiKV Placement Driver
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
Environment="PATH=/root/.tiup/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
ExecStart=/root/.tiup/components/pd/*/pd-server \
  --name=pd1 \
  --data-dir=/data/tikv/pd \
  --client-urls=http://0.0.0.0:2379 \
  --peer-urls=http://0.0.0.0:2380 \
  --log-file=/data/tikv/pd.log
Restart=always
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
PDSVC

# Find the actual PD binary path
PD_BIN=$(find /root/.tiup/components/pd -name "pd-server" -type f | head -1)
TIKV_BIN=$(find /root/.tiup/components/tikv -name "tikv-server" -type f | head -1)

# Update service files with actual paths
sed -i "s|/root/.tiup/components/pd/\*/pd-server|$PD_BIN|" /etc/systemd/system/tikv-pd.service

# Create systemd service for TiKV
cat > /etc/systemd/system/tikv-server.service <<TIKVEOF
[Unit]
Description=TiKV Server
After=tikv-pd.service
Requires=tikv-pd.service

[Service]
Type=simple
User=root
ExecStart=$TIKV_BIN \
  --pd-endpoints=http://127.0.0.1:2379 \
  --data-dir=/data/tikv/kv \
  --addr=0.0.0.0:20160 \
  --log-file=/data/tikv/tikv.log
Restart=always
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
TIKVEOF

systemctl daemon-reload
systemctl enable --now tikv-pd
sleep 5
systemctl enable --now tikv-server
sleep 3

echo "TiKV ready. PD: 0.0.0.0:2379, TiKV: 0.0.0.0:20160"

# -------------------------------------------------------
# 5. RucksFS MetadataServer data directory
# -------------------------------------------------------
mkdir -p "$DATA_MNT/rucksfs-meta"

# -------------------------------------------------------
# 6. Record environment info
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
  echo "--- MySQL status ---"
  systemctl status mysql --no-pager || true
  echo "--- TiKV PD status ---"
  systemctl status tikv-pd --no-pager || true
  echo "--- TiKV Server status ---"
  systemctl status tikv-server --no-pager || true
} > "$DATA_MNT/env-info-meta.txt"

systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true

echo "=== [$(date)] Machine B init complete ==="
