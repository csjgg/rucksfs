#!/bin/bash
# Full benchmark matrix for RucksFS performance evaluation
# Run on Machine A (client/test-driver)
set -euo pipefail

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_DIR="/data/test-results/comparison_${TIMESTAMP}"
mkdir -p "$RESULT_DIR"

TARGETS=(
    "ext4:/data/ext4-bench"
    "rucksfs-embedded:/mnt/rucksfs-embedded"
    "rucksfs-dist:/mnt/rucksfs-dist"
    "juicefs-mysql:/mnt/juicefs-mysql"
    "juicefs-redis:/mnt/juicefs-redis"
    "nfs:/mnt/nfs"
)

N_FILES=10000

echo "============================================================"
echo "  RucksFS Benchmark — Full Matrix"
echo "  Time: $(date)"
echo "  Files per test: $N_FILES"
echo "  Results: $RESULT_DIR"
echo "============================================================"

# Record environment
{
    echo "=== Environment ==="
    date
    uname -a
    lscpu | head -20
    free -h
    echo "=== Mounts ==="
    mount | grep -E "rucksfs|juicefs|nfs|ext4-bench"
} > "$RESULT_DIR/environment.txt"

# Try to set CPU governor to performance (may fail without permissions)
sudo cpupower frequency-set -g performance 2>/dev/null || true

# ============================================================
# Phase 1: Single-process mdtest (core comparison)
# ============================================================
echo ""
echo "============================================================"
echo "  Phase 1: Single-process mdtest (-n $N_FILES, -i 3)"
echo "============================================================"

for entry in "${TARGETS[@]}"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    test_dir="$path/mdtest_run"

    echo ""
    echo "--- $name ($path) ---"

    # Cleanup
    sudo rm -rf "$test_dir" 2>/dev/null || true
    sudo mkdir -p "$test_dir"

    # Drop caches
    sudo sh -c "echo 3 > /proc/sys/vm/drop_caches" 2>/dev/null || true

    # Warmup (not counted)
    echo "  [warmup]"
    sudo mdtest -d "$test_dir" -n 1000 -F -C -T -r -u 2>/dev/null || true
    sudo rm -rf "$test_dir"/* 2>/dev/null || true

    # Actual test
    echo "  [running mdtest single-process]"
    sudo mdtest -d "$test_dir" -n "$N_FILES" -F -C -T -r -u -i 3 \
        2>&1 | tee "$RESULT_DIR/${name}_single.txt"

    sudo rm -rf "$test_dir"/* 2>/dev/null || true
done

# ============================================================
# Phase 2: Multi-process scaling (file create)
# ============================================================
echo ""
echo "============================================================"
echo "  Phase 2: Multi-process scaling (-n $N_FILES, -i 3)"
echo "============================================================"

for entry in "${TARGETS[@]}"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    test_dir="$path/mdtest_run"

    echo ""
    echo "--- $name ($path) ---"

    for np in 1 2 4 8; do
        sudo rm -rf "$test_dir" 2>/dev/null || true
        sudo mkdir -p "$test_dir"
        sudo sh -c "echo 3 > /proc/sys/vm/drop_caches" 2>/dev/null || true

        echo "  [np=$np]"
        sudo mpirun --allow-run-as-root -np $np \
            mdtest -d "$test_dir" -n "$N_FILES" -F -u -i 3 \
            2>&1 | tee "$RESULT_DIR/${name}_np${np}.txt"

        sudo rm -rf "$test_dir"/* 2>/dev/null || true
    done
done

# ============================================================
# Phase 3: Directory tree test (JuiceFS standard params)
# ============================================================
echo ""
echo "============================================================"
echo "  Phase 3: Directory tree test (-b 6 -I 8 -z 4)"
echo "============================================================"

for entry in "${TARGETS[@]}"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    test_dir="$path/mdtest_run"

    echo ""
    echo "--- $name ($path) ---"

    sudo rm -rf "$test_dir" 2>/dev/null || true
    sudo mkdir -p "$test_dir"
    sudo sh -c "echo 3 > /proc/sys/vm/drop_caches" 2>/dev/null || true

    echo "  [tree test]"
    sudo mdtest -d "$test_dir" -b 6 -I 8 -z 4 -i 3 \
        2>&1 | tee "$RESULT_DIR/${name}_tree.txt"

    sudo rm -rf "$test_dir"/* 2>/dev/null || true
done

# ============================================================
# Phase 4: pjdfstest (correctness)
# ============================================================
echo ""
echo "============================================================"
echo "  Phase 4: pjdfstest (POSIX correctness)"
echo "============================================================"

PJDFS="/opt/pjdfstest"
if [ -x "$PJDFS/pjdfstest" ]; then
    for target in "rucksfs-embedded:/mnt/rucksfs-embedded" "rucksfs-dist:/mnt/rucksfs-dist" "juicefs-mysql:/mnt/juicefs-mysql"; do
        name="${target%%:*}"
        path="${target##*:}"

        echo ""
        echo "--- pjdfstest: $name ($path) ---"

        pjd_dir="$path/pjdfstest_run"
        sudo mkdir -p "$pjd_dir"
        cd "$pjd_dir"

        sudo prove -r "$PJDFS/tests/" 2>&1 | tail -20 | tee "$RESULT_DIR/${name}_pjdfstest.txt"

        cd /
        sudo rm -rf "$pjd_dir" 2>/dev/null || true
    done
else
    echo "WARNING: pjdfstest not found at $PJDFS, skipping"
fi

# ============================================================
# Summary
# ============================================================
echo ""
echo "============================================================"
echo "  Benchmark Complete!"
echo "  Time: $(date)"
echo "  Results saved to: $RESULT_DIR"
echo "============================================================"
echo ""
echo "Files:"
ls -la "$RESULT_DIR/"
