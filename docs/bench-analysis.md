# Benchmark Analysis: Per-Layer Overhead Quantification

Date: 2026-04-11 (updated with clean retest)
Branch: feat/fuser-multithread (fuser 0.17, n_threads=8, block_on)
Environment: 8C16G CVM × 3 (client / meta / data), same VPC, Ubuntu 22.04

## Methodology

To fairly compare "create" throughput across layers, the benchmark tool was
aligned with mdtest's actual syscall behavior. Strace of mdtest shows:

```
openat(AT_FDCWD, "file.mdtest.0.0", O_RDWR|O_CREAT, 0664)
```

This single `openat(O_CREAT)` triggers the following FUSE operations:
1. `FUSE_LOOKUP` — kernel checks if file exists (returns ENOENT)
2. `FUSE_CREATE` — atomic create + open (returns entry + file handle)
3. `FUSE_FLUSH` — triggered by close()
4. `FUSE_RELEASE` — triggered by close()

The bench tool's `--realistic` mode replicates this: `lookup + create + open + release`.

## Results

### mdtest File Creation — RucksFS vs JuiceFS MySQL (clean retest)

Test: `mpirun -np N mdtest -C -n 3000 -d <dir> -u`
All services freshly restarted, data dirs cleaned.

| np | RucksFS (ops/s) | JuiceFS MySQL (ops/s) | RucksFS / JuiceFS |
|----|-----------------|----------------------|-------------------|
| 1  | 754             | 227                  | **3.3x faster**   |
| 2  | 1,464           | 449                  | **3.3x faster**   |
| 4  | 2,807           | 824                  | **3.4x faster**   |
| 8  | 4,258           | 1,437                | **3.0x faster**   |
| 16 | 4,240           | 2,344                | **1.8x faster**   |

**Scaling:**

|          | 1→2  | 1→4  | 1→8  | 1→16 |
|----------|------|------|------|------|
| RucksFS  | 1.9x | 3.7x | 5.6x | 5.6x |
| JuiceFS  | 2.0x | 3.6x | 6.3x | 10.3x |

### mdtest File Stat — RucksFS vs JuiceFS MySQL

Test: `mpirun -np N mdtest -C -T -n 2000 -d <dir> -u`

| np | RucksFS (ops/s) | JuiceFS MySQL (ops/s) | RucksFS / JuiceFS |
|----|-----------------|----------------------|-------------------|
| 1  | 3,926           | 2,346                | **1.7x faster**   |
| 4  | 13,016          | 7,763                | **1.7x faster**   |
| 8  | 19,097          | 12,537               | **1.5x faster**   |
| 16 | 18,648          | 20,234               | **0.9x (slower)**  |

### Per-Layer Overhead (internal bench tool)

#### Layer 1: Local MetadataOps (in-process RocksDB, no network, no FUSE)

| Mode | 1T | 2T | 4T | 8T |
|------|-----|-----|------|------|
| create(raw) | 148,413 | 253,743 | 204,634 | 260,777 |
| create(real) | 128,336 | 119,712 | 172,814 | 251,513 |

#### Layer 2: gRPC MetadataRpcClient (remote MetadataServer, no FUSE)

**Raw create (1 RPC per op):**

| Threads | create ops/s | Avg latency (us) |
|---------|-------------|------------------|
| 1 | 3,648 | 274 |
| 2 | 7,363 | 271 |
| 4 | 12,955 | 308 |
| 8 | 20,884 | 381 |

**Realistic create (4 RPCs per op: lookup+create+open+release):**

| Threads | create ops/s | Avg latency (us) |
|---------|-------------|------------------|
| 1 | 916 | 1,091 |
| 2 | 1,871 | 1,068 |
| 4 | 3,557 | 1,122 |
| 8 | 5,725 | 1,395 |

#### Layer 3: FUSE + gRPC (mdtest, fuser 0.17, 8 FUSE threads)

| np | create ops/s | stat ops/s |
|----|-------------|-----------|
| 1  | 754         | 3,926     |
| 4  | 2,807       | 13,016    |
| 8  | 4,258       | 19,097    |
| 16 | 4,240       | 18,648    |

## Overhead Breakdown (4-thread create)

| Layer | ops/s | vs previous | Cumulative |
|-------|-------|-------------|------------|
| Local MetadataOps (realistic) | 172,814 | baseline | 1x |
| gRPC (realistic, 4 RPCs) | 3,557 | **48.6x** | 48.6x |
| FUSE + gRPC (mdtest np=4) | 2,807 | **1.3x** | 61.6x |

## Key Insights

1. **RucksFS is 3x faster than JuiceFS MySQL** at create, consistent across
   np=1–8. Both scale similarly up to 8 processes.

2. **RucksFS saturates at np=8** (4,258→4,240 at np=16) while JuiceFS continues
   to scale (1,437→2,344). This is because RucksFS has 8 FUSE threads = 8 cores,
   while JuiceFS uses goroutines that handle more concurrency per core.

3. **RucksFS stat is ~1.7x faster** up to np=8, but JuiceFS overtakes at np=16
   (likely due to JuiceFS's client-side attr cache kicking in).

4. **gRPC is the dominant overhead** (48.6x). FUSE adds only 1.3x on top.
   Each create = 4 sequential RPC round-trips at ~274us each.

5. **gRPC realistic create scales linearly** (916→5,725, 6.3x at 8T),
   confirming the server has no bottleneck.

## Optimization Priorities

1. **Reduce RPC round-trips per create** — The FUSE CREATE callback currently
   does 1 RPC; lookup and release are separate. Ensure the FUSE layer uses
   the atomic CREATE op (no separate lookup).
2. **Increase FUSE thread count** — Currently 8, matching core count. Consider
   overprovisioning (e.g., 32) since threads block on gRPC I/O.
3. **Client-side attr cache** — Eliminate ENOENT lookup RPCs.
4. **Batch/streaming RPCs** — Amortize round-trip latency for bulk operations.
