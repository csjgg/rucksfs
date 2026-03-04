#!/usr/bin/env bash
# =============================================================================
# benchmark/run_all.sh — Unified entry point for all RucksFS benchmarks
#
# Usage:
#   ./benchmark/run_all.sh --mountpoint <path> [options]
#
# Options:
#   --mountpoint <path>    Mounted filesystem to test (required)
#   --correctness-only     Run only correctness tests
#   --performance-only     Run only performance benchmarks
#   --num-files N          Files per benchmark (default: 10000)
#   --num-dirs M           Directories for multi-dir tests (default: 100)
#   --max-threads T        Max thread count for concurrency (default: 16)
#   --skip-pjdfstest       Skip pjdfstest (requires separate installation)
# =============================================================================

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MOUNTPOINT=""
CORRECTNESS_ONLY=false
PERFORMANCE_ONLY=false
NUM_FILES=10000
NUM_DIRS=100
MAX_THREADS=16
SKIP_PJDFSTEST=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --mountpoint)        MOUNTPOINT="$2"; shift 2 ;;
        --correctness-only)  CORRECTNESS_ONLY=true; shift ;;
        --performance-only)  PERFORMANCE_ONLY=true; shift ;;
        --num-files)         NUM_FILES="$2"; shift 2 ;;
        --num-dirs)          NUM_DIRS="$2"; shift 2 ;;
        --max-threads)       MAX_THREADS="$2"; shift 2 ;;
        --skip-pjdfstest)    SKIP_PJDFSTEST=true; shift ;;
        -h|--help)
            echo "Usage: $0 --mountpoint <path> [options]"
            echo ""
            echo "Options:"
            echo "  --mountpoint <path>    Mounted filesystem to test (required)"
            echo "  --correctness-only     Run only correctness tests"
            echo "  --performance-only     Run only performance benchmarks"
            echo "  --num-files N          Files per benchmark (default: 10000)"
            echo "  --num-dirs M           Directories for multi-dir (default: 100)"
            echo "  --max-threads T        Max concurrency threads (default: 16)"
            echo "  --skip-pjdfstest       Skip pjdfstest suite"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ -z "$MOUNTPOINT" ]]; then
    echo "ERROR: --mountpoint is required"
    exit 1
fi

if [[ ! -d "$MOUNTPOINT" ]]; then
    echo "ERROR: Mountpoint does not exist: $MOUNTPOINT"
    exit 1
fi

# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'
OVERALL_PASS=0
OVERALL_FAIL=0

run_suite() {
    local name="$1" script="$2"
    shift 2
    echo ""
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}  Running: $name${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""

    if [[ ! -x "$script" ]]; then
        chmod +x "$script" 2>/dev/null || true
    fi

    if bash "$script" "$@"; then
        echo -e "  ${GREEN}✓ $name: PASSED${NC}"
        OVERALL_PASS=$((OVERALL_PASS + 1))
    else
        echo -e "  ${RED}✗ $name: FAILED${NC}"
        OVERALL_FAIL=$((OVERALL_FAIL + 1))
    fi
}

# ---------------------------------------------------------------------------

echo "╔══════════════════════════════════════════════════════╗"
echo "║       RucksFS — Complete Benchmark Suite             ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
echo "  Mountpoint:  $MOUNTPOINT"
echo "  Files:       $NUM_FILES"
echo "  Dirs:        $NUM_DIRS"
echo "  Threads:     $MAX_THREADS"
echo ""

START_TIME=$(date +%s)

# =========================================================================
# Correctness Tests
# =========================================================================
if [[ "$PERFORMANCE_ONLY" != true ]]; then
    echo ""
    echo "┌──────────────────────────────────────────────────┐"
    echo "│              CORRECTNESS TESTS                   │"
    echo "└──────────────────────────────────────────────────┘"

    run_suite "POSIX Conformance" \
        "$SCRIPT_DIR/correctness/posix_conformance.sh" \
        --mountpoint "$MOUNTPOINT"

    if [[ "$SKIP_PJDFSTEST" != true ]]; then
        run_suite "pjdfstest (POSIX compliance)" \
            "$SCRIPT_DIR/correctness/run_pjdfstest.sh" \
            --mountpoint "$MOUNTPOINT"
    else
        echo -e "  ${YELLOW}⊘ Skipping pjdfstest (--skip-pjdfstest)${NC}"
    fi
fi

# =========================================================================
# Performance Benchmarks
# =========================================================================
if [[ "$CORRECTNESS_ONLY" != true ]]; then
    echo ""
    echo "┌──────────────────────────────────────────────────┐"
    echo "│            PERFORMANCE BENCHMARKS                │"
    echo "└──────────────────────────────────────────────────┘"

    run_suite "Metadata Operations" \
        "$SCRIPT_DIR/performance/metadata_ops.sh" \
        --mountpoint "$MOUNTPOINT" \
        --num-files "$NUM_FILES" \
        --num-dirs "$NUM_DIRS"

    run_suite "I/O Throughput" \
        "$SCRIPT_DIR/performance/io_throughput.sh" \
        --mountpoint "$MOUNTPOINT"

    run_suite "Concurrent Stress" \
        "$SCRIPT_DIR/performance/concurrent_stress.sh" \
        --mountpoint "$MOUNTPOINT" \
        --num-files "$NUM_FILES" \
        --max-threads "$MAX_THREADS"
fi

# =========================================================================
# Overall Summary
# =========================================================================
END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))

echo ""
echo "╔══════════════════════════════════════════════════════╗"
echo "║                 OVERALL SUMMARY                     ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
TOTAL=$((OVERALL_PASS + OVERALL_FAIL))
echo -e "  Suites: ${GREEN}$OVERALL_PASS passed${NC}, ${RED}$OVERALL_FAIL failed${NC}, $TOTAL total"
echo "  Time:   ${ELAPSED}s"
echo ""
echo "  Results directory: $SCRIPT_DIR/results/"
echo ""

if [[ -d "$SCRIPT_DIR/results" ]]; then
    echo "  Generated files:"
    ls -la "$SCRIPT_DIR/results/"*.csv "$SCRIPT_DIR/results/"*.log 2>/dev/null | while read -r line; do
        echo "    $line"
    done
fi

echo ""

if [[ $OVERALL_FAIL -gt 0 ]]; then
    exit 1
fi
exit 0
