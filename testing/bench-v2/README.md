# bench-v2 历史数据（已停用）

本目录下的 `results-v2-n{2,8,32,64,96}/` 是 2026 年 4 月中下旬完成的第一轮完整基准测试，其中 **RucksFS FUSE 客户端的 entry/attr TTL 被设为 1 秒**、**JuiceFS 客户端默认启用 1 秒的 attr/entry cache**、**NFS 使用 noac 关闭客户端 attr cache**。

这批数据的性质是"**含客户端 VFS 缓存的端到端性能**"——`stat` 测量值一部分来自内核 VFS 缓存命中（不到达 MDS），不完全反映 MDS 本身的处理能力。

## 为什么不再使用

论文的研究对象是**元数据键值存储下的文件操作**，测量对象是 MDS。客户端 VFS 缓存命中时请求根本不进入 MDS，对测量 MDS 处理能力没有贡献。再加上三套系统的客户端缓存配置不对等（RucksFS 与 JuiceFS 各自有 1s TTL，NFS 被 noac 关闭），导致 `stat` 数据不具备跨系统可比性。

因此在 `docs/thesis-template/BENCHMARK_RATIONALE.md` 里确定的第二轮测试方案把三套系统的客户端缓存**全部关闭**，以统一到"MDS 纯路径性能"的测量口径。第二轮数据放在 `testing/bench-v3/` 目录。

## 本目录数据的保留意义

- 作为历史留档，便于追溯第一轮实验的原始结果
- 与第二轮数据形成对照，能展示"开启客户端缓存后能获得多大的端到端收益"（如果将来需要补充端到端性能章节）
- `orchestrator.sh` 是第二轮 orchestrator 脚本的基础版本

## 数据索引

| 并发 | 目录 | 包含被测系统 |
|---|---|---|
| N=2  | `results-v2-n2/`  | rucksfs-delta, rucksfs-nodelta, nfs, juicefs-tikv |
| N=8  | `results-v2-n8/`  | 同上 |
| N=32 | `results-v2-n32/` | 同上 |
| N=64 | `results-v2-n64/` | rucksfs-delta, rucksfs-nodelta, nfs, juicefs-tikv (hard 模式缺失) |
| N=96 | `results-v2-n96/` | rucksfs-delta, rucksfs-nodelta, nfs |

每个目录下的 `summary.csv` 是该并发档位的汇总结果。

## 请勿继续基于本目录产出论文数据

论文正文中出现的 ops/s 数字、倍数、表格均应取自 `testing/bench-v3/`。
