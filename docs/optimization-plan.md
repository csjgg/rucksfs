# RucksFS Iterative Optimization Plan v2

## Core Philosophy: Measure First, Optimize Second

之前的 mdtest 测的是 FUSE 全链路，混杂了 FUSE 开销。**无法区分瓶颈在哪一层。**

新策略：**先写一个绕过 FUSE 的 microbench 工具**，直接调用各层 API，分层压测，定位真正的瓶颈后再针对性优化。

## Phase 0: Microbench Tool (最高优先级)

### 0.1 设计

一个 Rust binary `rucksfs-bench`，直接调用 trait 接口，跳过 FUSE 层。

**三层测试模式：**

```
┌─────────────────────────────────────────────────────────┐
│ Layer 1: 本地元数据 (MetadataServer 直接调用)            │
│   - 直接构造 MetadataServer，操作本地 RocksDB            │
│   - 测出元数据引擎的裸性能天花板                          │
│                                                         │
│ Layer 2: gRPC 元数据 (MetadataRpcClient 远程调用)        │
│   - 通过网络调用远端 MetadataServer                      │
│   - Layer1 - Layer2 差值 = gRPC + 网络开销                │
│                                                         │
│ Layer 3: 完整 VfsOps (VfsCore 远程调用)                  │
│   - 通过 VfsCore 调用，走完整的元数据+数据协调路径        │
│   - Layer2 - Layer3 差值 = VfsCore 协调开销               │
│                                                         │
│ (对比) FUSE 路径: mdtest 已有数据                        │
│   - Layer3 vs mdtest 差值 = FUSE 层开销                  │
└─────────────────────────────────────────────────────────┘
```

**测试操作：**

| 操作 | 方法 | 说明 |
|------|------|------|
| create | `create(parent, name, 0o644, 0, 0)` | 文件创建 |
| stat | `getattr(inode)` | 属性读取 |
| lookup | `lookup(parent, name)` | 目录查找 |
| unlink | `unlink(parent, name)` | 文件删除 |
| mkdir | `mkdir(parent, name, 0o755, 0, 0)` | 目录创建 |
| readdir | `readdir(parent)` | 目录列举 |

**并发模式：**
- 线程数: 1, 2, 4, 8, 16
- 每线程操作数: N (可配置，默认 10000)
- 使用 tokio 异步任务并发（不是 OS 线程），更贴近真实 gRPC 使用模式

**输出格式：**
```
=== Layer 1: Local MetadataServer ===
Op          Threads   Total ops    ops/s      P50(us)  P99(us)
create      1         10000        52,341     18.2     45.1
create      4         10000        198,223    19.5     62.3
create      16        10000        412,105    37.8     125.6
stat        1         10000        285,000    3.2      8.1
...

=== Layer 2: gRPC MetadataRpcClient ===
Op          Threads   Total ops    ops/s      P50(us)  P99(us)
create      1         10000        3,125      310.5    520.1
create      4         10000        11,200     345.2    890.3
...

=== Overhead Analysis ===
Op          Local       gRPC        FUSE        gRPC overhead   FUSE overhead
create      52,341      3,125       639         94.0%           79.5%
stat        285,000     18,200      6,387       93.6%           64.9%
```

### 0.2 实现要点

```rust
// demo/src/bin/bench.rs
// 直接构造各层实例，不走 FUSE

// Layer 1: 本地
let storage = RocksStorage::open(temp_dir)?;
let meta = MetadataServer::new(storage);
bench_ops(&meta, threads, ops_per_thread).await;

// Layer 2: gRPC (需要远端 metaserver 已启动)
let meta_client = MetadataRpcClient::connect(meta_addr).await?;
bench_ops(&meta_client, threads, ops_per_thread).await;

// Layer 3: VfsCore (需要远端 metaserver + dataserver 已启动)
let vfs = VfsCore::new(meta_client, data_client);
bench_ops(&vfs, threads, ops_per_thread).await;
```

关键：`bench_ops` 泛型接受 `MetadataOps`（Layer 1/2）或 `VfsOps`（Layer 3），统一压测逻辑。

### 0.3 预期产出

跑完 microbench 后，我们能精确回答：

1. **元数据层天花板多少？** Layer 1 单线程 create ops/s = RocksDB + 元数据逻辑的极限
2. **gRPC 加了多少开销？** Layer 1 vs Layer 2 的差值
3. **FUSE 加了多少开销？** Layer 3 vs mdtest 的差值
4. **哪个操作是瓶颈？** 比较 create/stat/unlink 的绝对数值
5. **并发 scaling 如何？** 每层的 1→4→16 线程扩展比
6. **延迟分布？** P50/P99 能暴露长尾问题（比如 delta compaction 触发时的 spike）

---

## Phase 1: 数据驱动优化（基于 Phase 0 结果）

