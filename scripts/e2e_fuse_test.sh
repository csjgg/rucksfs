#!/usr/bin/env bash
# =============================================================================
# scripts/e2e_fuse_test.sh — Deploy RucksFS FUSE mount and run E2E tests
#
# Usage:
#   ./scripts/e2e_fuse_test.sh [--data-dir <dir>] [--mountpoint <path>]
#
# Requirements (Linux only):
#   - FUSE support (fuse3 or fuse)
#   - Built rucksfs binary
#
# What it does:
#   1. Builds the project
#   2. Mounts RucksFS via FUSE
#   3. Runs a comprehensive set of file-system operations
#   4. Runs concurrent stress tests via shell
#   5. Unmounts and reports results
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MOUNTPOINT="/tmp/rucksfs_e2e"
DATA_DIR=""
DEMO_BIN=""
DEMO_PID=""
PASSED=0
FAILED=0

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

while [[ $# -gt 0 ]]; do
    case "$1" in
        --data-dir)
            DATA_DIR="$2"
            shift 2
            ;;
        --mountpoint)
            MOUNTPOINT="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [--data-dir <dir>] [--mountpoint <path>]"
            echo ""
            echo "Options:"
            echo "  --data-dir <dir>     Data directory for RocksDB storage (default: temp dir)"
            echo "  --mountpoint <path>  FUSE mount point (default: /tmp/rucksfs_e2e)"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

pass() {
    PASSED=$((PASSED + 1))
    echo -e "  ${GREEN}✓ PASS${NC}: $1"
}

fail() {
    FAILED=$((FAILED + 1))
    echo -e "  ${RED}✗ FAIL${NC}: $1"
}

assert_eq() {
    local desc="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then
        pass "$desc"
    else
        fail "$desc (expected='$expected', got='$actual')"
    fi
}

assert_file_content() {
    local desc="$1" file="$2" expected="$3"
    if [[ -f "$file" ]]; then
        local actual
        actual=$(cat "$file")
        if [[ "$actual" == "$expected" ]]; then
            pass "$desc"
        else
            fail "$desc (expected='$expected', got='$actual')"
        fi
    else
        fail "$desc (file does not exist: $file)"
    fi
}

assert_exists() {
    local desc="$1" path="$2"
    if [[ -e "$path" ]]; then
        pass "$desc"
    else
        fail "$desc (path does not exist: $path)"
    fi
}

assert_not_exists() {
    local desc="$1" path="$2"
    if [[ ! -e "$path" ]]; then
        pass "$desc"
    else
        fail "$desc (path should not exist: $path)"
    fi
}

assert_is_dir() {
    local desc="$1" path="$2"
    if [[ -d "$path" ]]; then
        pass "$desc"
    else
        fail "$desc (not a directory: $path)"
    fi
}

# ---------------------------------------------------------------------------
# Cleanup on exit
# ---------------------------------------------------------------------------

cleanup() {
    echo ""
    echo "── Cleaning up ──"
    if [[ -n "$DEMO_PID" ]] && kill -0 "$DEMO_PID" 2>/dev/null; then
        echo "Unmounting $MOUNTPOINT ..."
        fusermount -u "$MOUNTPOINT" 2>/dev/null || fusermount3 -u "$MOUNTPOINT" 2>/dev/null || true
        sleep 1
        if kill -0 "$DEMO_PID" 2>/dev/null; then
            kill "$DEMO_PID" 2>/dev/null || true
        fi
    fi
    if [[ -d "$MOUNTPOINT" ]]; then
        rmdir "$MOUNTPOINT" 2>/dev/null || true
    fi
}

trap cleanup EXIT

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

echo "╔══════════════════════════════════════════════════════╗"
echo "║       RucksFS — E2E FUSE Test Suite                 ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""

echo "── Building rucksfs ──"
cd "$PROJECT_ROOT"
cargo build -p rucksfs 2>&1
DEMO_BIN="$PROJECT_ROOT/target/debug/rucksfs"

if [[ ! -x "$DEMO_BIN" ]]; then
    echo -e "${RED}ERROR: Demo binary not found at $DEMO_BIN${NC}"
    exit 1
fi
echo -e "${GREEN}Build OK${NC}"
echo ""

# ---------------------------------------------------------------------------
# Mount FUSE
# ---------------------------------------------------------------------------

echo "── Mounting FUSE at $MOUNTPOINT ──"
mkdir -p "$MOUNTPOINT"

MOUNT_ARGS=(--mount "$MOUNTPOINT")
if [[ -n "$DATA_DIR" ]]; then
    mkdir -p "$DATA_DIR"
    MOUNT_ARGS+=(--data-dir "$DATA_DIR")
    echo "  Using persistent storage at: $DATA_DIR"
