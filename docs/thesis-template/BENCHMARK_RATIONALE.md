# 性能测试的目的与方案（BENCHMARK_RATIONALE）

本文件是性能实验章节的"意图文档"，记录本次重测的目的、每组对比的定位，以及具体的测试方案。后续无论论文怎么改、数据怎么变，都应当以本文件为准回看"我们当初为什么这样测"。写作时如果某段文字和本文件冲突，以本文件为准。

本文件仅讨论性能测试，不涉及 POSIX 合规性测试。

---

## 一、测量对象与测量范围

论文研究对象是**元数据键值存储下的文件操作**。具体落到系统上，测量对象是 RucksFS 的 MetadataServer（MDS）——它承担目录项编码、inode 属性维护、事务提交、DeltaOp 增量更新等全部元数据逻辑。

因此本章的"性能"一词严格指 MDS 的处理能力：一次元数据操作从 FUSE 请求进入到服务端事务提交再返回这条路径上的真实延迟和吞吐。

### 客户端缓存的处理

三套被测系统的**客户端属性/目录项缓存全部关闭**：

- **RucksFS**：FUSE 返回的 entry/attr TTL 设为 0
- **JuiceFS**：`--attr-cache=0 --entry-cache=0 --dir-entry-cache=0 --open-cache=0`
- **NFS**：`noac` 挂载选项

理由：内核 VFS 在 FUSE/NFS 客户端返回 TTL 之后会在本地维护一份属性缓存。命中时请求根本不到达 MDS，对测量 MDS 处理能力没有贡献。**三家全部关**就避免了"配置不对等"和"测的不是 MDS"这两个问题同时存在。

### 服务端缓存的处理

保留默认配置，由测试正常触发。包括：

- RucksFS MDS 的 `InodeFoldedCache`（16 片 LRU，容量 10,000）
- RucksFS 的 RocksDB BlockCache
- TiKV 内部的 BlockCache
- ext4 的 page cache（每轮 mdtest 前 drop_caches 清一次，减少跨轮污染）

理由：这些缓存都是被测系统设计的一部分，属于 MDS 内部的实现选择。

---

## 二、论述结构：两段式

本章性能部分按两段式组织。第一部分讲横向对比，第二部分讲 DeltaOp 专项。这种组织方式比"三组对比并列"更贴合论文的叙事：第一部分先把"RucksFS 在三方里的位置"讲清楚，第二部分再解释"DeltaOp 作为一个专项优化在哪里起作用"。

### 第一部分：横向性能对比（中低并发）

- **并发范围**：$N \in \{2, 8, 32\}$
- **对比对象**：RucksFS-Delta、RucksFS-NoDelta、JuiceFS+TiKV、NFS
- **核心目的**：把 RucksFS 放到 FUSE+KV 和本地 FS+NFS 两条常见工程路线之间，证明它处在什么位置
- **NoDelta 在此处的角色**：仅作为 DeltaOp 消融参考出现，顺带说明"DeltaOp 在低并发下不引入额外开销"；**不作为独立的被测系统宣传**

### 第二部分：DeltaOp 专项对比（高并发）

- **并发范围**：$N \in \{64, 96\}$
- **对比对象**：RucksFS-Delta、RucksFS-NoDelta
- **核心目的**：单独考察 DeltaOp 在共享父目录高并发场景下的作用
- **不在此档位与 JuiceFS/NFS 对比**：两者在该区间都无法稳定运行；强行对比会引入锁竞争等 DeltaOp 之外的因素

---

## 三、为什么要测 NFS 和 JuiceFS

三套对比各自承担不同角色，不要混起来讲。

### 3.1 为什么测 JuiceFS+TiKV

**对比目标**：证明 RucksFS 的元数据路径在性能上达到了"基础生产可用的 KV 元数据 FS"水平。

**对比逻辑**：JuiceFS+TiKV 是业界使用较广的"FUSE 客户端 + 通用分布式事务 KV 作为元数据后端"组合。它和 RucksFS 在上层架构上完全对等（都是 FUSE + KV），区别只在**元数据后端是专用的还是通用的**：

- RucksFS：本机直连 RocksDB，针对元数据访问模式定制（多列族、边中心键、DeltaOp、PCC 事务）
- JuiceFS+TiKV：通用分布式事务 KV，走 Percolator 两阶段提交 + Raft

结论方向：**在同样的 FUSE + KV 架构下，针对元数据定制的专用后端（本文）能达到甚至超过通用分布式 KV 后端（JuiceFS）的性能**。

