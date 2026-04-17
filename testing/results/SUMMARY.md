# Benchmark Results Summary

## Experiment 1: Fill stat/remove N=4 and N=32 (mdtest, -n 5000, -u)

All data: 3 runs averaged, local FUSE mount on client1.

### stat (ops/s)
| N | RucksFS | JuiceFS+MySQL | JuiceFS+TiKV | NFS |
|---|---------|---------------|--------------|-----|
| 4 | 1,033,401 | 7,216 | 7,874 | 68,401 |
| 32 | 139,916 | 28,428 | 34,106 | 59,568 |

### remove (ops/s)
| N | RucksFS | JuiceFS+MySQL | JuiceFS+TiKV | NFS |
|---|---------|---------------|--------------|-----|
| 4 | 23,529 | 660 | 1,095 | 3,047 |
| 32 | 28,903 | 2,721 | 3,205 | 5,141 |

### create (ops/s) — bonus data
| N | RucksFS | JuiceFS+MySQL | JuiceFS+TiKV | NFS |
|---|---------|---------------|--------------|-----|
| 4 | 19,591 | 818 | 1,315 | 3,207 |
| 32 | 32,953 | 3,499 | 4,797 | 5,643 |

**Note**: RucksFS numbers are significantly higher than the thesis's
original distributed-mode data because this test uses local FUSE mount
(no gRPC overhead). The other systems still go over the network.

## Experiment 2: Directory scale (N=16, vary -n per process)

### RucksFS
| -n | create | stat | remove |
|----|--------|------|--------|
| 100 | 23,660 | 720,358 | 16,984 |
| 1,000 | 32,164 | 1,044,610 | 39,469 |
| 5,000 | 32,637 | 166,439 | 33,453 |
| 10,000 | 32,225 | 139,373 | 31,998 |

### JuiceFS+TiKV
| -n | create | stat | remove |
|----|--------|------|--------|
| 100 | 3,144 | 493,020 | 2,685 |
| 1,000 | 3,262 | 21,381 | 2,372 |
| 5,000 | 3,225 | 21,717 | 2,331 |
| 10,000 | 3,188 | 21,730 | 2,324 |

### NFS
| -n | create | stat | remove |
|----|--------|------|--------|
| 100 | 4,944 | 846,706 | 4,871 |
| 1,000 | 5,622 | 587,689 | 5,280 |
| 5,000 | 5,593 | 61,246 | 5,181 |
| 10,000 | 5,615 | 52,733 | 5,186 |

**Key finding**: RucksFS create/remove throughput is stable across
directory sizes (32K ops/s regardless of -n). stat drops from 1M to
140K as -n grows due to cache effects, but remains 6.4x higher than
JuiceFS+TiKV at -n=10000.