fi

"$DEMO_BIN" "${MOUNT_ARGS[@]}" &
DEMO_PID=$!
sleep 2

if ! kill -0 "$DEMO_PID" 2>/dev/null; then
    echo -e "${RED}ERROR: rucksfs process exited unexpectedly${NC}"
    exit 1
fi

if ! mountpoint -q "$MOUNTPOINT" 2>/dev/null; then
    echo -e "${YELLOW}WARNING: mountpoint check failed (may still work)${NC}"
fi
echo -e "${GREEN}FUSE mounted (PID=$DEMO_PID)${NC}"
echo ""

# ===========================================================================
# Test Suite 1: Basic Operations
# ===========================================================================

echo "══ Test Suite 1: Basic File Operations ══"

# mkdir
mkdir "$MOUNTPOINT/testdir"
assert_is_dir "mkdir creates directory" "$MOUNTPOINT/testdir"

# create file + write + read
echo "hello rucksfs" > "$MOUNTPOINT/testdir/hello.txt"
assert_exists "create file" "$MOUNTPOINT/testdir/hello.txt"
assert_file_content "write and read" "$MOUNTPOINT/testdir/hello.txt" "hello rucksfs"

# ls / readdir
count=$(ls "$MOUNTPOINT/testdir" | wc -l | tr -d ' ')
assert_eq "readdir count" "1" "$count"

# rename / mv
mv "$MOUNTPOINT/testdir/hello.txt" "$MOUNTPOINT/testdir/greeting.txt"
assert_not_exists "rename removes old name" "$MOUNTPOINT/testdir/hello.txt"
assert_exists "rename creates new name" "$MOUNTPOINT/testdir/greeting.txt"
assert_file_content "rename preserves content" "$MOUNTPOINT/testdir/greeting.txt" "hello rucksfs"

# unlink / rm
rm "$MOUNTPOINT/testdir/greeting.txt"
assert_not_exists "unlink removes file" "$MOUNTPOINT/testdir/greeting.txt"

# rmdir
rmdir "$MOUNTPOINT/testdir"
assert_not_exists "rmdir removes directory" "$MOUNTPOINT/testdir"

echo ""

# ===========================================================================
# Test Suite 2: Write Patterns
# ===========================================================================

echo "══ Test Suite 2: Write Patterns ══"

# Large file write
dd if=/dev/urandom of="$MOUNTPOINT/large.bin" bs=4096 count=256 2>/dev/null
actual_size=$(stat -c%s "$MOUNTPOINT/large.bin" 2>/dev/null || stat -f%z "$MOUNTPOINT/large.bin" 2>/dev/null)
assert_eq "large file size" "1048576" "$actual_size"

# Data integrity via checksum
cp "$MOUNTPOINT/large.bin" "/tmp/rucksfs_verify.bin"
orig_md5=$(md5sum "$MOUNTPOINT/large.bin" 2>/dev/null | cut -d' ' -f1 || md5 -q "$MOUNTPOINT/large.bin" 2>/dev/null)
copy_md5=$(md5sum "/tmp/rucksfs_verify.bin" 2>/dev/null | cut -d' ' -f1 || md5 -q "/tmp/rucksfs_verify.bin" 2>/dev/null)
assert_eq "data integrity (md5)" "$orig_md5" "$copy_md5"
rm -f "/tmp/rucksfs_verify.bin" "$MOUNTPOINT/large.bin"

# Append writes
echo "line1" > "$MOUNTPOINT/append.txt"
echo "line2" >> "$MOUNTPOINT/append.txt"
echo "line3" >> "$MOUNTPOINT/append.txt"
expected=$(printf "line1\nline2\nline3")
actual=$(cat "$MOUNTPOINT/append.txt")
assert_eq "append writes" "$expected" "$actual"
rm -f "$MOUNTPOINT/append.txt"

echo ""

# ===========================================================================
# Test Suite 3: Deep Directory Tree
# ===========================================================================

echo "══ Test Suite 3: Deep Directory Tree ══"

mkdir -p "$MOUNTPOINT/a/b/c/d/e"
assert_is_dir "deep mkdir -p" "$MOUNTPOINT/a/b/c/d/e"

echo "deep content" > "$MOUNTPOINT/a/b/c/d/e/deep.txt"
assert_file_content "deep file read" "$MOUNTPOINT/a/b/c/d/e/deep.txt" "deep content"

rm "$MOUNTPOINT/a/b/c/d/e/deep.txt"
rmdir "$MOUNTPOINT/a/b/c/d/e"
rmdir "$MOUNTPOINT/a/b/c/d"
rmdir "$MOUNTPOINT/a/b/c"
rmdir "$MOUNTPOINT/a/b"
rmdir "$MOUNTPOINT/a"
assert_not_exists "deep tree cleanup" "$MOUNTPOINT/a"

