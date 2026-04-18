# RucksFS vs NFS Controlled Benchmark Report (v2)

**Date:** 2026-04-18 06:11 — 08:07 (CST)
**Duration:** ~2 hours
**Cluster:** Tencent Cloud Hong Kong (ap-hongkong-2)
**Version:** v2 — symmetric 2-server topology

---

## 1. Motivation

The v1 controlled benchmark (2026-04-17) identified the following residual issues:

| Issue | Impact |
|-------|--------|
| RucksFS MDS ran on a dedicated Meta machine; NFS shared the Data machine with RucksFS DataServer | Topology asymmetry: RucksFS had a dedicated server, NFS did not |
| NFS and RucksFS tests ran interleaved at each np | Potential resource cross-contamination between test phases |
| nfsd thread count fixed at 64 without per-machine validation | May not be optimal for the specific hardware |
| No warmup phase before measurement runs | Cold VFS/mount path could penalize the first run |

This v2 experiment eliminates all of the above with a fully symmetric design.

---

## 2. Experimental Setup

### 2.1 Cluster Topology (Symmetric 2-Server)

```
┌───────────────────────────────────────┐
│  Client (8C16G, SA5.2XLARGE16)        │
│  - mdtest 4.1.0+dev (OpenMPI)         │
│  - RucksFS FUSE client (fuse3)        │
│  - NFS v4.2 client (noac)             │
│  Private IP: 10.0.1.11                │
└────────────────┬──────────────────────┘
                 │  VPC internal network
    ┌────────────┴────────────┐
    │                         │
┌───┴───────────────────┐  ┌──┴────────────────────┐
│  Server-1 (8C16G)     │  │  Server-2 (8C16G)      │
│  SA5.2XLARGE16        │  │  SA5.2XLARGE16          │
│  DEDICATED to RucksFS │  │  DEDICATED to NFS       │
│  - MetadataServer     │  │  - nfs-kernel-server    │
│    (gRPC :8001)       │  │    (ext4 backend)       │
│  - DataServer         │  │  - No other services    │
│    (gRPC :8002)       │  │                         │
│  - RocksDB on SSD     │  │  Private IP: 10.0.1.8  │
│  Private IP: 10.0.1.15│  │                         │
└───────────────────────┘  └────────────────────────┘
```

**Key design choice:** Each filesystem has its own **dedicated identical server**. No resource sharing. This eliminates the topology asymmetry from v1.

### 2.2 Hardware Specifications (Identical for All 3 Machines)

| Component | Specification |
|-----------|---------------|
| Instance type | SA5.2XLARGE16 |
| CPU | AMD EPYC 9754, 4 cores × 2 threads = **8 vCPUs** |
| Memory | **15 GiB** DDR5 |
| Data disk | 200 GB CLOUD_SSD, formatted as **ext4** (`noatime`) |
| Kernel | Linux 5.15.0-171-generic (Ubuntu 22.04) |
| Hypervisor | KVM |

### 2.3 Control Variable Checklist

| Variable | RucksFS test | NFS test | Controlled? |
|----------|-------------|----------|-------------|
| Client hardware | 8C16G SA5.2XLARGE16 | Same | **Yes** |
| Server hardware | 8C16G SA5.2XLARGE16 (Server-1) | 8C16G SA5.2XLARGE16 (Server-2) | **Yes** (identical) |
| Server isolation | Server-1 dedicated | Server-2 dedicated | **Yes** (no sharing) |
| Server disk | 200G SSD, ext4, noatime | Same | **Yes** |
| Network | VPC 10.0.1.0/24 | Same VPC | **Yes** |
| Network RTT | 0.137ms (to Server-1) | 0.154ms (to Server-2) | **Yes** (12% diff, negligible) |
| Network bandwidth | 3.35 Gbps | 3.77 Gbps | **Yes** (~11% diff, metadata-bound) |
| OS / Kernel | Ubuntu 22.04 / 5.15.0-171 | Same | **Yes** |
| Test tool | mdtest 4.1.0+dev | Same | **Yes** |
| mdtest parameters | `-n 5000 -F -C -T -r -u -i 3` | Same | **Yes** |
| Cache clearing | `drop_caches` on all 3 nodes before each run | Same | **Yes** |
| Client-side caching | None (FUSE, no attr cache) | `noac` (attr cache disabled) | **Yes** |
| Warmup | 100-file throwaway run before each phase | Same | **Yes** |
| Iterations | 3 per mdtest run, 3 independent runs | Same | **Yes** |
| Execution order | **NFS all np FIRST**, then RucksFS all np | Serial, no overlap | **Yes** |
| Server concurrency | tokio async (8 workers) | nfsd (scanned: 8/16/32/64, optimal=16) | **Each at its best** |

