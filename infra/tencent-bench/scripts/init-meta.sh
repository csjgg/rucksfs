#!/bin/bash
# cloud-init script for Machine B (Metadata Server)
# Installs: MySQL 8.0, Redis 7.x, iperf3
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

# Move MySQL data to SSD data disk
systemctl stop mysql
mkdir -p "$DATA_MNT/mysql"
rsync -av /var/lib/mysql/ "$DATA_MNT/mysql/"
chown -R mysql:mysql "$DATA_MNT/mysql"

# Configure MySQL
cat > /etc/mysql/mysql.conf.d/bench.cnf <<'MYCNF'
[mysqld]
# Bind to all interfaces (for client access from Machine A)
bind-address = 0.0.0.0

# Data directory on SSD
datadir = /data/mysql

# InnoDB tuning for fair benchmark (sync writes)
innodb_flush_log_at_trx_commit = 1
innodb_buffer_pool_size = 4G
innodb_log_file_size = 256M
innodb_flush_method = O_DIRECT

# Connection limits
max_connections = 200
MYCNF

# AppArmor: allow MySQL to use /data/mysql
if [ -f /etc/apparmor.d/usr.sbin.mysqld ]; then
  sed -i 's|/var/lib/mysql/|/data/mysql/|g' /etc/apparmor.d/usr.sbin.mysqld
  apparmor_parser -r /etc/apparmor.d/usr.sbin.mysqld 2>/dev/null || true
fi

systemctl start mysql

# Create JuiceFS database and user
mysql -e "CREATE DATABASE IF NOT EXISTS juicefs;"
mysql -e "CREATE USER IF NOT EXISTS 'juicefs'@'%' IDENTIFIED BY 'juicefs_bench';"
mysql -e "GRANT ALL PRIVILEGES ON juicefs.* TO 'juicefs'@'%';"
mysql -e "FLUSH PRIVILEGES;"

echo "MySQL ready. JuiceFS DB/user created."

# -------------------------------------------------------
# 4. Redis 7.x
# -------------------------------------------------------
echo "=== Installing Redis ==="
apt-get install -y redis-server

# Move Redis data to SSD
mkdir -p "$DATA_MNT/redis"
chown redis:redis "$DATA_MNT/redis"

# Configure Redis
cat > /etc/redis/redis.conf.d/bench.conf <<'REDISCNF' || true
bind 0.0.0.0
port 6379
dir /data/redis
appendonly yes
appendfsync everysec
maxmemory 8gb
maxmemory-policy noeviction
REDISCNF

# Redis on Ubuntu uses /etc/redis/redis.conf directly
sed -i 's/^bind .*/bind 0.0.0.0/' /etc/redis/redis.conf
# Keep default dir (/var/lib/redis) — systemd ReadWritePaths blocks /data/redis
# sed -i 's|^dir .*|dir /data/redis|' /etc/redis/redis.conf
sed -i 's/^# *appendonly .*/appendonly yes/' /etc/redis/redis.conf
sed -i 's/^appendonly .*/appendonly yes/' /etc/redis/redis.conf
sed -i 's/^# *appendfsync .*/appendfsync everysec/' /etc/redis/redis.conf

# Add systemd override to allow writing to /data/redis if we want it later
mkdir -p /etc/systemd/system/redis-server.service.d
cat > /etc/systemd/system/redis-server.service.d/data-dir.conf <<'SYSOVER'
[Service]
ReadWritePaths=-/data/redis
SYSOVER
systemctl daemon-reload

systemctl restart redis-server

echo "Redis ready. Listening on 0.0.0.0:6379."

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
  echo "--- Redis status ---"
  systemctl status redis-server --no-pager || true
} > "$DATA_MNT/env-info-meta.txt"

# Disable apt auto-updates
systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true

echo "=== [$(date)] Machine B init complete ==="
