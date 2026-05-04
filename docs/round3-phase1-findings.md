# Round 3 Phase 1 Findings — Delta vs No-Delta on 6-client Cluster

Date: 2026-04-25
Cluster: Tencent Cloud HK (ap-hongkong-2), 1 server + 6 clients, SA5.2XLARGE16 (8C16G).
Test: mdtest-hard (`-F -C -T -r`, shared parent dir), files/rank = 2000..800 (reduced at N≥96), 3 runs/point.

## Summary — Breakout FAILED

**Expected**: delta/nodelta file-creation ratio ≥ 1.5× at N=64, ≥ 2.0× at N=128.
**Observed**: ratio ≈ **1.00 at every N**. Delta mechanism provides no measurable benefit.

Per the user's standing rule — *"如果跑出来没效果，我们得再优化一下，先不要着急跑后面的"* — Phases 2-4 (NFS, JuiceFS+Redis, JuiceFS+TiKV) were **not run**. Cluster destroyed (2026-04-25 04:42 UTC+8) to stop cost.

## Raw results (File creation ops/sec, 3 runs per N)

| N   | rucksfs-delta           | rucksfs-nodelta         | ratio (delta/nodelta) |
|-----|-------------------------|-------------------------|-----------------------|
| 8   | 1048.7 / 1040.6 / 1053.5 | 1037.5 / 1037.5 / 1051.2 | 1.01 |
| 16  | 2015.9 / 2015.6 / 2012.7 | 2015.9 / 2010.1 / 2012.5 | 1.00 |
| 32  | 3951.3 / 3948.8 / 3951.4 | 3942.7 / 3932.1 / 3923.4 | 1.01 |
| 64  | 5676.7 / 5669.8 / 5664.4 | 4405.5\* / 5667.9 / 5660.9 | 1.00 (after skipping cold-run) |
| 96  | 5820.3 / 5803.3 / 5786.3 | 5794.6 / 5804.6 / 5794.8 | 1.00 |
| 128 | 5663.7 / 5658.8 / 5659.5 | 5647.5 / 5630.5 / 5642.0 | 1.00 |
| 192 | 5774.5 / 5797.3 / 5794.4 | 5766.0 / 5768.9 / 5778.8 | 1.00 |

\* nodelta N=64 run1 is a cold-mount outlier. Runs 2 and 3 match delta exactly.

Both curves are linear up to N=64 (~90 ops/sec per added rank) then saturate hard at ~5800 ops/s.

Raw mdtest output is in `testing/round3_results/phase1/`.

## Why no delta advantage — server bottleneck analysis

Server snapshot taken mid-test (no-delta, N=128):
- CPU: metaserver ≈ **112 %** (~1.1 cores of 8). Dataserver ~10%.
- Memory: 880 Mi / 15 Gi used — no pressure.
- Disk (cloud SSD vdb): essentially idle after RocksDB absorbs writes into memtable. `%util ≈ 0`.
- Accumulated: metaserver used 22:55 of CPU time across the whole 20-min no-delta matrix — pinned to ~100 % of a single core.

**The bottleneck is single-thread contention inside the metaserver process**, not CPU count, not disk, not network. With 192 concurrent client ranks hitting a handler that serializes on a single mutex/RocksDB write path, both variants are stuck at the same ~5800 ops/s ceiling. Delta's goal — fewer bytes and fewer lookups per create — only pays off when the server handler is throughput-limited by per-op work, not by a global lock.

This reconciles with our earlier localhost findings (see `project_delta_scaling_analysis.md`): the delta/no-delta gap only showed at T ≥ 32 on the *server microbench* because that bench hit the handler directly. Add a real gRPC transport + filesystem coherency layer and the mutex inside the handler becomes the dominant contention source before the delta savings can surface.

## Candidate optimizations before re-running Phase 1

Ranked by likely impact / effort:

1. **Identify the serializing critical section in metaserver.** Top candidates:
   - Single `Mutex<RocksDB>` or similar around batch writes.
   - `WriteBatchWithIndex` that holds an exclusive lock for the whole create path.
   - A global inode-allocation counter.
   Instrument with `tracing` spans + Tokio-console or a flamegraph of `rucksfs-metaserver` under load.

2. **Parallelize write path** — shard RocksDB writes by parent-inode hash, or use RocksDB's `WriteBatch` with `disable_wal` + group commit. Delta becomes effective only after the handler can actually dispatch > N work units concurrently.

3. **Remove redundant work in create path** — even before delta, e.g. coalesce the per-create RPC into `create_and_open` (already done, commit 2028e32) but double-check that both variants truly take the fast path.

4. **Reconsider test design** — if we insist on shared-parent contention semantics, a single-metadata-server design is *inherently* serialized. The realistic comparison is against JuiceFS+Redis, which shards keys across Redis slots. We may be measuring an architectural choice rather than a delta-vs-nodelta gap.

## What was spent

Rough Tencent Cloud HK cost: 7 instances × ~1.5 h × ¥1.79/h ≈ **¥19**. Well under the Phase 1 budget.

## Status of the plan

- Phase 0 (pool + Terraform + scripts): ✅
- Phase 1 (rucksfs-delta + rucksfs-nodelta matrix): ✅ data collected, breakout failed
- Phase 2 (NFS): ⛔ skipped per breakout rule
- Phase 3 (JuiceFS+Redis): ⛔ skipped
- Phase 4 (JuiceFS+TiKV): ⛔ skipped
- Phase 5 (destroy): ✅

Ready for user review and direction on which optimization to pursue next.