### 2.4 Software Configuration

**RucksFS (distributed mode):**
- MetadataServer: gRPC on port 8001, RocksDB backend, default settings
- DataServer: gRPC on port 8002, RawDisk backend
- FUSE client: fuse3 + tokio async runtime, `default_permissions`, `allow_other`
- Concurrency: tokio multi-thread runtime, 8 worker threads (= CPU count)

**NFS:**
- Server: Linux kernel NFS server (`nfs-kernel-server`)
- Export: `/data/nfs-export *(rw,sync,no_subtree_check,no_root_squash)`
- Backend: ext4 on SSD (same physical disk type as RocksDB)
- Client mount: `mount -t nfs -o noac,vers=4.2 10.0.1.8:/data/nfs-export /mnt/nfs`
- Thread count: **16** (determined by Experiment 1.5 thread scan)

### 2.5 Test Methodology

- **Tool**: mdtest from the IOR project (HPC standard metadata benchmark)
- **Parameters**: `-n 5000` (files per process), `-F` (files only), `-C -T -r` (create, stat, remove), `-u` (unique directory per process), `-i 3` (3 internal iterations)
- **Cache clearing**: Before each mdtest run, `sync && echo 3 > /proc/sys/vm/drop_caches` executed on **all 3 nodes** (Client, Server-1, Server-2) + 2s settle time
- **Cleanup**: Server-side `find /data/nfs-export -mindepth 1 -delete` between NFS runs to eliminate stale file handle issues
- **Warmup**: 100-file single-process throwaway run before each test phase to prime the FUSE/NFS mount path
- **Repetition**: Each configuration repeated **3 independent runs**; each run reports the **max** of 3 internal mdtest iterations
- **Execution order**: **Serial** — all NFS tests complete before any RucksFS test starts, preventing cross-interference

---

## 3. Experiment 1: Network Symmetry Verification

### 3.1 Latency (ICMP ping, 50 packets)

| Path | Min | Avg | Max | Mdev |
|------|-----|-----|-----|------|
| Client → Server-1 (RucksFS) | 0.100ms | **0.137ms** | 0.219ms | 0.028ms |
| Client → Server-2 (NFS) | 0.122ms | **0.154ms** | 0.200ms | 0.013ms |

Latency difference: 0.017ms (12%) — negligible for metadata operations (individual RPCs take >0.3ms).

### 3.2 Bandwidth (iperf3, 10 seconds)

| Path | Throughput |
|------|-----------|
| Client → Server-1 (RucksFS) | **3.35 Gbps** |
| Client → Server-2 (NFS) | **3.77 Gbps** |

NFS server has 12% more bandwidth — this slightly **favors NFS** in this comparison. For metadata-bound workloads, bandwidth is not the bottleneck.

---

## 4. Experiment 1.5: NFS Thread Count Scan

**Purpose:** Determine the optimal nfsd thread count on the dedicated Server-2, eliminating the concern that NFS underperforms due to thread starvation.

**Fixed:** np=16, all other parameters as above. Warmup before each thread count change.
**Variable:** nfsd threads = {8, 16, 32, 64}.

### 4.1 Results (ops/sec, mean of 3 cold-start runs)

