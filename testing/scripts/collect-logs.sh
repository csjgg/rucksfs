#!/usr/bin/env bash
# =============================================================================
# testing/scripts/collect-logs.sh
# Collect relevant logs from remote host after benchmark
#
# Usage:
#   collect-logs.sh <output_dir> [rucksfs_log_path]
# =============================================================================

set -euo pipefail

LOGS_DIR="${1:-/tmp/rucksfs-logs}"
RUCKSFS_LOG="${2:-/opt/rucksfs-test/rucksfs.log}"

mkdir -p "$LOGS_DIR"

echo "==> Collecting logs to $LOGS_DIR..."

# RucksFS application log
if [ -f "$RUCKSFS_LOG" ]; then
  cp "$RUCKSFS_LOG" "$LOGS_DIR/rucksfs.log"
  echo "  Copied: rucksfs.log ($(wc -l < "$RUCKSFS_LOG") lines)"
else
  echo "  Skipped: rucksfs.log (not found at $RUCKSFS_LOG)"
fi

# Kernel messages (FUSE-related)
if command -v dmesg &>/dev/null; then
  dmesg | grep -i -E 'fuse|rucksfs|mount' > "$LOGS_DIR/dmesg_fuse.log" 2>/dev/null || true
  echo "  Captured: dmesg_fuse.log"
fi

# Recent kernel messages (last 500 lines)
if command -v dmesg &>/dev/null; then
  dmesg | tail -500 > "$LOGS_DIR/dmesg_recent.log" 2>/dev/null || true
  echo "  Captured: dmesg_recent.log"
fi

# Journal logs for FUSE if available
if command -v journalctl &>/dev/null; then
  journalctl -k --since "1 hour ago" --no-pager > "$LOGS_DIR/journal_kernel.log" 2>/dev/null || true
  echo "  Captured: journal_kernel.log"
fi

# Mount info
mount | grep -i fuse > "$LOGS_DIR/fuse_mounts.log" 2>/dev/null || true
echo "  Captured: fuse_mounts.log"

# /proc info
if [ -f /proc/filesystems ]; then
  grep fuse /proc/filesystems > "$LOGS_DIR/proc_fuse.log" 2>/dev/null || true
fi

echo "==> Log collection complete"
echo "  Files:"
ls -lh "$LOGS_DIR"/ 2>/dev/null || true
