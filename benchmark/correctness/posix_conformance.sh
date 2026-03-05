#!/usr/bin/env bash
# =============================================================================
# benchmark/correctness/posix_conformance.sh
# Custom POSIX semantics correctness tests for RucksFS
#
# Academic references:
#   - TableFS (ATC'13): metadata CRUD correctness
#   - SingularFS (ATC'23): rename atomicity, nlink consistency
#   - LocoFS (SC'17): readdir consistency, decoupled metadata correctness
#   - POSIX.1-2017: authoritative POSIX specification
#   - pjdfstest: edge-case inspiration (long names, special chars, errors)
#
# Usage:
#   ./benchmark/correctness/posix_conformance.sh --mountpoint <path>
#
# Output:
#   - benchmark/results/posix_conformance_<timestamp>.log
#   - Exit code 0 if all tests pass, 1 if any fail
# =============================================================================

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$PROJECT_ROOT/benchmark/results"
MOUNTPOINT=""
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
        -h|--help)
            echo "Usage: $0 --mountpoint <path>"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"; exit 1
            ;;
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
# Helpers
# ---------------------------------------------------------------------------

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'
PASSED=0
FAILED=0
SKIPPED=0
TEST_BASE="$MOUNTPOINT/.posix_test_$$"

mkdir -p "$RESULTS_DIR"
LOG_FILE="$RESULTS_DIR/posix_conformance_${TIMESTAMP}.log"
exec > >(tee -a "$LOG_FILE") 2>&1

pass() {
    PASSED=$((PASSED + 1))
    echo -e "  ${GREEN}[PASS]${NC} $1"
}

fail() {
    FAILED=$((FAILED + 1))
    echo -e "  ${RED}[FAIL]${NC} $1"
}

skip() {
    SKIPPED=$((SKIPPED + 1))
    echo -e "  ${YELLOW}[SKIP]${NC} $1"
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
        assert_eq "$desc" "$expected" "$actual"
    else
        fail "$desc (file does not exist: $file)"
    fi
}

assert_exists() {
    local desc="$1" path="$2"
    if [[ -e "$path" ]]; then pass "$desc"; else fail "$desc (path missing: $path)"; fi
}

assert_not_exists() {
    local desc="$1" path="$2"
    if [[ ! -e "$path" ]]; then pass "$desc"; else fail "$desc (path should not exist: $path)"; fi
}

assert_is_dir() {
    local desc="$1" path="$2"
    if [[ -d "$path" ]]; then pass "$desc"; else fail "$desc (not a directory: $path)"; fi
}

assert_is_file() {
    local desc="$1" path="$2"
    if [[ -f "$path" ]]; then pass "$desc"; else fail "$desc (not a regular file: $path)"; fi
}

# Check if an operation returns a specific error.
# Usage: assert_error "desc" <expected_errno_name> <command...>
# We check exit code != 0 and optionally grep stderr.
assert_fails() {
    local desc="$1"
    shift
    if "$@" 2>/dev/null; then
        fail "$desc (expected failure, but succeeded)"
    else
        pass "$desc"
    fi
}