| nfsd threads | File Creation | File Stat | File Removal |
|:---:|---:|---:|---:|
| 8 | 3,275 | 6,593 | 2,933 |
| **16** | **3,565** | **6,571** | **3,338** |
| 32 | 3,544 | 6,598 | 3,361 |
| 64 | 3,510 | 6,606 | 3,322 |

### 4.2 Raw Data (File Creation, ops/sec per run)

| nfsd threads | Run 1 | Run 2 | Run 3 | Mean | Δ vs 16 |
|:---:|---:|---:|---:|---:|---:|
| 8 | 3,266 | 3,293 | 3,268 | 3,275 | −8.1% |
| **16** | **3,550** | **3,602** | **3,544** | **3,565** | baseline |
| 32 | 3,554 | 3,543 | 3,534 | 3,544 | −0.6% |
| 64 | 3,519 | 3,492 | 3,520 | 3,510 | −1.5% |

### 4.3 Analysis

- **8 → 16 threads**: Create +8.8%, Stat −0.3%, Remove +13.8% — significant improvement
- **16 → 32 threads**: Create −0.6%, Stat +0.4%, Remove +0.7% — within noise
- **32 → 64 threads**: Create −1.0%, Stat +0.1%, Remove −1.2% — within noise

**Conclusion:** NFS performance saturates at **16 nfsd threads** on this 8-core machine. This is consistent with v1 results and confirms that the NFS performance ceiling is **not caused by thread starvation** but by fundamental limitations of the ext4 metadata path and/or NFS protocol serialization.

**Optimal configuration used in subsequent experiments: 16 nfsd threads.**

---

## 5. Experiment 2: Concurrency Scaling Comparison

**Purpose:** Compare RucksFS-dist vs NFS metadata throughput across concurrent process counts, with both systems on dedicated identical servers using their optimal configuration.

**Fixed:** n=5000, NFS with 16 nfsd threads (optimal from Exp 1.5), each system on dedicated server.
**Variable:** np = {1, 2, 4, 8, 16, 32}.

### 5.1 Results — File Creation (ops/sec)

| np | NFS (mean) | RucksFS (mean) | RucksFS / NFS |
|:---:|---:|---:|:---:|
| 1 | 462 | 630 | **1.36x** |
| 2 | 856 | 1,236 | **1.44x** |
| 4 | 1,340 | 2,313 | **1.73x** |
| 8 | 2,388 | 3,770 | **1.58x** |
| 16 | 3,576 | 5,398 | **1.51x** |
| 32 | 4,733 | 7,348 | **1.55x** |

### 5.2 Results — File Stat (ops/sec)

| np | NFS (mean) | RucksFS (mean) | RucksFS / NFS |
|:---:|---:|---:|:---:|
| 1 | 1,279 | 3,176 | **2.48x** |
| 2 | 1,709 | 6,094 | **3.57x** |
| 4 | 2,767 | 11,552 | **4.17x** |
| 8 | 4,555 | 18,621 | **4.09x** |
| 16 | 6,608 | 25,778 | **3.90x** |
| 32 | 7,102 | 34,210 | **4.82x** |

### 5.3 Results — File Removal (ops/sec)

| np | NFS (mean) | RucksFS (mean) | RucksFS / NFS |
|:---:|---:|---:|:---:|
| 1 | 479 | 805 | **1.68x** |
| 2 | 860 | 1,560 | **1.81x** |
| 4 | 1,278 | 2,952 | **2.31x** |
| 8 | 2,176 | 5,058 | **2.32x** |
| 16 | 3,433 | 7,523 | **2.19x** |
| 32 | 4,387 | 10,376 | **2.36x** |

### 5.4 Scaling Efficiency (np=1 → np=32)

| Filesystem | Create | Stat | Remove |
|:---:|:---:|:---:|:---:|
| NFS | 4,733 / 462 = **10.2x** | 7,102 / 1,279 = **5.6x** | 4,387 / 479 = **9.2x** |
| RucksFS | 7,348 / 630 = **11.7x** | 34,210 / 3,176 = **10.8x** | 10,376 / 805 = **12.9x** |

