#!/usr/bin/env bash
# =============================================================================
# testing/scripts/collect-metrics.sh
# System metrics collection (CPU, memory, disk I/O) during benchmark
#
# Usage:
#   collect-metrics.sh start <output_dir> <interval_sec>
#   collect-metrics.sh stop
# =============================================================================

set -euo pipefail

SUBCMD="${1:-help}"
METRICS_DIR="${2:-/tmp/rucksfs-metrics}"
INTERVAL="${3:-1}"
PIDFILE="/tmp/rucksfs-metrics.pid"

case "$SUBCMD" in
  start)
    mkdir -p "$METRICS_DIR"

    # Stop any previous collection
    if [ -f "$PIDFILE" ]; then
      OLD_PID=$(cat "$PIDFILE")
      kill "$OLD_PID" 2>/dev/null || true
      rm -f "$PIDFILE"
    fi

    echo "==> Starting metrics collection (interval=${INTERVAL}s)..."

    # vmstat (CPU + memory)
    if command -v vmstat &>/dev/null; then
      nohup vmstat "$INTERVAL" > "$METRICS_DIR/vmstat.log" 2>&1 &
      echo $! >> "$METRICS_DIR/pids"
      echo "  vmstat: PID=$!"
    fi

    # iostat (disk I/O)
    if command -v iostat &>/dev/null; then
      nohup iostat -x "$INTERVAL" > "$METRICS_DIR/iostat.log" 2>&1 &
      echo $! >> "$METRICS_DIR/pids"
      echo "  iostat: PID=$!"
    fi

    # mpstat (per-CPU)
    if command -v mpstat &>/dev/null; then
      nohup mpstat -P ALL "$INTERVAL" > "$METRICS_DIR/mpstat.log" 2>&1 &
      echo $! >> "$METRICS_DIR/pids"
      echo "  mpstat: PID=$!"
    fi

    # Simple top snapshot every interval (fallback if no sysstat)
    nohup bash -c "
      while true; do
        echo '--- \$(date +%Y-%m-%dT%H:%M:%S) ---' >> $METRICS_DIR/top.log
        top -bn1 | head -20 >> $METRICS_DIR/top.log
        echo '' >> $METRICS_DIR/top.log
        sleep $INTERVAL
      done
    " > /dev/null 2>&1 &
    echo $! >> "$METRICS_DIR/pids"
    echo "  top sampler: PID=$!"

    # Record start time
    date +%s > "$METRICS_DIR/start_time"
    echo "metrics_started dir=$METRICS_DIR"
    ;;

  stop)
    echo "==> Stopping metrics collection..."

    if [ -f "$METRICS_DIR/pids" ]; then
      while read -r pid; do
        kill "$pid" 2>/dev/null || true
      done < "$METRICS_DIR/pids"
      rm -f "$METRICS_DIR/pids"
      echo "  Stopped all collectors"
    else
      echo "  No active collectors found"
    fi

    # Record end time
    if [ -d "$METRICS_DIR" ]; then
      date +%s > "$METRICS_DIR/end_time"

      # Generate summary
      if [ -f "$METRICS_DIR/start_time" ] && [ -f "$METRICS_DIR/end_time" ]; then
        START=$(cat "$METRICS_DIR/start_time")
        END=$(cat "$METRICS_DIR/end_time")
        DURATION=$((END - START))
        echo "  Duration: ${DURATION}s"
      fi

      echo "  Files:"
      ls -lh "$METRICS_DIR"/*.log 2>/dev/null || echo "    (no log files)"
    fi

    echo "metrics_stopped"
    ;;

  help|*)
    echo "Usage: $0 <start|stop> [output_dir] [interval_sec]"
    echo ""
    echo "  start <dir> <interval>  Start collecting CPU/memory/IO metrics"
    echo "  stop                    Stop all collectors"
    ;;
esac