Phase 0 跑完后，根据数据选择优化方向。以下是**候选优化项**，按 microbench 结果来决定优先级：

### 场景 A: 如果 Layer 1 天花板就不高（< 20K create/s）

**说明元数据引擎本身是瓶颈**，优化重点在 RocksDB 写路径：

| 优化项 | 做法 | 预期效果 |
|--------|------|---------|
| WAL sync 关闭 | `write_opts.set_sync(false)` | Create +30-50% |
| 合并 parent delta | 4 个 delta put → 1 个 compound delta | Create +10-15% |
| Bloom filter | metadata CF 加 10-bit bloom | Stat/Lookup +20% |
| WriteBatch 优化 | 减少序列化开销 | Create +5-10% |
| Memtable 调优 | 增大 write_buffer_size, 多 memtable | Write throughput ↑ |

### 场景 B: 如果 Layer 1 很高但 Layer 2 骤降（gRPC 开销 > 80%）

**说明网络/序列化是瓶颈**，优化重点在 gRPC 层：

| 优化项 | 做法 | 预期效果 |
|--------|------|---------|
| 多连接 | 多个 gRPC channel round-robin | 并发吞吐 ↑ |
| 请求批量化 | 攒一批请求，一次 RPC 发送 | 摊薄 per-RPC overhead |
| 协议优化 | 减少 protobuf 字段 / 用 flatbuffers | 序列化开销 ↓ |
| TCP 调优 | Nagle off, TCP_NODELAY | 延迟 ↓ |

### 场景 C: 如果 Layer 2 不错但 mdtest 骤降（FUSE 开销 > 70%）

**说明 FUSE 层是瓶颈**，优化重点在 FUSE 并发：

| 优化项 | 做法 | 预期效果 |
|--------|------|---------|
| FUSE 多线程 | 多线程 /dev/fuse 读取 | Scaling 1.1x → 3x+ |
| 属性缓存 | 客户端缓存 getattr | Stat 2-4x |
| Readdir 预取 | readdir 时预取子项属性 | ls -l ↑ |

### 场景 D: 混合瓶颈

**每层都有优化空间**，按照投入产出比排序执行。

---

## Phase 2: 迭代优化循环

对每个候选优化项，严格执行：

```
1. Baseline: 跑 rucksfs-bench，记录各操作 ops/s + P50/P99
2. Code change: 实施一个优化（最小改动）
3. Re-bench: 再跑 rucksfs-bench，同样参数
4. Compare:
   - 改进 > 5%  → KEEP，提交代码，更新 baseline
   - 改进 0-5%  → 看代码复杂度，简单则保留，复杂则回滚
   - 回退        → 立即 git revert
5. 重复直到达成目标或投入产出比不合理
```

每轮优化后都**重跑 mdtest**，确认 microbench 改进能传导到 FUSE 全链路。

---

## 当前 Baseline（mdtest，待 microbench 补充）

| Metric | rucksfs-embedded | rucksfs-dist | NFS+ext4 | JuiceFS-Redis |
|--------|-----------------|-------------|----------|---------------|
| Create (np=1) | 9,403 | 639 | 774 | 1,237 |
| Stat (np=1) | 237,129 | 6,387 | 25,992 | 7,453 |
| Remove (np=1) | 13,511 | 889 | 869 | 1,148 |
| Create scaling (1→4) | 9,298→23,271 | 636→697 | 794→2,171 | 1,227→3,978 |

**注意**：embedded 模式 create 9,403 说明 MetadataServer + RocksDB 本身就有不错的性能（这还包含了 FUSE 开销！），所以 Layer 1 天花板应该远高于此。分布式模式 639 骤降到 embedded 的 6.8%，说明大部分开销在 gRPC + 网络 + FUSE 串行化。

---

## 目标

| Metric | Current (dist) | Target | Stretch |
|--------|---------------|--------|---------|
| Create (np=1) | 639 | 2,000 | 5,000 |
| Stat (np=1) | 6,387 | 20,000 | 50,000 |
| Remove (np=1) | 889 | 2,000 | 5,000 |
| Create scaling (np=1→4) | 1.1x | 3.0x | Near-linear |

对比目标：
- 全面超过 NFS+ext4
- 全面超过 JuiceFS-Redis
- Scaling 接近线性

---

## 执行计划

| 顺序 | 任务 | 工时 | 产出 |
|------|------|------|------|
| **0** | **写 rucksfs-bench 工具** | **3-4h** | **分层性能数据** |
| 1 | 跑 microbench，生成各层 baseline | 1h | 数据驱动的优化优先级 |
| 2 | 根据数据选择 Top-3 优化项 | - | 确定方向 |
| 3-N | 迭代优化循环（每项 1-4h） | 每项 1-4h | 逐步提升 |
| Final | 全量 mdtest 回归 | 2h | 最终效果确认 |