这一组的性能比值不需要很大。只要在关掉客户端缓存的条件下 RucksFS ≥ JuiceFS，就足以支撑"达到生产级水平"这个论断。比值 1.5-3× 是符合预期的、诚实的数字；比值过大反而要警惕是不是实验配置出了问题。

### 3.2 为什么测 NFS

**对比目标**：证明使用 KV 作为元数据存储模型相比传统本地文件系统的目录 B+ 树建模具有优势。

**对比逻辑**：NFS 服务端底层是内核 NFS + ext4。ext4 用 HTree（hashed B-tree）组织目录项，一次 `lookup`/`create` 需要在目录 B+ 树里查找文件名条目。RucksFS 用 RocksDB 点查（边中心键 `parent+name` 大端拼接），目录项以键空间邻接方式存储。

两者在相同"服务化 + 网络 RPC"拓扑下对比（NFS 走内核 NFS 协议，RucksFS 走 gRPC），**主要区别集中在元数据存储模型这一点上**。

**低并发 (N=2)**：不存在锁竞争，差距干净地归因于"KV 点查 vs B+ 树目录查找"、"KV 写入 vs ext4 多区域更新"——这是最具建模层面说服力的数据点。

**中并发 (N=8, 32)**：NFS 受 ext4 目录 `i_rwsem` 锁限制，RucksFS 继续扩展。差距增大，但增大的部分混合了"锁竞争"这一额外因素，论文里需要如实指出。

不测 NFS 高并发（N ≥ 64）。理由：高并发下 NFS 的瓶颈主导是 ext4 目录锁，差距再大也不是对"KV vs B+ 树"这一论点的额外支持。

---

## 四、为什么要对比 Delta 和 NoDelta

**对比目标**：单独衡量 DeltaOp 增量更新机制的作用。

**对比逻辑**：Delta 与 NoDelta 是 RucksFS 同一套代码的两个编译期变体，仅差 `batch_parent_deltas` 一个函数——Delta 把父目录属性更新编码为增量追加到 `delta_entries` 列族；NoDelta 退回传统的"读父 inode → 改 mtime/ctime/nlink → 写回"。其他代码路径（包括 `InodeFoldedCache`、`load_inode`、事务流程）完全一致。

这是一组控制变量极干净的对照。两者之间观察到的任何差距都只能归因于父目录更新路径的不同，不存在其他解释空间。

### 为什么需要高并发才能看到效果

DeltaOp 的设计动机是消除共享父目录下的写锁竞争。这种竞争在低并发下本来就不存在——没有多个事务在同一时刻争抢父 inode 键。因此 DeltaOp 和 RMW 在低并发下没有可观察的差别，两者吞吐应当接近相等。

只有在高并发（共享父目录、大量并发客户端）下，RMW 的等待-回滚-重试循环才会触发，DeltaOp 的追加写路径才显现收益。因此验证 DeltaOp 必须专门在高并发档位进行。

### 为什么不用第三方做这个对比

高并发区间 NFS 被目录锁限制、JuiceFS+TiKV 的单节点部署在连接上也不稳，二者都无法作为对照。所以 DeltaOp 的验证只能依赖 RucksFS 自身的 Delta vs NoDelta 内部对照。

---

## 五、三类操作各自要讲的事

### 5.1 Create

**核心论点**：

- 对 NFS：KV 键值插入路径 vs ext4 多磁盘区域更新路径。低并发就已经有 2-3× 差距。
- 对 JuiceFS：本机直连 RocksDB 一次 `AtomicWriteBatch` vs TiKV 的 Percolator + Raft。低并发约 1.5-3× 差距。
- Delta vs NoDelta 在第一部分（N ≤ 32）：应当接近相等，作为"DeltaOp 在低并发下不引入额外开销"的注脚。
- Delta vs NoDelta 在第二部分（N ∈ {64, 96}）：Delta 显著领先（预期 3-6×），这是 DeltaOp 的主战场。

**数据怎么读**：

Create 是元数据写入路径的核心代表。第一部分的三组数据合起来说明：
1. 写入路径在 KV 方案下比在 B+ 树方案下更适合——来自低并发的 NFS 对比
2. 专用 KV 后端比通用分布式 KV 后端的写入路径更快——来自 JuiceFS 对比

第二部分的数据单独说明：
3. DeltaOp 机制在共享父目录高并发场景下进一步消除了写冲突

这是三个**递进的论点**，不是重复论证。

