#!/usr/bin/env bash
# =============================================================================
# benchmark/local_run.sh — Build, mount, benchmark, unmount in one shot
#
# Self-contained local execution script for RucksFS benchmarks.
# All paths are relative to the benchmark/ directory — no system directories
# are used. Designed for agent-driven or manual one-command execution.
#
# Usage:
#   ./benchmark/local_run.sh [options]
#
# Options:
#   --correctness-only     Run only correctness tests
#   --performance-only     Run only performance benchmarks
#   --num-files N          Files per benchmark (default: 1000)
#   --num-dirs M           Directories for multi-dir tests (default: 50)
#   --max-threads T        Max thread count for concurrency (default: 4)
#   --skip-pjdfstest       Skip pjdfstest (default: skipped)
#   --skip-build           Skip cargo build (use existing binary)
#   --release              Use release build (default: debug for speed)
#   --keep                 Keep mountpoint and data after run (no cleanup)
#
# Exit codes:
#   0  All tests passed
#   1  One or more tests failed
#   2  Setup error (build failure, mount failure, etc.)
#
# Directories created (under benchmark/):
#   mnt/    — FUSE mountpoint (cleaned up after run)
#   data/   — RocksDB + RawDisk storage (cleaned up after run)
#
# Results written to:
#   benchmark/results/*.csv   — machine-parseable performance data
#   benchmark/results/*.log   — human-readable test logs
# =============================================================================

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MOUNTPOINT="$SCRIPT_DIR/mnt"
DATA_DIR="$SCRIPT_DIR/data"
RUCKSFS_PID=""

# Defaults (lightweight for quick verification)
NUM_FILES=1000
NUM_DIRS=50
MAX_THREADS=4
SKIP_PJDFSTEST=true
SKIP_BUILD=false
BUILD_PROFILE="debug"
KEEP_DATA=false
BENCH_ARGS=()

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

while [[ $# -gt 0 ]]; do
    case "$1" in
        --correctness-only)  BENCH_ARGS+=("--correctness-only"); shift ;;
        --performance-only)  BENCH_ARGS+=("--performance-only"); shift ;;
        --num-files)         NUM_FILES="$2"; shift 2 ;;
        --num-dirs)          NUM_DIRS="$2"; shift 2 ;;
        --max-threads)       MAX_THREADS="$2"; shift 2 ;;
        --skip-pjdfstest)    SKIP_PJDFSTEST=true; shift ;;
        --skip-build)        SKIP_BUILD=true; shift ;;
        --release)           BUILD_PROFILE="release"; shift ;;
        --keep)              KEEP_DATA=true; shift ;;
        -h|--help)
            # Print the header comment block as help
            sed -n '2,/^# =====/p' "$0" | head -n -1 | sed 's/^# \?//'
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 2 ;;
    esac
