# RucksFS Performance Optimization Log

> Tracks each optimization round: target, approach, result, decision.

## Baseline — 2026-03-06

| Operation | 1T easy ops/s | 2T easy ops/s | 4T easy ops/s |
|-----------|--------------|--------------|--------------|
| create    | 17,082       | 4.86         | 4.96         |
| stat      | 854,489      | 968,143      | 1,580,428    |
| unlink    | 31.82        | 32.10        | 6.85         |
| mkdir     | 13,257       | 28,378       | 46,734       |
| rmdir     | 19,452       | 45,512       | 46,496       |
| readdir   | 9,008        | 17,664       | 23,207       |
| rename    | 20,904       | 40,819       | 41,420       |

**Notes:**
- `create` easy 2T/4T show catastrophic regression (~5 ops/s) — likely RocksDB write contention or transaction deadlock at scale
- `unlink` is extremely slow across all configurations (~28-32 ops/s 1T) — deferred delete dominates
- `readdir` actually exceeds ext4 baseline (9,008 vs 6,300) — already well-optimized
- `stat` is close to ext4 (854K vs 1.1M) — mostly irreducible FUSE overhead

---

## Round 1 — 2026-03-06 — RocksDB Block Cache

- **Target**: all operations (infrastructure-level)
- **Bottleneck**: block_cache at default 8MB, no index/filter caching
- **Optimization**: 256MB shared LRU block cache, pin L0 filter/index, cache_index_and_filter_blocks
- **Branch**: opt/round-1-rocksdb-block-cache
- **Result**:
  - create: 17,082 → 11,042 ops/s (**-35.4%** regression)
  - stat: 854,489 → 664,033 ops/s (**-22.3%** regression)
  - rename: 20,904 → 15,286 ops/s (**-26.9%** regression)
  - unlink: 31.82 → 31.41 ops/s (-1.3%)
  - mkdir: 13,257 → 13,006 ops/s (-1.9%)
  - readdir: 9,008 → 12,272 ops/s (+36.2% improvement)
  - rmdir: 19,452 → 19,292 ops/s (-0.8%)
- **Analysis**: Block cache overhead outweighs benefit at small working set (-n 100). Cache management cost (LRU bookkeeping, cache_index_and_filter) adds latency to fast operations. Readdir benefits from cached prefix scan blocks.
- **Decision**: REVERTED
- **Baseline updated**: no

---