### 5.5 Analysis

**RucksFS consistently outperforms NFS across all concurrency levels and all operations:**

1. **File Creation (1.4x–1.7x):** RucksFS benefits from RocksDB's LSM-Tree write path: a create operation appends to the WAL (sequential I/O) and inserts into a memtable (in-memory). NFS+ext4 must update the on-disk B-tree directory entry, allocate an inode, and write the journal — involving multiple synchronous I/Os even on SSD.

2. **File Stat (2.5x–4.8x):** The largest advantage. RucksFS stat resolves to a point query on RocksDB (inode CF), which benefits from bloom filters and block cache. NFS stat (with `noac`) requires a full network round-trip plus ext4 inode lookup. The advantage grows with concurrency because RocksDB's read path is lock-free (concurrent memtable reads), while ext4 has per-inode locking overhead.

3. **File Removal (1.7x–2.4x):** Similar to creation — RocksDB's atomic write batch (WAL + memtable) is faster than ext4's journal-based unlink which must update the directory, deallocate the inode, and sync the journal.

4. **Scaling:** Both systems scale approximately linearly, but RucksFS has better stat scaling (10.8x vs 5.6x from np=1 to np=32) due to RocksDB's lock-free read architecture. NFS stat scaling flattens because each stat requires a synchronous GETATTR RPC.

---

## 6. Comparison with v1 Results

| Metric | v1 (3-machine, shared) | v2 (2-server, dedicated) | Δ |
|--------|------------------------|--------------------------|---|
| NFS server | Shared with DataServer | **Dedicated Server-2** | Fixed |
| RucksFS MDS | Dedicated Meta machine | **Dedicated Server-1** (MDS+DS) | Symmetric |
| NFS threads | 64 (fixed) | **16 (scanned optimal)** | More rigorous |
| Test execution | Interleaved per np | **Serial** (NFS all → RucksFS all) | No cross-contamination |
| Warmup | None | **100-file throwaway** | Fairer |
| NFS np=32 create | 4,214 | **4,733** (+12%) | Dedicated server helps NFS |
| RFS np=32 create | 7,651 | **7,348** (−4%) | Sharing MDS+DS has small cost |
| Create ratio (np=32) | 1.82x | **1.55x** | NFS closer under fair conditions |
| Stat ratio (np=32) | 5.02x | **4.82x** | Consistent |
| Remove ratio (np=32) | 2.56x | **2.36x** | NFS closer under fair conditions |

**Key observation:** Under fully symmetric conditions, NFS performs 12% better at np=32 create (dedicated server vs shared), closing the gap from 1.82x to 1.55x. The stat advantage remains dominant at ~4x. **The v2 results are more conservative and more credible.**

---

## 7. Discussion

### 7.1 Why RucksFS Outperforms NFS

The performance advantage stems from architectural differences at two levels:

**1. Metadata engine: LSM-Tree vs B-tree+Journal**

RocksDB's LSM-Tree converts random metadata writes into sequential WAL appends + in-memory memtable inserts. ext4's metadata path requires updating an on-disk B-tree (directory entry), allocating from the inode bitmap, and writing a journal entry — multiple synchronous I/Os even on SSD. For reads, RocksDB's bloom-filter-accelerated point queries outperform ext4's B-tree traversal.

**2. Concurrency model: async I/O multiplexing vs synchronous threads**

RucksFS uses tokio's async runtime (8 worker threads handling thousands of concurrent gRPC requests via I/O multiplexing). NFS uses synchronous nfsd kernel threads (each thread blocks on one request at a time). The thread scan confirms that increasing threads beyond 16 provides zero benefit — the bottleneck is ext4 metadata serialization, not NFS thread count.

### 7.2 FUSE Overhead

Both RucksFS and NFS have kernel→user→kernel transition overhead:
- **RucksFS**: User process → kernel VFS → FUSE → user FUSE daemon → gRPC → remote server
- **NFS**: User process → kernel VFS → NFS client (kernel) → TCP → remote NFS server (kernel)

