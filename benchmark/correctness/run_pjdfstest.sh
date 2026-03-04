#!/usr/bin/env bash
# =============================================================================
# benchmark/correctness/run_pjdfstest.sh — Run pjdfstest POSIX compliance suite
#
# Usage:
#   ./benchmark/correctness/run_pjdfstest.sh --mountpoint <path> [--pjdfstest-bin <path>]
#
# Prerequisites:
#   - pjdfstest installed (Rust version recommended)
#     git clone https://github.com/saidsay-so/pjdfstest
#     cd pjdfstest && cargo build --release
#   - Root access recommended for full coverage
#   - FUSE filesystem mounted at <mountpoint>
#
# Reference:
#   pjdfstest provides 8,800+ POSIX compliance tests covering:
#   chmod, chown, link, mkdir, mkfifo, open, rename, rmdir, symlink,
#   truncate, unlink, utimensat, and more.
#
# Output:
#   - benchmark/results/pjdfstest_<timestamp>.log  (full output)
#   - benchmark/results/pjdfstest_<timestamp>.csv  (summary)
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$PROJECT_ROOT/benchmark/results"
MOUNTPOINT=""
PJDFSTEST_BIN=""
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

while [[ $# -gt 0 ]]; do
    case "$1" in
        --mountpoint)
            MOUNTPOINT="$2"
            shift 2
            ;;
        --pjdfstest-bin)
            PJDFSTEST_BIN="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 --mountpoint <path> [--pjdfstest-bin <path>]"
            echo ""
            echo "Options:"
            echo "  --mountpoint <path>      Mounted filesystem to test (required)"
            echo "  --pjdfstest-bin <path>   Path to pjdfstest binary"
            echo "                           (default: auto-detect from PATH or ./pjdfstest)"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ -z "$MOUNTPOINT" ]]; then
    echo "ERROR: --mountpoint is required"
    echo "Usage: $0 --mountpoint <path>"
    exit 1
fi

if [[ ! -d "$MOUNTPOINT" ]]; then
    echo "ERROR: Mountpoint does not exist: $MOUNTPOINT"
    exit 1
fi

# ---------------------------------------------------------------------------
# Locate pjdfstest binary
# ---------------------------------------------------------------------------

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

find_pjdfstest() {
    # 1. User-specified path
    if [[ -n "$PJDFSTEST_BIN" ]]; then
        if [[ -x "$PJDFSTEST_BIN" ]]; then
            echo "$PJDFSTEST_BIN"
            return 0
        else
            echo ""
            return 1
        fi
    fi

    # 2. In PATH
    if command -v pjdfstest &>/dev/null; then
        command -v pjdfstest
        return 0
    fi

    # 3. Common build locations
    local candidates=(
        "$PROJECT_ROOT/pjdfstest/target/release/pjdfstest"
        "$PROJECT_ROOT/pjdfstest/target/debug/pjdfstest"
        "$HOME/.cargo/bin/pjdfstest"
        "/usr/local/bin/pjdfstest"
    )
    for c in "${candidates[@]}"; do
        if [[ -x "$c" ]]; then
            echo "$c"
            return 0
        fi
    done

    echo ""
    return 1
}

PJDFSTEST_BIN=$(find_pjdfstest) || true

if [[ -z "$PJDFSTEST_BIN" ]]; then
    echo -e "${YELLOW}WARNING: pjdfstest not found.${NC}"
    echo ""
    echo "To install pjdfstest (Rust version):"
    echo "  git clone https://github.com/saidsay-so/pjdfstest"
    echo "  cd pjdfstest && cargo build --release"
    echo "  # Binary will be at target/release/pjdfstest"
    echo ""
    echo "Then re-run:"
    echo "  $0 --mountpoint $MOUNTPOINT --pjdfstest-bin /path/to/pjdfstest"
    exit 1
fi

echo -e "${GREEN}Found pjdfstest: $PJDFSTEST_BIN${NC}"

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------

mkdir -p "$RESULTS_DIR"
LOG_FILE="$RESULTS_DIR/pjdfstest_${TIMESTAMP}.log"
CSV_FILE="$RESULTS_DIR/pjdfstest_${TIMESTAMP}.csv"
TEST_DIR="$MOUNTPOINT/.pjdfstest_$$"