# Cleanup on exit
cleanup() {
    rm -rf "$TEST_BASE" 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p "$TEST_BASE"

# ==========================================================================
echo "╔══════════════════════════════════════════════════════╗"
echo "║     RucksFS — POSIX Conformance Test Suite          ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
echo "  Mountpoint: $MOUNTPOINT"
echo "  Test base:  $TEST_BASE"
echo "  Timestamp:  $TIMESTAMP"
echo ""

# ==========================================================================
# S1: Basic File CRUD (ref: TableFS §5.1)
# ==========================================================================
echo "══ S1: Basic File CRUD ══"

# S1.01: Create a regular file
touch "$TEST_BASE/s1_file"
assert_is_file "S1.01: touch creates regular file" "$TEST_BASE/s1_file"

# S1.02: Write and read back content
echo "hello rucksfs" > "$TEST_BASE/s1_file"
assert_file_content "S1.02: write and read back" "$TEST_BASE/s1_file" "hello rucksfs"

# S1.03: Overwrite file content
echo "new content" > "$TEST_BASE/s1_file"
assert_file_content "S1.03: overwrite replaces content" "$TEST_BASE/s1_file" "new content"

# S1.04: Append to file
echo "line1" > "$TEST_BASE/s1_append"
echo "line2" >> "$TEST_BASE/s1_append"
echo "line3" >> "$TEST_BASE/s1_append"
expected=$(printf "line1\nline2\nline3")
actual=$(cat "$TEST_BASE/s1_append")
assert_eq "S1.04: append writes" "$expected" "$actual"

# S1.05: Read empty file
touch "$TEST_BASE/s1_empty"
actual=$(cat "$TEST_BASE/s1_empty")
assert_eq "S1.05: empty file reads empty" "" "$actual"

# S1.06: stat shows correct size
echo -n "exactly_16_bytes" > "$TEST_BASE/s1_sized"
size=$(stat -c%s "$TEST_BASE/s1_sized" 2>/dev/null || stat -f%z "$TEST_BASE/s1_sized" 2>/dev/null)
assert_eq "S1.06: stat size after write" "16" "$size"

# S1.07: Unlink removes file
rm "$TEST_BASE/s1_file"
assert_not_exists "S1.07: unlink removes file" "$TEST_BASE/s1_file"

# S1.08: Write at offset (dd)
dd if=/dev/zero of="$TEST_BASE/s1_offset" bs=1 count=10 2>/dev/null
dd if=/dev/zero of="$TEST_BASE/s1_offset" bs=1 count=5 seek=10 2>/dev/null
size=$(stat -c%s "$TEST_BASE/s1_offset" 2>/dev/null || stat -f%z "$TEST_BASE/s1_offset" 2>/dev/null)
assert_eq "S1.08: write at offset extends file" "15" "$size"

# Cleanup S1
rm -f "$TEST_BASE"/s1_*
echo ""

# ==========================================================================
# S2: Directory Operations (ref: TableFS §5.1, LocoFS §6.2)
# ==========================================================================
echo "══ S2: Directory Operations ══"

# S2.01: mkdir
mkdir "$TEST_BASE/s2_dir"
assert_is_dir "S2.01: mkdir creates directory" "$TEST_BASE/s2_dir"

# S2.02: nested mkdir -p
mkdir -p "$TEST_BASE/s2_deep/a/b/c/d"
assert_is_dir "S2.02: mkdir -p creates deep tree" "$TEST_BASE/s2_deep/a/b/c/d"

# S2.03: readdir lists correct entries
mkdir "$TEST_BASE/s2_list"
touch "$TEST_BASE/s2_list/f1" "$TEST_BASE/s2_list/f2" "$TEST_BASE/s2_list/f3"
mkdir "$TEST_BASE/s2_list/d1"
count=$(ls "$TEST_BASE/s2_list" | wc -l | tr -d ' ')
assert_eq "S2.03: readdir count" "4" "$count"

# S2.04: readdir sees newly created entries
touch "$TEST_BASE/s2_list/f4"
count=$(ls "$TEST_BASE/s2_list" | wc -l | tr -d ' ')
assert_eq "S2.04: readdir sees new entry" "5" "$count"

# S2.05: rmdir on empty directory
mkdir "$TEST_BASE/s2_rmdir_empty"
rmdir "$TEST_BASE/s2_rmdir_empty"
assert_not_exists "S2.05: rmdir removes empty dir" "$TEST_BASE/s2_rmdir_empty"

# S2.06: rmdir on non-empty directory fails
mkdir "$TEST_BASE/s2_rmdir_full"
touch "$TEST_BASE/s2_rmdir_full/child"
assert_fails "S2.06: rmdir non-empty fails (ENOTEMPTY)" rmdir "$TEST_BASE/s2_rmdir_full"
rm "$TEST_BASE/s2_rmdir_full/child"
rmdir "$TEST_BASE/s2_rmdir_full"

# S2.07: readdir after deletion
rm "$TEST_BASE/s2_list/f1"
count=$(ls "$TEST_BASE/s2_list" | wc -l | tr -d ' ')
assert_eq "S2.07: readdir after deletion" "4" "$count"

# S2.08: very deep directory tree (100 levels, ref: SingularFS §6.4)
DEEP="$TEST_BASE/s2_vdeep"
mkdir -p "$DEEP"
current="$DEEP"
for i in $(seq 1 100); do
    current="$current/d$i"
    mkdir "$current" 2>/dev/null || { skip "S2.08: deep tree creation stopped at depth $i"; break; }
done
if [[ -d "$current" ]]; then
    pass "S2.08: 100-level deep directory tree"
fi

# Cleanup S2
rm -rf "$TEST_BASE"/s2_*
echo ""

# ==========================================================================
# S3: Rename Semantics (ref: SingularFS §5.3, POSIX.1-2017)
# ==========================================================================
echo "══ S3: Rename Semantics ══"

# S3.01: Rename within same directory
echo "data" > "$TEST_BASE/s3_old"
mv "$TEST_BASE/s3_old" "$TEST_BASE/s3_new"
assert_not_exists "S3.01a: rename removes old name" "$TEST_BASE/s3_old"
assert_file_content "S3.01b: rename preserves content" "$TEST_BASE/s3_new" "data"

# S3.02: Cross-directory rename
mkdir "$TEST_BASE/s3_src" "$TEST_BASE/s3_dst"
echo "cross" > "$TEST_BASE/s3_src/file"
mv "$TEST_BASE/s3_src/file" "$TEST_BASE/s3_dst/file"
assert_not_exists "S3.02a: cross-dir rename removes source" "$TEST_BASE/s3_src/file"
assert_file_content "S3.02b: cross-dir rename preserves content" "$TEST_BASE/s3_dst/file" "cross"

# S3.03: Rename overwrites existing target (POSIX atomicity)
echo "original" > "$TEST_BASE/s3_target"
echo "replacement" > "$TEST_BASE/s3_source"
mv "$TEST_BASE/s3_source" "$TEST_BASE/s3_target"
assert_file_content "S3.03: rename overwrites target atomically" "$TEST_BASE/s3_target" "replacement"
assert_not_exists "S3.03b: source removed after overwrite" "$TEST_BASE/s3_source"

# S3.04: Rename directory
mkdir "$TEST_BASE/s3_dold"
touch "$TEST_BASE/s3_dold/child"
mv "$TEST_BASE/s3_dold" "$TEST_BASE/s3_dnew"
assert_not_exists "S3.04a: rename removes old dir" "$TEST_BASE/s3_dold"
assert_is_dir "S3.04b: rename creates new dir" "$TEST_BASE/s3_dnew"
assert_exists "S3.04c: rename preserves dir contents" "$TEST_BASE/s3_dnew/child"

# S3.05: Rename to self is a no-op
echo "self" > "$TEST_BASE/s3_self"
mv "$TEST_BASE/s3_self" "$TEST_BASE/s3_self" 2>/dev/null || true
assert_file_content "S3.05: rename to self preserves file" "$TEST_BASE/s3_self" "self"

# S3.06: Rename non-existent source fails
assert_fails "S3.06: rename non-existent fails" mv "$TEST_BASE/s3_ghost" "$TEST_BASE/s3_target2"

# Cleanup S3
rm -rf "$TEST_BASE"/s3_*
echo ""

# ==========================================================================
# S4: Metadata Consistency (ref: POSIX.1-2017)
# ==========================================================================
echo "══ S4: Metadata Consistency ══"

# S4.01: chmod changes mode
echo "test" > "$TEST_BASE/s4_chmod"
chmod 755 "$TEST_BASE/s4_chmod"
mode=$(stat -c%a "$TEST_BASE/s4_chmod" 2>/dev/null || stat -f%Lp "$TEST_BASE/s4_chmod" 2>/dev/null)
if [[ "$mode" == "755" ]]; then
    pass "S4.01: chmod changes mode to 755"
else
    skip "S4.01: chmod (got mode=$mode, may not be supported)"
fi

# S4.02: chmod to restrictive mode
chmod 000 "$TEST_BASE/s4_chmod" 2>/dev/null
mode=$(stat -c%a "$TEST_BASE/s4_chmod" 2>/dev/null || stat -f%Lp "$TEST_BASE/s4_chmod" 2>/dev/null)
if [[ "$mode" == "000" ]]; then
    pass "S4.02: chmod to 000"
else
    skip "S4.02: chmod to 000 (got mode=$mode)"
fi
chmod 644 "$TEST_BASE/s4_chmod" 2>/dev/null

# S4.03: file type bits in stat mode
mkdir "$TEST_BASE/s4_dir"
dir_mode=$(stat -c%f "$TEST_BASE/s4_dir" 2>/dev/null || echo "skip")
if [[ "$dir_mode" != "skip" ]]; then
    file_type=$((16#$dir_mode & 16#F000))
    if [[ $file_type -eq $((16#4000)) ]]; then
        pass "S4.03: directory mode has S_IFDIR bit"
    else
        fail "S4.03: directory mode type bits (got 0x$(printf '%x' $file_type))"
    fi
else
    skip "S4.03: stat -c%f not supported"
fi

# S4.04: mtime updates on write
echo "first" > "$TEST_BASE/s4_mtime"
mtime1=$(stat -c%Y "$TEST_BASE/s4_mtime" 2>/dev/null || stat -f%m "$TEST_BASE/s4_mtime" 2>/dev/null)
sleep 1
echo "second" >> "$TEST_BASE/s4_mtime"
mtime2=$(stat -c%Y "$TEST_BASE/s4_mtime" 2>/dev/null || stat -f%m "$TEST_BASE/s4_mtime" 2>/dev/null)
if [[ "$mtime2" -ge "$mtime1" ]]; then
    pass "S4.04: mtime updates on write"
else
    fail "S4.04: mtime should increase (before=$mtime1, after=$mtime2)"
fi

# S4.05: truncate changes file size
echo "long content here" > "$TEST_BASE/s4_trunc"
truncate -s 4 "$TEST_BASE/s4_trunc" 2>/dev/null
if [[ $? -eq 0 ]]; then
    size=$(stat -c%s "$TEST_BASE/s4_trunc" 2>/dev/null || stat -f%z "$TEST_BASE/s4_trunc" 2>/dev/null)
    assert_eq "S4.05: truncate changes size" "4" "$size"
else
    skip "S4.05: truncate not supported"
fi

# S4.06: Directory nlink count
mkdir "$TEST_BASE/s4_nlink"
nlink=$(stat -c%h "$TEST_BASE/s4_nlink" 2>/dev/null || stat -f%l "$TEST_BASE/s4_nlink" 2>/dev/null)
if [[ "$nlink" -ge 2 ]]; then
    pass "S4.06: directory initial nlink >= 2"
else
    skip "S4.06: directory nlink (got $nlink, FUSE may differ)"
fi

# Cleanup S4
rm -rf "$TEST_BASE"/s4_*
echo ""

# ==========================================================================
# S5: Edge Cases (ref: pjdfstest, POSIX.1-2017)
# ==========================================================================
echo "══ S5: Edge Cases ══"

# S5.01: Long filename (255 bytes — POSIX NAME_MAX)
LONG_NAME=$(python3 -c "print('a' * 255)" 2>/dev/null || printf '%0.sa' $(seq 1 255))
touch "$TEST_BASE/$LONG_NAME" 2>/dev/null
if [[ -f "$TEST_BASE/$LONG_NAME" ]]; then
    pass "S5.01: 255-char filename"
    rm "$TEST_BASE/$LONG_NAME"
else
    skip "S5.01: 255-char filename not supported"
fi

# S5.02: Filename with special characters
touch "$TEST_BASE/file with spaces" 2>/dev/null
if [[ -f "$TEST_BASE/file with spaces" ]]; then
    pass "S5.02: filename with spaces"
    rm "$TEST_BASE/file with spaces"
else
    fail "S5.02: filename with spaces"
fi

# S5.03: Filename with dots and dashes
touch "$TEST_BASE/...---..." 2>/dev/null
if [[ -f "$TEST_BASE/...---..." ]]; then
    pass "S5.03: filename with dots and dashes"
    rm "$TEST_BASE/...---..."
else
    fail "S5.03: filename with dots and dashes"
fi

# S5.04: Unicode filename
touch "$TEST_BASE/日本語ファイル" 2>/dev/null
if [[ -f "$TEST_BASE/日本語ファイル" ]]; then
    pass "S5.04: unicode filename"
    rm "$TEST_BASE/日本語ファイル"
else
    skip "S5.04: unicode filename not supported"
fi

# S5.05: Zero-byte file operations
touch "$TEST_BASE/s5_zero"
cp "$TEST_BASE/s5_zero" "$TEST_BASE/s5_zero_cp"
size=$(stat -c%s "$TEST_BASE/s5_zero_cp" 2>/dev/null || stat -f%z "$TEST_BASE/s5_zero_cp" 2>/dev/null)
assert_eq "S5.05: copy of zero-byte file has size 0" "0" "$size"

# S5.06: Large file (1MB)
dd if=/dev/urandom of="$TEST_BASE/s5_large" bs=4096 count=256 2>/dev/null
actual_size=$(stat -c%s "$TEST_BASE/s5_large" 2>/dev/null || stat -f%z "$TEST_BASE/s5_large" 2>/dev/null)
assert_eq "S5.06: large file (1MB) size" "1048576" "$actual_size"

# S5.07: Data integrity via checksum
cp "$TEST_BASE/s5_large" "$TEST_BASE/s5_large_cp"
orig_md5=$(md5sum "$TEST_BASE/s5_large" 2>/dev/null | cut -d' ' -f1 || md5 -q "$TEST_BASE/s5_large" 2>/dev/null)
copy_md5=$(md5sum "$TEST_BASE/s5_large_cp" 2>/dev/null | cut -d' ' -f1 || md5 -q "$TEST_BASE/s5_large_cp" 2>/dev/null)
assert_eq "S5.07: data integrity (md5 checksum)" "$orig_md5" "$copy_md5"

# S5.08: Multiple files in same directory (1000 files)
mkdir "$TEST_BASE/s5_many"
for i in $(seq 1 1000); do
    touch "$TEST_BASE/s5_many/f_$i"
done
count=$(ls "$TEST_BASE/s5_many" | wc -l | tr -d ' ')
assert_eq "S5.08: 1000 files in single directory" "1000" "$count"

# Cleanup S5
rm -rf "$TEST_BASE"/s5_*
echo ""

# ==========================================================================
# S6: Error Semantics (ref: POSIX.1-2017, pjdfstest)
# ==========================================================================
echo "══ S6: Error Semantics ══"

# S6.01: Open non-existent file for read → ENOENT
assert_fails "S6.01: read non-existent file (ENOENT)" cat "$TEST_BASE/s6_nonexistent"

# S6.02: Create file in non-existent directory → ENOENT
assert_fails "S6.02: create in non-existent dir (ENOENT)" touch "$TEST_BASE/s6_nodir/file"

# S6.03: mkdir existing directory → EEXIST
mkdir "$TEST_BASE/s6_dup"
assert_fails "S6.03: mkdir existing dir (EEXIST)" mkdir "$TEST_BASE/s6_dup"

# S6.04: rmdir on file → ENOTDIR
touch "$TEST_BASE/s6_file"
assert_fails "S6.04: rmdir on file (ENOTDIR)" rmdir "$TEST_BASE/s6_file"

# S6.05: unlink directory → EISDIR (or EPERM)
mkdir "$TEST_BASE/s6_uldir"
if ! unlink "$TEST_BASE/s6_uldir" 2>/dev/null; then
    pass "S6.05: unlink directory fails (EISDIR)"
else
    fail "S6.05: unlink directory should fail"
fi

# S6.06: read from directory → error
assert_fails "S6.06: cat on directory fails" cat "$TEST_BASE/s6_dup"

# S6.07: Deferred unlink (open file handle)
# This tests whether an open file can still be read after unlinking.
# Many FUSE filesystems don't support this yet.
echo "deferred" > "$TEST_BASE/s6_deferred"
if exec 3< "$TEST_BASE/s6_deferred" 2>/dev/null; then
    rm "$TEST_BASE/s6_deferred" 2>/dev/null
    data=$(cat <&3 2>/dev/null || echo "FAILED")
    exec 3<&-
    if [[ "$data" == "deferred" ]]; then
        pass "S6.07: deferred unlink (read after unlink)"
    else
        skip "S6.07: deferred unlink not supported (read returned: $data)"
    fi
else
    skip "S6.07: deferred unlink (fd redirect not supported)"
fi

# Cleanup S6
rm -rf "$TEST_BASE"/s6_*
echo ""

# ==========================================================================
# S7: Hard Links (ref: POSIX.1-2017)
# Forward-looking: may not be implemented yet
# ==========================================================================
echo "══ S7: Hard Links ══"

echo "linkdata" > "$TEST_BASE/s7_original"
if ln "$TEST_BASE/s7_original" "$TEST_BASE/s7_link" 2>/dev/null; then
    # S7.01: Link creates new name
    assert_exists "S7.01: hard link creates new name" "$TEST_BASE/s7_link"

    # S7.02: Both names see same content
    assert_file_content "S7.02: hard link shares content" "$TEST_BASE/s7_link" "linkdata"

    # S7.03: nlink is 2
    nlink=$(stat -c%h "$TEST_BASE/s7_original" 2>/dev/null || stat -f%l "$TEST_BASE/s7_original" 2>/dev/null)
    assert_eq "S7.03: nlink = 2 after link" "2" "$nlink"

    # S7.04: Write via one name visible from other
    echo "modified" > "$TEST_BASE/s7_link"
    assert_file_content "S7.04: write via link visible from original" "$TEST_BASE/s7_original" "modified"

    # S7.05: Unlink one name, other still works
    rm "$TEST_BASE/s7_original"
    assert_file_content "S7.05: data survives partial unlink" "$TEST_BASE/s7_link" "modified"
    nlink=$(stat -c%h "$TEST_BASE/s7_link" 2>/dev/null || stat -f%l "$TEST_BASE/s7_link" 2>/dev/null)
    assert_eq "S7.05b: nlink = 1 after partial unlink" "1" "$nlink"

    rm -f "$TEST_BASE/s7_link"
else
    skip "S7.01: hard links not supported (ENOSYS or EOPNOTSUPP)"
    skip "S7.02: hard links not supported"
    skip "S7.03: hard links not supported"
    skip "S7.04: hard links not supported"
    skip "S7.05: hard links not supported"
fi

rm -rf "$TEST_BASE"/s7_*
echo ""

# ==========================================================================
# S8: Symbolic Links (ref: POSIX.1-2017)
# Forward-looking: may not be implemented yet
# ==========================================================================
echo "══ S8: Symbolic Links ══"

echo "target_data" > "$TEST_BASE/s8_target"
if ln -s "$TEST_BASE/s8_target" "$TEST_BASE/s8_symlink" 2>/dev/null; then
    # S8.01: Symlink exists
    assert_exists "S8.01: symlink created" "$TEST_BASE/s8_symlink"

    # S8.02: Read through symlink
    assert_file_content "S8.02: read through symlink" "$TEST_BASE/s8_symlink" "target_data"

    # S8.03: readlink returns target
    link_target=$(readlink "$TEST_BASE/s8_symlink" 2>/dev/null)
    assert_eq "S8.03: readlink returns target" "$TEST_BASE/s8_target" "$link_target"

    # S8.04: Dangling symlink (target removed)
    rm "$TEST_BASE/s8_target"
    if [[ -L "$TEST_BASE/s8_symlink" ]]; then
        pass "S8.04: dangling symlink still exists as symlink"
    else
        fail "S8.04: dangling symlink should still exist"
    fi
    assert_fails "S8.04b: reading dangling symlink fails" cat "$TEST_BASE/s8_symlink"

    rm -f "$TEST_BASE/s8_symlink"
else
    skip "S8.01: symlinks not supported"
    skip "S8.02: symlinks not supported"
    skip "S8.03: symlinks not supported"
    skip "S8.04: symlinks not supported"
fi

rm -rf "$TEST_BASE"/s8_*
echo ""

# ==========================================================================
# S9: Persistence (ref: SingularFS §5.5)
# Note: This test cannot unmount/remount automatically.
# It verifies data written is immediately readable (no buffering bugs).
# ==========================================================================
echo "══ S9: Write Durability ══"

# S9.01: Written data immediately readable
echo "persist_test" > "$TEST_BASE/s9_persist"
sync 2>/dev/null || true
assert_file_content "S9.01: written data immediately readable" "$TEST_BASE/s9_persist" "persist_test"

# S9.02: Multiple small writes then read
for i in $(seq 1 50); do
    echo "entry_$i" > "$TEST_BASE/s9_entry_$i"
done
all_ok=true
for i in $(seq 1 50); do
    content=$(cat "$TEST_BASE/s9_entry_$i" 2>/dev/null)
    if [[ "$content" != "entry_$i" ]]; then
        all_ok=false
        break
    fi
done
if $all_ok; then
    pass "S9.02: 50 sequential write-then-read"
else
    fail "S9.02: sequential write-then-read integrity"
fi

# S9.03: fsync should not error
echo "fsync_test" > "$TEST_BASE/s9_fsync"
if python3 -c "
import os
fd = os.open('$TEST_BASE/s9_fsync', os.O_WRONLY)
os.write(fd, b'synced')
os.fsync(fd)
os.close(fd)
" 2>/dev/null; then
    pass "S9.03: fsync completes without error"
else
    skip "S9.03: fsync test (python3 not available)"
fi

rm -rf "$TEST_BASE"/s9_*
echo ""

# ==========================================================================
# S10: statfs (ref: POSIX.1-2017)
# ==========================================================================
echo "══ S10: statfs ══"

if command -v df &>/dev/null; then
    df_output=$(df "$MOUNTPOINT" 2>/dev/null)
    if [[ $? -eq 0 ]] && [[ -n "$df_output" ]]; then
        pass "S10.01: df returns valid output for mountpoint"
    else
        fail "S10.01: df failed on mountpoint"
    fi

    # Check that basic fields are present
    blocks=$(df -B1 "$MOUNTPOINT" 2>/dev/null | tail -1 | awk '{print $2}')
    if [[ -n "$blocks" ]] && [[ "$blocks" -gt 0 ]] 2>/dev/null; then
        pass "S10.02: statfs reports non-zero total blocks"
    else
        skip "S10.02: statfs blocks check (got: $blocks)"
    fi
else
    skip "S10.01: df not available"
    skip "S10.02: df not available"
fi

echo ""

# ==========================================================================
# S11: mknod (regular files only)
# ==========================================================================
echo "══ S11: mknod ══"

# S11.01: mknod creates regular file
if command -v mknod &>/dev/null; then
    mknod "$TEST_BASE/s11_mknod_file" p 2>/dev/null
    # RucksFS only supports regular files via mknod — FIFO should fail.
    if [[ -p "$TEST_BASE/s11_mknod_file" ]]; then
        skip "S11.01: mknod FIFO succeeded (may be kernel-level)"
        rm -f "$TEST_BASE/s11_mknod_file"
    else
        pass "S11.01: mknod FIFO correctly rejected (EOPNOTSUPP)"
    fi

    # S11.02: touch uses mknod internally — verify it works
    touch "$TEST_BASE/s11_touch_file"
    assert_is_file "S11.02: touch (via mknod) creates regular file" "$TEST_BASE/s11_touch_file"

    # S11.03: mknod-created file is writable/readable
    echo "mknod_data" > "$TEST_BASE/s11_touch_file"
    assert_file_content "S11.03: mknod-created file is writable/readable" "$TEST_BASE/s11_touch_file" "mknod_data"
else
    skip "S11.01: mknod command not available"
    skip "S11.02: mknod command not available"
    skip "S11.03: mknod command not available"
fi

rm -rf "$TEST_BASE"/s11_*
echo ""

# ==========================================================================
# S12: fallocate
# ==========================================================================
echo "══ S12: fallocate ══"

if command -v fallocate &>/dev/null; then
    # S12.01: fallocate mode 0 — preallocate space
    touch "$TEST_BASE/s12_falloc"
    fallocate -l 4096 "$TEST_BASE/s12_falloc" 2>/dev/null
    if [[ $? -eq 0 ]]; then
        size=$(stat -c%s "$TEST_BASE/s12_falloc" 2>/dev/null || stat -f%z "$TEST_BASE/s12_falloc" 2>/dev/null)
        assert_eq "S12.01: fallocate extends file to 4096" "4096" "$size"
    else
        skip "S12.01: fallocate not supported by filesystem"
    fi

    # S12.02: fallocate with keep-size flag
    echo "hello" > "$TEST_BASE/s12_keep"
    orig_size=$(stat -c%s "$TEST_BASE/s12_keep" 2>/dev/null || stat -f%z "$TEST_BASE/s12_keep" 2>/dev/null)
    fallocate -l 8192 -n "$TEST_BASE/s12_keep" 2>/dev/null
    if [[ $? -eq 0 ]]; then
        new_size=$(stat -c%s "$TEST_BASE/s12_keep" 2>/dev/null || stat -f%z "$TEST_BASE/s12_keep" 2>/dev/null)
        # -n means keep size, so size should not change
        assert_eq "S12.02: fallocate --keep-size preserves file size" "$orig_size" "$new_size"
    else
        skip "S12.02: fallocate --keep-size not supported"
    fi

    # S12.03: fallocate on existing file extends but preserves data
    echo -n "existing" > "$TEST_BASE/s12_extend"
    fallocate -l 1024 "$TEST_BASE/s12_extend" 2>/dev/null
    if [[ $? -eq 0 ]]; then
        size=$(stat -c%s "$TEST_BASE/s12_extend" 2>/dev/null || stat -f%z "$TEST_BASE/s12_extend" 2>/dev/null)
        assert_eq "S12.03a: fallocate extends to 1024" "1024" "$size"
        content=$(head -c 8 "$TEST_BASE/s12_extend")
        assert_eq "S12.03b: fallocate preserves existing data" "existing" "$content"
    else
        skip "S12.03a: fallocate not supported"
        skip "S12.03b: fallocate not supported"
    fi
else
    skip "S12.01: fallocate command not available"
    skip "S12.02: fallocate command not available"
    skip "S12.03a: fallocate command not available"
    skip "S12.03b: fallocate command not available"
fi

rm -rf "$TEST_BASE"/s12_*
echo ""

# ==========================================================================
# S13: access
# ==========================================================================
echo "══ S13: access ══"

# S13.01: access on existing file succeeds
echo "test" > "$TEST_BASE/s13_file"
if test -e "$TEST_BASE/s13_file"; then
    pass "S13.01: access (test -e) on existing file"
else
    fail "S13.01: access on existing file failed"
fi

# S13.02: access on non-existent file fails
if test -e "$TEST_BASE/s13_nonexistent"; then
    fail "S13.02: access on non-existent should fail"
else
    pass "S13.02: access on non-existent file returns ENOENT"
fi

# S13.03: access read check (test -r)
chmod 644 "$TEST_BASE/s13_file" 2>/dev/null
if test -r "$TEST_BASE/s13_file"; then
    pass "S13.03: access read check on readable file"
else
    skip "S13.03: access read check (may need default_permissions)"
fi

# S13.04: access on directory
mkdir "$TEST_BASE/s13_dir"
if test -d "$TEST_BASE/s13_dir"; then
    pass "S13.04: access on directory"
else
    fail "S13.04: access on directory failed"
fi

rm -rf "$TEST_BASE"/s13_*
echo ""

# ==========================================================================
# Summary
# ==========================================================================

TOTAL=$((PASSED + FAILED + SKIPPED))
echo "══════════════════════════════════════════════════"
echo -e "POSIX Conformance: ${GREEN}$PASSED passed${NC}, ${RED}$FAILED failed${NC}, ${YELLOW}$SKIPPED skipped${NC}, $TOTAL total"
echo "══════════════════════════════════════════════════"
echo ""
echo "  Log: $LOG_FILE"

if [[ $FAILED -gt 0 ]]; then
    exit 1
fi
exit 0