NFS has the advantage of staying in kernel space on the client side, while RucksFS pays the FUSE context-switch penalty. Despite this handicap, RucksFS still outperforms NFS because the server-side advantage (RocksDB vs ext4) more than compensates.

### 7.3 Limitations

1. **Single client node:** Multi-client scaling not tested.
2. **Metadata only:** Data I/O throughput (large file read/write) not compared.
3. **Cold start only:** All tests run with cleared caches. Warm-cache behavior may differ.
4. **SSD-only:** On HDD, the LSM-Tree advantage would be larger.
5. **gRPC overhead:** RucksFS uses multiple sequential RPCs per create. Optimizing to fewer RPCs could further improve performance.
6. **NFS `sync` export:** Using `async` export would improve NFS write performance at the cost of durability. However, RocksDB also uses synchronous WAL writes, so `sync` is the fair comparison.

### 7.4 Threats to Validity

1. **Network asymmetry:** Client→Server-2 (NFS) has 12% more bandwidth and 12% higher RTT than Client→Server-1 (RucksFS). For metadata workloads, these differences are negligible but not zero.
2. **Co-located MDS+DS:** RucksFS runs both MetadataServer and DataServer on Server-1, which could cause resource contention. However, mdtest tests are metadata-only (no data I/O), so the DataServer is idle during tests.

---

## 8. Conclusion

Under rigorously controlled conditions — identical dedicated hardware per filesystem (8C16G), identical network (VPC, <0.2ms RTT), NFS thread count tuned to measured optimum (16), attribute cache disabled (`noac`), warmup, serial execution, and server-side cleanup — **RucksFS consistently outperforms NFS:**

| Operation | Advantage Range | Typical (np=32) |
|-----------|:---:|:---:|
| **File Creation** | 1.4x – 1.7x | **1.55x** |
| **File Stat** | 2.5x – 4.8x | **4.82x** |
| **File Removal** | 1.7x – 2.4x | **2.36x** |

The advantage is attributable to:
- **RocksDB's LSM-Tree write path** vs ext4's journal-based write path (1.5x for create/remove)
- **RocksDB's bloom-filter-accelerated reads** vs ext4's B-tree inode lookup (~4x for stat)
- **tokio's async I/O multiplexing** vs nfsd's synchronous thread model (better scaling under load)

These results confirm that using an LSM-Tree-based KV store (RocksDB) as the metadata engine provides a meaningful performance advantage over traditional filesystem metadata management (ext4+NFS), even after accounting for the additional FUSE and gRPC protocol overhead in the RucksFS architecture.

---

## Appendix A: Raw Data Location

All raw mdtest output files are stored in:
```
testing/results/controlled_v2/
├── environment.txt
├── optimal_nfsd_threads.txt
├── exp1_network/
│   ├── ping.txt
│   └── iperf3.txt
├── exp1.5_thread_scan/
│   ├── nfsd_t{8,16,32,64}_run{1,2,3}.txt   (12 files)
│   └── summary.txt
└── exp2_scaling/
    ├── nfs_np{1,2,4,8,16,32}_run{1,2,3}.txt     (18 files)
    └── rucksfs_np{1,2,4,8,16,32}_run{1,2,3}.txt  (18 files)
```

## Appendix B: Full Experiment 2 Raw Data

### File Creation (ops/sec, max of 3 mdtest iterations per run)