### 5.2 Remove

**核心论点**：与 Create 对称。`unlink` 同样要更新父目录 mtime/ctime，同样走事务，DeltaOp 机制对它同样适用。

**数据怎么读**：形态和比值应当与 Create 接近，作为 Create 结论的交叉验证。

唯一需要诚实写明的：RucksFS 当前 `delete_data` 是空操作，没有做真正的数据回收；NFS 会触发 ext4 inode 位图翻转 + journal，TiKV 会写 tombstone + 后台 GC。这部分工作量差异在本文空文件负载下被放大，因此 `remove` 的倍数含有一部分"尚未回收数据"带来的虚高。这一点在论文中如实指出。

### 5.3 Stat

Stat 的论述分两层，分别对应第一部分和第二部分。

#### 第一层（在第一部分讲）：RucksFS vs NFS 的 stat 对比

**核心论点**：KV 点查 vs B+ 树目录项查找，在元数据读路径上也具备优势。

**数据怎么读**：

关掉客户端缓存之后，每次 `stat()` 都会下沉到 FUSE/NFS 客户端 → 网络 → 服务端 → 元数据后端的完整路径。

- NFS 服务端：nfsd → ext4 lookup（HTree 查找）→ 读 inode table → 返回
- RucksFS 服务端：gRPC → MDS → RocksDB 点查（大端键 `parent+name`）→ `load_inode` → 返回

RocksDB 点查在命中 MemTable/BlockCache 时是纯内存操作 + Bloom 过滤器兜底，延迟在微秒级；ext4 HTree 查找要经过 B-tree 路径，即使命中 page cache 也要做树层遍历。**这是本文想支撑的"KV 点查快于 B+ 树目录查找"论点**。

预期：低并发下 RucksFS stat 约为 NFS 的 2-3 倍。

第一部分表里 Delta/NoDelta 的 stat 差距应当在 ±5% 以内——说明 `InodeFoldedCache` 命中时消除了折叠成本，cache miss 时折叠成本也被 RocksDB 点查延迟覆盖。

#### 第二层（在第二部分讲）：Delta vs NoDelta 的 stat 对比——防御性论点

**核心论点**：验证 DeltaOp 的折叠机制**没有损害**查询效率。

这是一个**防御性论点**，不是进攻性论点。Delta 方案在读路径上引入了额外工作：

- 每次 `load_inode`：先查 `InodeFoldedCache`，cache miss 时读基值 + 扫描 `delta_entries` + 折叠
- 这比 NoDelta 的"直接读基值"多了折叠这一步

理论上 Delta 的 stat 应该比 NoDelta 慢一些。后台的 `DeltaCompactionWorker` 在增量条目超过阈值（默认 32）时会折叠回基值，把折叠成本摊在后台。本节要回答的问题是：**折叠成本能否被 `InodeFoldedCache` 和后台压缩摊平，让 Delta 的 stat 与 NoDelta 基本持平？**

**预期数据形态**：

- 高并发下 Delta 和 NoDelta 的 stat 吞吐接近相等，或 Delta 略优
- 如果 Delta 略优，解释为"NoDelta 下父 inode 的 RMW 写锁间接影响了 stat 读路径"——这是一个有条件的解释
- 如果两者接近相等，解释为"读路径本身不受写路径影响"——即 DeltaOp 无额外读代价
- **两种结果对论文都是可接受的**，因为论点是"折叠机制未损害查询"，而不是"折叠机制让查询变快"

**写作要点**：

- 本节不追求 Delta stat 领先 NoDelta 的倍数
- 本节追求"DeltaOp 的折叠机制不拖慢查询"这一结论
- 如果数据显示 Delta stat 略低于 NoDelta（比如低 5% 以内），也是可接受的——理论上就该略低一点，这证明机制没有意外副作用

---

## 六、测试方案

### 6.1 测试矩阵

| SUT | N 档位 | mode | 迭代 | 备注 |
|---|---|---|---|---|
| rucksfs-delta | 2, 8, 32, 64, 96 | hard, easy | 3 | 全档覆盖 |
| rucksfs-nodelta | 2, 8, 32, 64, 96 | hard, easy | 3 | 对照，全档覆盖 |
| nfs | 2, 8, 32 | hard, easy | 3 | 只测中低并发 |
| juicefs-tikv | 2, 8, 32 | hard, easy | 3 | 只测中低并发 |

**总组合**：(5+5+3+3) × 2 = **32 组 mdtest**。

