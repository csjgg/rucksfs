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