cleanup() {
    rm -rf "$TEST_DIR" 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p "$TEST_DIR"

# ---------------------------------------------------------------------------
# Run pjdfstest
# ---------------------------------------------------------------------------

echo "╔══════════════════════════════════════════════════════╗"
echo "║       pjdfstest — POSIX Compliance Test Suite       ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
echo "  Mountpoint:  $MOUNTPOINT"
echo "  Test dir:    $TEST_DIR"
echo "  Log file:    $LOG_FILE"
echo ""

# Detect if we're running the Rust or legacy version
PJDFSTEST_VERSION="unknown"
if "$PJDFSTEST_BIN" --version 2>/dev/null | grep -q "pjdfstest"; then
    PJDFSTEST_VERSION=$("$PJDFSTEST_BIN" --version 2>/dev/null || echo "unknown")
fi

echo "  pjdfstest:   $PJDFSTEST_VERSION"
echo ""

# Create a minimal config for the Rust version
CONFIG_FILE="$TEST_DIR/pjdfstest.toml"
cat > "$CONFIG_FILE" <<'EOF'
# pjdfstest configuration for RucksFS
[settings]
# Reduce sleep time for faster tests
naptime = 0.001

[features]
# Enable features that RucksFS supports
# Add more features as they are implemented
EOF

echo "── Running pjdfstest ──"
echo ""

PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0
TOTAL_COUNT=0

# Try Rust-version invocation first, fall back to legacy
if "$PJDFSTEST_BIN" --help 2>&1 | grep -q "\-\-path"; then
    # Rust version
    echo "Using Rust pjdfstest"
    "$PJDFSTEST_BIN" -c "$CONFIG_FILE" --path "$TEST_DIR" 2>&1 | tee "$LOG_FILE" || true
else
    # Legacy version — run with prove if available
    PJDFSTEST_DIR="$(dirname "$PJDFSTEST_BIN")"
    TESTS_DIR=""
    for candidate in "$PJDFSTEST_DIR/tests" "$PJDFSTEST_DIR/../tests"; do
        if [[ -d "$candidate" ]]; then
            TESTS_DIR="$candidate"
            break
        fi
    done

    if [[ -n "$TESTS_DIR" ]] && command -v prove &>/dev/null; then
        echo "Using legacy pjdfstest with prove"
        (cd "$TEST_DIR" && prove -rv "$TESTS_DIR" 2>&1) | tee "$LOG_FILE" || true
    else
        echo "Running pjdfstest directly"
        (cd "$TEST_DIR" && "$PJDFSTEST_BIN" 2>&1) | tee "$LOG_FILE" || true
    fi
fi

# ---------------------------------------------------------------------------
# Parse results
# ---------------------------------------------------------------------------

if [[ -f "$LOG_FILE" ]]; then
    PASS_COUNT=$(grep -c -E '(^ok |PASS|passed)' "$LOG_FILE" 2>/dev/null || echo "0")
    FAIL_COUNT=$(grep -c -E '(^not ok |FAIL|failed)' "$LOG_FILE" 2>/dev/null || echo "0")
    SKIP_COUNT=$(grep -c -E '(^ok.*# skip|SKIP|skipped)' "$LOG_FILE" 2>/dev/null || echo "0")
    TOTAL_COUNT=$((PASS_COUNT + FAIL_COUNT + SKIP_COUNT))
fi

# Write CSV summary
echo "timestamp,test_suite,passed,failed,skipped,total" > "$CSV_FILE"
echo "${TIMESTAMP},pjdfstest,${PASS_COUNT},${FAIL_COUNT},${SKIP_COUNT},${TOTAL_COUNT}" >> "$CSV_FILE"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

echo ""
echo "══════════════════════════════════════════════════"
echo -e "pjdfstest Results: ${GREEN}${PASS_COUNT} passed${NC}, ${RED}${FAIL_COUNT} failed${NC}, ${YELLOW}${SKIP_COUNT} skipped${NC}, ${TOTAL_COUNT} total"
echo "══════════════════════════════════════════════════"
echo ""
echo "  Full log:    $LOG_FILE"
echo "  CSV summary: $CSV_FILE"

if [[ "$FAIL_COUNT" -gt 0 ]]; then
    exit 1
fi
exit 0