### 6.2 每 rank 文件数

| N | 每 rank 文件数 | 总文件数 |
|---|---|---|
| 2  | 2,000 | 4,000 |
| 8  | 2,000 | 16,000 |
| 32 | 2,000 | 64,000 |
| 64 | 2,000 | 128,000 |
| 96 | 2,000 | 192,000 |

相比 v2 的递减方案，**统一到 2000/rank**。目的是让每个阶段的执行时间足够长，减少测量噪声。

### 6.3 mdtest 参数

```
mdtest -d /mnt/sut/bench -n <fpr> -F -C -T -r [-u] -i 3
```

- `-i 3`：每组 3 次迭代，mdtest 报告 min/mean/max/stddev
- `-u`：easy 模式开启，hard 模式不开

### 6.4 前置代码改动

**改动 A**：`client/src/fuse.rs:20`

```rust
const TTL: Duration = Duration::from_secs(0);
```

改完后需要重新编译 `rucksfs-remote-client` 二进制并分发到所有客户端节点。

**改动 B**：新建 `testing/bench-v3/orchestrator.sh`（在 bench-v2 基础上修改）

- JuiceFS 挂载命令加上 `--attr-cache=0 --entry-cache=0 --dir-entry-cache=0 --open-cache=0`
- `files_per_rank()` 统一返回 2000
- `run_mdtest()` 里的 `-i 1` 改成 `-i 3`
- `SUTS` 按 N 档位分组处理：N ≤ 32 跑四家；N ≥ 64 只跑 `rucksfs-delta,rucksfs-nodelta`

**改动 C**：新建 `testing/bench-v3/` 目录存放结果；bench-v2/ 原位保留作为历史数据。

**改动 D**：新建 `infra/tencent-bench-v3/` 目录（复制自 `infra/tencent-bench/`），保留全部已验证配置（香港二区 AZ、SA5.MEDIUM2 客户端、SA5.16XLARGE256 服务端、原 AK/SK 与 SSH key）。本目录仅用于 bench-v3 实验，与原 `infra/tencent-bench/` 隔离。

### 6.5 Sanity check（8 分钟，大跑前必做）

1. **RucksFS TTL=0 生效**（3 分钟）：起 server + 1 client，做两次相隔 2 秒的 stat 对同一文件，MDS log 里确认两次都到 `getattr`。
2. **JuiceFS 缓存全关**（3 分钟）：挂载后同样做两次 stat，用 `juicefs stats` 或 `tikv-ctl` 观察请求数，确认每次 stat 都穿透到 TiKV。
3. **mdtest -i 3 解析通过**（2 分钟）：N=2 小负载跑一次，确认 `summary.csv` 解析脚本能处理 3 次迭代的输出。

### 6.6 运行中的异常信号

跑完立刻检查：

- **信号 1**：同一组 3 次迭代的 stddev/mean > 10% → 该组不稳定，需要重跑
- **信号 2**：Delta 与 NoDelta 在低并发 (N ≤ 32) 的 create/remove/stat 差距 > 5% → 说明有别的变量漏掉，需要排查
- **信号 3**：RucksFS 总体 stat 吞吐掉到 < 100 ops/s → TTL=0 可能改错地方，或 FUSE 路径有问题

### 6.7 时间预算

- Sanity check：8 分钟
- 32 组 mdtest × 约 70 秒/组（含清场）= 37 分钟
- SUT 切换开销：4 次切换 × 60 秒 × 部分档位 = 约 10 分钟
- 结果汇总：5 分钟

**总计约 60 分钟**集群时间，一次跑完。

---

## 七、跑完后的论文改动计划

**只在数据到手后才能确定的部分**（现在不动）：

1. 按两段式结构重组 `experiments.tex` 性能部分
   - 第一部分：横向对比表 + 论述（N≤32，四列含 NoDelta 作为 DeltaOp 消融参考）
   - 第二部分：DeltaOp 专项对比表 + 论述（N≥64，仅 Delta vs NoDelta）
2. 删除原"三级缓存"长段（312-324 行）、删除"加成/扣除"措辞、删除"136× 来自写锁阻塞"的错误解释
3. 所有表格数字更新为 v3 数据

**不等数据就可以改的部分**（已完成或现在可做）：

- ✅ 实验目的与测量范围段落（已写入 `experiments.tex` 第 5-17 行）
- ✅ 基线选择动机（已融入上述段落）
- 改写第 5.4 节里的自损词汇（"容易被误读""尽量"等）
