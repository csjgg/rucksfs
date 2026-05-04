#!/bin/bash
# client init: just install base tools, bench binary will be scp'd
set -euo pipefail
exec > /var/log/init.log 2>&1
echo "=== [$(date)] client init ==="

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y build-essential curl

{ date; uname -a; lscpu; free -h; } > /home/ubuntu/env-info.txt
chown ubuntu:ubuntu /home/ubuntu/env-info.txt

systemctl disable --now apt-daily.timer apt-daily-upgrade.timer 2>/dev/null || true
echo "=== [$(date)] client init done ==="