| np | NFS Run1 | NFS Run2 | NFS Run3 | NFS Mean | RFS Run1 | RFS Run2 | RFS Run3 | RFS Mean | Ratio |
|:---:|---:|---:|---:|---:|---:|---:|---:|---:|:---:|
| 1 | 462.4 | 464.1 | 460.0 | 462.2 | 636.1 | 625.2 | 628.2 | 629.8 | 1.36x |
| 2 | 855.7 | 860.1 | 850.6 | 855.5 | 1231.9 | 1237.9 | 1237.8 | 1235.9 | 1.44x |
| 4 | 1344.1 | 1340.0 | 1336.2 | 1340.1 | 2305.3 | 2322.0 | 2310.5 | 2312.6 | 1.73x |
| 8 | 2396.3 | 2381.0 | 2387.5 | 2388.3 | 3755.5 | 3779.6 | 3776.1 | 3770.4 | 1.58x |
| 16 | 3581.2 | 3567.0 | 3580.4 | 3576.2 | 5408.5 | 5398.2 | 5386.4 | 5397.7 | 1.51x |
| 32 | 4742.6 | 4731.3 | 4724.9 | 4732.9 | 7344.3 | 7345.9 | 7353.6 | 7347.9 | 1.55x |

### File Stat (ops/sec, max of 3 mdtest iterations per run)

| np | NFS Run1 | NFS Run2 | NFS Run3 | NFS Mean | RFS Run1 | RFS Run2 | RFS Run3 | RFS Mean | Ratio |
|:---:|---:|---:|---:|---:|---:|---:|---:|---:|:---:|
| 1 | 1281.2 | 1277.8 | 1278.0 | 1279.0 | 3167.5 | 3173.8 | 3187.7 | 3176.3 | 2.48x |
| 2 | 1710.3 | 1708.6 | 1707.5 | 1708.8 | 6083.9 | 6099.2 | 6098.0 | 6093.7 | 3.57x |
| 4 | 2773.0 | 2764.5 | 2763.8 | 2767.1 | 11536.2 | 11560.1 | 11560.4 | 11552.2 | 4.17x |
| 8 | 4566.3 | 4547.2 | 4551.4 | 4554.9 | 18559.8 | 18634.8 | 18669.3 | 18621.3 | 4.09x |
| 16 | 6617.1 | 6612.0 | 6595.9 | 6608.3 | 25796.2 | 25755.4 | 25783.7 | 25778.4 | 3.90x |
| 32 | 7112.4 | 7097.5 | 7097.4 | 7102.4 | 34117.2 | 34244.4 | 34269.3 | 34210.3 | 4.82x |

### File Removal (ops/sec, max of 3 mdtest iterations per run)

| np | NFS Run1 | NFS Run2 | NFS Run3 | NFS Mean | RFS Run1 | RFS Run2 | RFS Run3 | RFS Mean | Ratio |
|:---:|---:|---:|---:|---:|---:|---:|---:|---:|:---:|
| 1 | 478.5 | 479.9 | 478.6 | 479.0 | 810.2 | 801.6 | 803.2 | 805.0 | 1.68x |
| 2 | 862.3 | 858.3 | 858.7 | 859.8 | 1562.8 | 1558.5 | 1559.2 | 1560.2 | 1.81x |
| 4 | 1280.4 | 1276.5 | 1277.1 | 1278.0 | 2951.8 | 2948.6 | 2954.0 | 2951.5 | 2.31x |
| 8 | 2178.3 | 2174.1 | 2176.5 | 2176.3 | 5062.4 | 5067.0 | 5044.3 | 5057.9 | 2.32x |
| 16 | 3441.3 | 3425.5 | 3432.9 | 3433.2 | 7524.1 | 7498.6 | 7544.9 | 7522.5 | 2.19x |
| 32 | 4395.2 | 4383.1 | 4382.3 | 4386.9 | 10437.9 | 10344.6 | 10344.4 | 10375.6 | 2.36x |

## Appendix C: Benchmark Scripts

All benchmark scripts used in this experiment are stored in:
```
scripts/benchmark/v2/
├── bench_common.sh          # Shared functions (cache clearing, mdtest wrapper, etc.)
├── bench_network.sh         # Experiment 1: Network symmetry verification
├── bench_thread_scan.sh     # Experiment 1.5: NFS thread count scan
├── bench_nfs.sh             # Experiment 2A: NFS concurrency scaling
└── bench_rucksfs.sh         # Experiment 2B: RucksFS concurrency scaling
```

Scripts were executed sequentially by the operator, with manual verification of cleanup between phases.
