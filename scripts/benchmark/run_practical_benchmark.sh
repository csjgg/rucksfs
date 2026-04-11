#!/bin/bash
# Practical benchmark matrix for RucksFS performance evaluation
# Handles slow distributed targets gracefully
set -euo pipefail

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_DIR="/data/test-results/comparison_${TIMESTAMP}"
mkdir -p "$RESULT_DIR"

echo "============================================================"
echo "  RucksFS Benchmark — Practical Matrix"
echo "  Time: $(date)"
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
    mount | grep -E "rucksfs|juicefs|nfs" || true
    df -h /data/ext4-bench /mnt/rucksfs-embedded /mnt/rucksfs-dist /mnt/juicefs-mysql /mnt/juicefs-redis /mnt/nfs 2>/dev/null || true
} > "$RESULT_DIR/environment.txt"

sudo cpupower frequency-set -g performance 2>/dev/null || true

# Helper: run mdtest with timeout
run_mdtest() {
    local name="$1"
    local path="$2"
    local n_files="$3"
    local extra_args="$4"
    local output_file="$5"
    local test_dir="$path/mdtest_run"

    sudo rm -rf "$test_dir" 2>/dev/null || true
    sudo mkdir -p "$test_dir"
    sudo sh -c "echo 3 > /proc/sys/vm/drop_caches" 2>/dev/null || true

    echo "  [$name: n=$n_files $extra_args]"
    timeout 300 sudo mdtest -d "$test_dir" -n "$n_files" $extra_args \
        2>&1 | tee "$output_file"
    local rc=$?

    sudo rm -rf "$test_dir"/* 2>/dev/null || true
    return $rc
}

# ============================================================
# Phase 1: Single-process mdtest (core comparison)
# ============================================================
echo ""
echo "============================================================"
echo "  Phase 1: Single-process mdtest"
echo "============================================================"

# Fast targets: 10000 files, 3 iterations
for entry in "ext4:/data/ext4-bench" "rucksfs-embedded:/mnt/rucksfs-embedded" "juicefs-redis:/mnt/juicefs-redis" "nfs:/mnt/nfs"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    echo ""
    echo "--- $name ---"

    # Warmup
    run_mdtest "${name}-warmup" "$path" 1000 "-F -C -T -r -u" "/dev/null" || true

    # Actual test
    run_mdtest "$name" "$path" 10000 "-F -C -T -r -u -i 3" "$RESULT_DIR/${name}_single.txt" || true
done

# Slow targets: 1000 files, 3 iterations
for entry in "rucksfs-dist:/mnt/rucksfs-dist" "juicefs-mysql:/mnt/juicefs-mysql"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    echo ""
    echo "--- $name (1000 files, slower target) ---"

    # Warmup
    run_mdtest "${name}-warmup" "$path" 100 "-F -C -T -r -u" "/dev/null" || true

    # Actual test
    run_mdtest "$name" "$path" 1000 "-F -C -T -r -u -i 3" "$RESULT_DIR/${name}_single.txt" || true
done

# ============================================================
# Phase 2: Multi-process scaling (file create, fast targets)
# ============================================================
echo ""
echo "============================================================"
echo "  Phase 2: Multi-process scaling"
echo "============================================================"

for entry in "ext4:/data/ext4-bench" "rucksfs-embedded:/mnt/rucksfs-embedded" "juicefs-redis:/mnt/juicefs-redis" "nfs:/mnt/nfs"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    echo ""
    echo "--- $name (scaling) ---"

    for np in 1 2 4 8; do
        test_dir="$path/mdtest_run"
        sudo rm -rf "$test_dir" 2>/dev/null || true
        sudo mkdir -p "$test_dir"
        sudo sh -c "echo 3 > /proc/sys/vm/drop_caches" 2>/dev/null || true

        echo "  [np=$np]"
        timeout 300 sudo mpirun --allow-run-as-root -np $np \
            mdtest -d "$test_dir" -n 10000 -F -u -i 3 \
            2>&1 | tee "$RESULT_DIR/${name}_np${np}.txt" || true

        sudo rm -rf "$test_dir"/* 2>/dev/null || true
    done
done

# Slow targets: 1000 files per process
for entry in "rucksfs-dist:/mnt/rucksfs-dist" "juicefs-mysql:/mnt/juicefs-mysql"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    echo ""
    echo "--- $name (scaling, 1000 files) ---"

    for np in 1 2 4; do
        test_dir="$path/mdtest_run"
        sudo rm -rf "$test_dir" 2>/dev/null || true
        sudo mkdir -p "$test_dir"
        sudo sh -c "echo 3 > /proc/sys/vm/drop_caches" 2>/dev/null || true

        echo "  [np=$np]"
        timeout 600 sudo mpirun --allow-run-as-root -np $np \
            mdtest -d "$test_dir" -n 1000 -F -u -i 3 \
            2>&1 | tee "$RESULT_DIR/${name}_np${np}.txt" || true

        sudo rm -rf "$test_dir"/* 2>/dev/null || true
    done
done

# ============================================================
# Phase 3: Directory tree test
# ============================================================
echo ""
echo "============================================================"
echo "  Phase 3: Directory tree test (-b 6 -I 8 -z 4)"
echo "============================================================"

for entry in "ext4:/data/ext4-bench" "rucksfs-embedded:/mnt/rucksfs-embedded" "juicefs-redis:/mnt/juicefs-redis" "nfs:/mnt/nfs"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    echo ""
    echo "--- $name ---"
    run_mdtest "$name" "$path" 0 "-b 6 -I 8 -z 4 -i 3" "$RESULT_DIR/${name}_tree.txt" || true
done

for entry in "rucksfs-dist:/mnt/rucksfs-dist" "juicefs-mysql:/mnt/juicefs-mysql"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    echo ""
    echo "--- $name (tree, slower) ---"
    run_mdtest "$name" "$path" 0 "-b 3 -I 4 -z 3 -i 3" "$RESULT_DIR/${name}_tree.txt" || true
done

# ============================================================
# Phase 4: pjdfstest (correctness, quick subset)
# ============================================================
echo ""
echo "============================================================"
echo "  Phase 4: pjdfstest"
echo "============================================================"

PJDFS="/opt/pjdfstest"
if [ -x "$PJDFS/pjdfstest" ]; then
    for target in "rucksfs-embedded:/mnt/rucksfs-embedded" "juicefs-mysql:/mnt/juicefs-mysql"; do
        name="${target%%:*}"
        path="${target##*:}"

        echo ""
        echo "--- pjdfstest: $name ---"

        pjd_dir="$path/pjdfstest_run"
        sudo mkdir -p "$pjd_dir"
        pushd "$pjd_dir" > /dev/null

        # Run a focused subset (chmod, chown, mkdir, open, rename, unlink)
        for suite in chmod mkdir open rename unlink; do
            if [ -d "$PJDFS/tests/$suite" ]; then
                echo "  [$suite]"
                sudo prove -r "$PJDFS/tests/$suite/" 2>&1 | tail -3
            fi
        done | tee "$RESULT_DIR/${name}_pjdfstest.txt"

        popd > /dev/null
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
echo "  Results: $RESULT_DIR"
echo "============================================================"

# Extract summary table
echo ""
echo "=== Single-process Summary (Mean ops/s) ==="
printf "%-20s %12s %12s %12s %12s %12s\n" "Target" "Create" "Stat" "Remove" "TreeCreate" "TreeRemove"
echo "-------------------- ------------ ------------ ------------ ------------ ------------"

for f in "$RESULT_DIR"/*_single.txt; do
    name=$(basename "$f" _single.txt)
    create=$(grep "File creation" "$f" | awk '{print $3}' | tail -1)
    stat=$(grep "File stat" "$f" | awk '{print $3}' | tail -1)
    remove=$(grep "File removal" "$f" | awk '{print $3}' | tail -1)
    tree_c=$(grep "Tree creation" "$f" | awk '{print $3}' | tail -1)
    tree_r=$(grep "Tree removal" "$f" | awk '{print $3}' | tail -1)
    printf "%-20s %12s %12s %12s %12s %12s\n" "$name" "$create" "$stat" "$remove" "$tree_c" "$tree_r"
done

echo ""
ls -la "$RESULT_DIR/"