done

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} $*"; }
ok()   { echo -e "${GREEN}[OK]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
err()  { echo -e "${RED}[ERROR]${NC} $*"; }

# ---------------------------------------------------------------------------
# Cleanup on exit (always unmount, optionally remove data)
# ---------------------------------------------------------------------------

cleanup() {
    log "Cleaning up..."

    # Unmount FUSE
    if mountpoint -q "$MOUNTPOINT" 2>/dev/null; then
        log "Unmounting $MOUNTPOINT"
        fusermount -u "$MOUNTPOINT" 2>/dev/null || fusermount3 -u "$MOUNTPOINT" 2>/dev/null || true
        sleep 1
    fi

    # Kill rucksfs process
    if [[ -n "$RUCKSFS_PID" ]] && kill -0 "$RUCKSFS_PID" 2>/dev/null; then
        log "Stopping rucksfs (PID=$RUCKSFS_PID)"
        kill "$RUCKSFS_PID" 2>/dev/null || true
        wait "$RUCKSFS_PID" 2>/dev/null || true
    fi

    # Remove directories unless --keep
    if [[ "$KEEP_DATA" != true ]]; then
        rm -rf "$MOUNTPOINT" "$DATA_DIR" 2>/dev/null || true
        log "Removed mnt/ and data/"
    else
        log "Kept mnt/ and data/ (--keep)"
    fi
}

trap cleanup EXIT

# ==========================================================================
# Phase 1: Build
# ==========================================================================

log "━━━ Phase 1: Build ━━━"

if [[ "$SKIP_BUILD" == true ]]; then
    log "Skipping build (--skip-build)"
else
    BUILD_FLAGS=("-p" "rucksfs")
    if [[ "$BUILD_PROFILE" == "release" ]]; then
        BUILD_FLAGS+=("--release")
    fi
    log "Building: cargo build ${BUILD_FLAGS[*]}"
    if ! cargo build "${BUILD_FLAGS[@]}" --manifest-path "$PROJECT_ROOT/Cargo.toml" 2>&1; then
        err "Build failed"
        exit 2
    fi
fi

BINARY="$PROJECT_ROOT/target/$BUILD_PROFILE/rucksfs"
if [[ ! -x "$BINARY" ]]; then
    err "Binary not found: $BINARY"
    exit 2
fi
ok "Binary: $BINARY"

# ==========================================================================
# Phase 2: Mount
# ==========================================================================

log "━━━ Phase 2: Mount ━━━"

# Check FUSE availability
if [[ ! -e /dev/fuse ]]; then
    err "/dev/fuse not found. FUSE support required."
    exit 2
fi

# Prepare directories
mkdir -p "$MOUNTPOINT" "$DATA_DIR"

# Start rucksfs in background
log "Starting: rucksfs --mount $MOUNTPOINT --data-dir $DATA_DIR --max-file-size 4194304"
"$BINARY" --mount "$MOUNTPOINT" --data-dir "$DATA_DIR" --max-file-size 4194304 &
RUCKSFS_PID=$!
sleep 3

# Verify mount
if ! kill -0 "$RUCKSFS_PID" 2>/dev/null; then
    err "rucksfs exited unexpectedly (PID=$RUCKSFS_PID)"
    exit 2
fi

if mountpoint -q "$MOUNTPOINT" 2>/dev/null; then
    ok "Mounted at $MOUNTPOINT (PID=$RUCKSFS_PID)"
else
    warn "mountpoint check returned non-zero (may still work)"
fi

# ==========================================================================
# Phase 3: Benchmark
# ==========================================================================

log "━━━ Phase 3: Benchmark ━━━"

RUN_ALL_ARGS=(
    --mountpoint "$MOUNTPOINT"
    --num-files "$NUM_FILES"
    --num-dirs "$NUM_DIRS"
    --max-threads "$MAX_THREADS"
)

if [[ "$SKIP_PJDFSTEST" == true ]]; then
    RUN_ALL_ARGS+=(--skip-pjdfstest)
fi

RUN_ALL_ARGS+=("${BENCH_ARGS[@]}")

log "Executing: run_all.sh ${RUN_ALL_ARGS[*]}"
BENCH_EXIT=0
bash "$SCRIPT_DIR/run_all.sh" "${RUN_ALL_ARGS[@]}" || BENCH_EXIT=$?

# ==========================================================================
# Phase 4: Results Summary
# ==========================================================================

log "━━━ Phase 4: Results ━━━"

if [[ -d "$SCRIPT_DIR/results" ]]; then
    CSV_COUNT=$(find "$SCRIPT_DIR/results" -name "*.csv" -newer "$SCRIPT_DIR/run_all.sh" 2>/dev/null | wc -l | tr -d ' ')
    LOG_COUNT=$(find "$SCRIPT_DIR/results" -name "*.log" -newer "$SCRIPT_DIR/run_all.sh" 2>/dev/null | wc -l | tr -d ' ')
    log "Result files: ${CSV_COUNT} CSV, ${LOG_COUNT} log"
    echo ""
    echo "CSV files:"
    find "$SCRIPT_DIR/results" -name "*.csv" -newer "$SCRIPT_DIR/run_all.sh" -exec ls -lh {} \; 2>/dev/null
fi

echo ""
if [[ $BENCH_EXIT -eq 0 ]]; then
    ok "All benchmarks passed"
else
    err "Some benchmarks failed (exit=$BENCH_EXIT)"
fi

# Cleanup is handled by trap
exit $BENCH_EXIT