echo ""

# ===========================================================================
# Test Suite 4: Concurrent Stress Tests
# ===========================================================================

echo "══ Test Suite 4: Concurrent Stress Tests ══"

# 4a. Concurrent file creation
mkdir "$MOUNTPOINT/concurrent_create"
for i in $(seq 1 100); do
    echo "data$i" > "$MOUNTPOINT/concurrent_create/file_$i" &
done
wait

count=$(ls "$MOUNTPOINT/concurrent_create" | wc -l | tr -d ' ')
assert_eq "concurrent create (100 files)" "100" "$count"

# Verify all files are readable
all_ok=true
for i in $(seq 1 100); do
    content=$(cat "$MOUNTPOINT/concurrent_create/file_$i" 2>/dev/null || echo "MISSING")
    if [[ "$content" != "data$i" ]]; then
        all_ok=false
        break
    fi
done
if $all_ok; then
    pass "concurrent create data integrity"
else
    fail "concurrent create data integrity"
fi
rm -rf "$MOUNTPOINT/concurrent_create"

# 4b. Concurrent writes to separate files
mkdir "$MOUNTPOINT/concurrent_write"
for i in $(seq 1 50); do
    (
        echo "initial_$i" > "$MOUNTPOINT/concurrent_write/file_$i"
        echo "append_$i" >> "$MOUNTPOINT/concurrent_write/file_$i"
    ) &
done
wait

write_ok=true
for i in $(seq 1 50); do
    expected=$(printf "initial_$i\nappend_$i")
    actual=$(cat "$MOUNTPOINT/concurrent_write/file_$i" 2>/dev/null || echo "MISSING")
    if [[ "$actual" != "$expected" ]]; then
        write_ok=false
        break
    fi
done
if $write_ok; then
    pass "concurrent write data integrity"
else
    fail "concurrent write data integrity"
fi
rm -rf "$MOUNTPOINT/concurrent_write"

# 4c. Concurrent mkdir
for i in $(seq 1 50); do
    mkdir "$MOUNTPOINT/cdir_$i" &
done
wait

dir_count=$(ls -d "$MOUNTPOINT"/cdir_* 2>/dev/null | wc -l | tr -d ' ')
assert_eq "concurrent mkdir (50 dirs)" "50" "$dir_count"
for i in $(seq 1 50); do
    rmdir "$MOUNTPOINT/cdir_$i" 2>/dev/null
done

# 4d. Create-then-delete storm
mkdir "$MOUNTPOINT/storm"
for i in $(seq 1 100); do
    (
        echo "storm_$i" > "$MOUNTPOINT/storm/file_$i"
        rm "$MOUNTPOINT/storm/file_$i"
    ) &
done
wait

remaining=$(ls "$MOUNTPOINT/storm" 2>/dev/null | wc -l | tr -d ' ')
assert_eq "create-delete storm (0 remaining)" "0" "$remaining"
rmdir "$MOUNTPOINT/storm"

echo ""

# ===========================================================================
# Test Suite 5: Metadata Consistency
# ===========================================================================

echo "══ Test Suite 5: Metadata Consistency ══"

# stat after write should reflect size
echo -n "exactly_16_bytes" > "$MOUNTPOINT/sized.txt"
size=$(stat -c%s "$MOUNTPOINT/sized.txt" 2>/dev/null || stat -f%z "$MOUNTPOINT/sized.txt" 2>/dev/null)
assert_eq "stat size after write" "16" "$size"
rm "$MOUNTPOINT/sized.txt"

# chmod (if supported)
echo "test" > "$MOUNTPOINT/chmod_test.txt"
chmod 755 "$MOUNTPOINT/chmod_test.txt" 2>/dev/null
mode=$(stat -c%a "$MOUNTPOINT/chmod_test.txt" 2>/dev/null || stat -f%Lp "$MOUNTPOINT/chmod_test.txt" 2>/dev/null)
if [[ "$mode" == "755" ]]; then
    pass "chmod changes mode"
else
    echo -e "  ${YELLOW}⚠ SKIP${NC}: chmod (got mode=$mode)"
fi
rm "$MOUNTPOINT/chmod_test.txt"

echo ""

# ===========================================================================
# Summary
# ===========================================================================

echo "══════════════════════════════════════════════════"
TOTAL=$((PASSED + FAILED))
echo -e "Results: ${GREEN}$PASSED passed${NC}, ${RED}$FAILED failed${NC}, $TOTAL total"
echo "══════════════════════════════════════════════════"

if [[ $FAILED -gt 0 ]]; then
    exit 1
fi
exit 0
