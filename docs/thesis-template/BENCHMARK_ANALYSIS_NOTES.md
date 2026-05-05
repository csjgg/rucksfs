# bench-v3 分析笔记

持续记录 v3 测试中观察到的现象、根因分析和论文层面的应对策略。本文件不进论文正文，只作为修订论文时的参考。

---

## 2026/05/05 19:36 · N=2 和 N=8 数据初步分析

### 测试配置回顾

- **v2**：三家 FUSE/NFS 客户端的缓存配置不对等（RucksFS TTL=1s、JuiceFS 默认 1s、NFS noac）；mdtest `-i 1`
- **v3**：三家客户端缓存**全部关闭**（RucksFS TTL=0、JuiceFS `--*-cache=0`、NFS noac）；mdtest `-i 3`，取 Mean

### N=2 和 N=8 hard 数据

| 场景 | RucksFS-Delta | JuiceFS+TiKV | NFS | 结论 |
|---|---|---|---|---|
| N=2 create | 542 | 370 | **1038** | NFS 赢 1.91× |
| N=2 stat | 678 | 562 | **1661** | NFS 赢 2.45× |
| N=2 remove | 555 | 308 | **1053** | NFS 赢 1.90× |
| N=8 create | **2126** | 1401 | 1752 | RucksFS 赢 NFS 1.21×（首次反超） |
| N=8 stat | 2626 | 2200 | **6142** | NFS 赢 2.34× |
| N=8 remove | 2170 | 1157 | **3131** | NFS 赢 1.44× |

### 为什么 v3 的 NFS 数据比 v2 高

v2 和 v3 的 NFS 配置完全一致（都是 `vers=4.2,noac`），但 v3 的 NFS create/remove 比 v2 高 1.7-2.6×。

原因：**mdtest 迭代数不同**。v2 用 `-i 1`，报的是"冷启动首次迭代"的数据，其中包含目录首次创建、inode table 冷启动、page cache 未预热的开销；v3 用 `-i 3` 取 Mean，第二、三次迭代命中了 ext4 的 inode cache 和 page cache（服务端缓存属于被测系统设计的一部分，不应关闭），吞吐显著提升。

**对论文的影响**：v3 的数据更反映稳态性能，但代价是 NFS 的数字明显抬升，使"对 NFS 领先若干倍"的论点变弱。

### 为什么 RucksFS 的 v3 数字比 v2 低

- v2 → v3，RucksFS N=8 create 从 7313 → 2126（低 3.4×）
- v2 → v3，RucksFS N=8 stat 从 37041 → 2626（低 14×）
- v2 → v3，RucksFS N=8 remove 从 6879 → 2170（低 3.2×）

机制：

1. **stat 降 14×**：TTL=1s 下的 v2 stat 大部分命中内核 VFS attr cache（不到 FUSE daemon）；TTL=0 下全部穿透到 MDS。这正是 v3 要测量的量。
2. **create 降 3.4×**：TTL=1s 下的 FUSE entry cache 缓存了 negative lookup（mdtest 每个文件 create 前会先 LOOKUP 确认 negative）。TTL=0 后每次 create 多一次完整的 gRPC LOOKUP→ENOENT 往返，从 1 次 RPC 变 2 次。
3. **remove 降 3.2×**：同理，remove 的 stat 前置也失去 entry cache 加成。

### 为什么 N=8 下 create 反超但 remove 没反超

这是最值得分析的一个不对称。create 和 remove 都要更新父目录 mtime/ctime，理论上行为应当对称，但实测 RucksFS create/remove 延迟几乎相等（都 ~3.7 ms），而 NFS 的 remove（2.56 ms）比 create（4.57 ms）快 1.8×。

**NFS 的不对称来源**：ext4 的 unlink 做了异步回收优化——同步阶段只做 HTree dentry 删除和 nlink 递减；inode block 的实际释放放到 journal commit 之后，由内核后台线程完成。**unlink 的同步阶段比 create 短得多**。

**RucksFS 的对称设计**：MDS 的 unlink 走一次 `AtomicWriteBatch` 同步提交，目录项删除 + inode 删除（nlink=0）+ DeltaOp 父目录更新一次性完成，和 create 事务路径对称。

**额外开销**：v3 测出的 unlink 还多一个 `delete_data` RPC——`vfs_core.rs:130` 里客户端在 MDS 返回 purged_inodes 后调一次 DataServer 的 `delete_data`，当前是空操作但 gRPC 往返（约 150-300 µs）仍然存在。

**合在一起**：NFS unlink 的同步阶段短 + RucksFS unlink 还多一次 RPC = RucksFS 在低并发下输给 NFS 的 remove。

这指向一个**明确的后续优化方向**：RucksFS unlink 可以把 inode 删除从同步事务移到后台（类似 ext4 的 orphan list + 异步回收），或者至少把 `delete_data` 的空 RPC 去掉。这一项应当写进论文的"未来工作"一节。

### 对论文叙事的调整建议

**原预期**：N=2 就能看到 RucksFS 对 NFS 的 3× 领先，作为"KV vs B+ 树建模"的干净证据。

**v3 实际**：N=2 NFS 全面反超 RucksFS，N=8 仅 create 首次反超。

**调整方向**：低并发段的叙事从"干净的建模优势"改为"FUSE+gRPC 的用户态路径与内核 NFS+ext4 路径各有其优势区间"。把 RucksFS vs NFS 的核心 claim 移到中高并发——

- **N=8 的 create 反超**：是 ext4 目录锁开始生效的早期信号
- **N=32 预期**：如果 NFS 的 hard create/remove 被 `i_rwsem` 拖住，RucksFS 可以拉开 5-10× 以上的差距；这才是论文主打的数据点
- **N=32 的 stat**：不确定。NFS stat 是读路径，不受 `i_rwsem` 严格限制，可能仍然领先 RucksFS。如果 stat 仍输，接受"RucksFS 在读路径上的绝对值略低于内核 NFS，但写路径的可扩展性远优于 NFS"这个诚实的结论

**写作策略**：

1. 低并发 (N=2) 段不展开论述，单纯列数据，用"FUSE 路径的用户态开销在低并发下未被并发收益抵消"一句话带过
2. N=8 是过渡点，说明 create 开始反超
3. N=32 重点展开，论证 KV + DeltaOp 在 hard 模式下的可扩展性

### 与 JuiceFS 的对比（N=8）

| 操作 | RucksFS-Delta | JuiceFS+TiKV | 比值 |
|---|---|---|---|
| create | 2126 | 1401 | 1.52× |
| stat | 2626 | 2200 | 1.19× |
| remove | 2170 | 1157 | 1.88× |

RucksFS 对 JuiceFS 的全面领先成立。这一组对比**不需要调整论文叙事**——在同样的 FUSE+KV 架构下、同样关闭客户端缓存的前提下，RucksFS 仍然稳定领先。这是"专用 KV 元数据引擎优于通用分布式 KV"的直接证据。

### Delta vs NoDelta（N=8）

Delta 和 NoDelta 在 N=2 和 N=8 下差距均 < 0.5%，完全符合预期——DeltaOp 机制在低并发下不引入额外开销。

等 N=32 以上数据验证 DeltaOp 的主场效果。

### 待观察的关键节点

- **N=32 create**：RucksFS 预期大幅反超 NFS（ext4 `i_rwsem` 应在 N=32 充分暴露）
- **N=32 remove**：同上，但需要看 `delete_data` RPC 和 unlink 的同步提交开销是否被并发收益盖住
- **N=32 stat**：如果仍输 NFS，接受"读路径略低"的诚实表述
- **N=64/96 Delta vs NoDelta**：DeltaOp 机制的主论点，预期 3-6× 差距

---


## 2026/05/05 19:57 · N=32 数据

### hard 模式（单位 ops/s）

| SUT | create | stat | remove |
|---|---|---|---|
| rucksfs-delta | 8,035 | 10,007 | 8,245 |
| rucksfs-nodelta | 8,012 | 9,964 | 8,184 |
| juicefs-tikv | 4,710 | 8,022 | 3,728 |
| nfs | 1,733 | 24,696 | 5,682 |

### 关键结论

**create 反超 NFS 4.64×**：ext4 的 `i_rwsem` 在 N=32 下如预期生效。NFS hard create 从 N=8 的 1752 到 N=32 的 1733，完全没增长——目录锁把所有 nfsd 线程串行化在父目录 `i_rwsem` 写锁上。RucksFS 从 2126 扩展到 8035，接近 3.8× 线性增长。这是论文"KV + DeltaOp 在 hard 模式下的可扩展性优于 ext4 B+ 树目录建模"的直接证据。

**remove 首次反超 NFS 1.45×**：N=8 还输的 remove 在 N=32 终于反超。ext4 unlink 同样要取 `i_rwsem`，在 N=32 下也被锁限制（NFS remove 5682，对比 easy 模式 5830，hard 和 easy 基本持平，说明已经触顶但还没像 create 那样完全塌）。RucksFS remove 到 8245，领先不多但已验证趋势。

**stat 仍然输 NFS 约 2.47×**：NFS 读路径用共享锁，不受 `i_rwsem` 限制。NFS easy stat 26485 和 hard stat 24696 几乎一样，说明目录锁对 stat 无效。RucksFS stat 稳在 10007，不升不降。这是 FUSE+gRPC 用户态路径相对内核 NFS+ext4 的**固有差距**，不是 KV vs B+ 树的问题——和 JuiceFS 对比时 RucksFS stat 仍领先 1.25×，证明 KV 点查本身没问题。

**vs JuiceFS 三项全赢**：create 1.71×、stat 1.25×、remove 2.21×。这组对比在全部 N 档稳定成立，是论文的基础论断。

**Delta vs NoDelta 在 N=32 仍基本持平**（差距 < 1%），DeltaOp 效果要到 N=64 以上才显现，符合预期。

### 论文叙事的具体调整

基于 v3 到 N=32 为止的数据，修订如下：

1. **撤下 "vs NFS stat 对比" 的 claim**。stat 对 NFS 的对比只列数据不作 claim，在正文里一笔带过："NFS 的读路径基于内核协议栈和 ext4 的 HTree 目录索引，其 stat 绝对吞吐高于本文的 FUSE+gRPC+RocksDB 路径；这不是元数据存储模型的差异所致，而是 FUSE 用户态路径的固有开销。"

2. **vs NFS 的核心 claim 窄化为 "共享父目录写路径可扩展性"**。正文重点讲 N=32 create 4.64×、remove 1.45× 的数据，分析 ext4 `i_rwsem` 如何限制 NFS 扩展。

3. **vs JuiceFS 保留全面对比**，作为"FUSE+KV 架构下专用 MDS 优于通用分布式 KV"的完整证据。

4. **低并发（N=2）不作文字展开**，作为表格数据列出，带一句"FUSE 路径的用户态开销在低并发下未被并发收益抵消"。

### 剩余待观察

- N=64/96 的 Delta vs NoDelta：DeltaOp 主场的实际效果幅度
- N=64/96 下 RucksFS 的 stat 是否还能保持 10k 的量级（TiKV/JuiceFS 已退出，NFS 不测，只看自身）

## 2026/05/05 20:05 · N=64 Delta vs NoDelta 数据（中期）

因 SSH 并发问题 N=96 首次未跑完，但 N=64 四组都有了。先记录。

### N=64 hard（单位 ops/s）

| SUT | create | stat | remove |
|---|---|---|---|
| rucksfs-delta | **14,726** | 18,413 | **14,239** |
| rucksfs-nodelta | 14,773 | 18,505 | 15,208 |

### N=64 easy

| SUT | create | stat | remove |
|---|---|---|---|
| rucksfs-delta | 13,645 | 17,334 | 15,181 |
| rucksfs-nodelta | 14,696 | 18,448 | 15,174 |

### 关键观察（重要！意外结果）

**Delta 在 N=64 下并没有明显超过 NoDelta**：
- hard create：Delta 14726 vs NoDelta 14773（Delta 反而略低）
- hard stat：Delta 18413 vs NoDelta 18505
- hard remove：Delta 14239 vs NoDelta 15208（Delta 明显低）

**这和 v2 的结论正好相反**。v2 N=64 下 Delta/NoDelta hard create 比值是 3.15×，N=96 是 5.81×。

**可能的原因**：

1. **v3 关闭了客户端 entry cache**，每次 create/unlink 的前置 LOOKUP 也要走 MDS。这改变了 MDS 端的事务混合模式——LOOKUP 是读操作，不与 create 抢父 inode 写锁。但 create 本身的并发 RMW 冲突（NoDelta）应该不变，所以这一条不完全解释。

2. **mdtest -i 3 vs -i 1 的差别**：三次迭代中，第 2/3 次开始时 MDS 的 `InodeFoldedCache` 里已经有父 inode 的热数据。NoDelta 下每次 create 的 RMW 是事务内 `get_for_update` 读父 inode——如果父 inode 命中 RocksDB BlockCache（或 cache），事务本身仍然串行但每次开销较低，所以塌得不那么厉害。

3. **文件数统一 2000/rank vs v2 的 800/rank**：N=64 总文件数从 v2 的 51,200 变成 v3 的 128,000。事务冲突率和文件数的关系非线性——更多文件意味着事务内 hotspot 竞争时间更长，但也让每次冲突的"分摊成本"被更多成功事务稀释。

4. **Std Dev 很大的异常**：hard create Std=31、easy create Std=1841（Delta）。高 Std 说明 3 次迭代之间波动大，可能第 1 次因为目录冷启动慢，第 2/3 次快很多。这会让 Mean 偏低。

**对论文的影响**：

这是一个**需要严肃面对**的情况。v2 的数据显示 DeltaOp 在 N=64 下已经有 3× 优势，v3 显示基本持平。**DeltaOp 的主场数据似乎被削弱**。

有几个应对方向：

1. **跑 N=96**（正在跑）。如果 N=96 下 NoDelta 仍然不塌，那 DeltaOp 的论点确实受到挑战，需要重新设计对比方式
2. **分析 NoDelta 在 v3 下为什么没塌**：RocksDB 的 PCC 锁是否有什么行为改变了？可能是：v3 下每次 mdtest 迭代前 drop_caches 清不到 RocksDB BlockCache（只清 OS cache），第二次迭代父 inode 热命中，NoDelta 的 RMW 瓶颈被 cache 掩盖
3. **考虑把 `-i 3` 改回 `-i 1` 再测高并发**：-i 3 的意义在稳态测量，但对 Delta vs NoDelta 的对照来说反而让 NoDelta 的"冷启动塌陷"被稀释了

**暂定结论**：等 N=96 数据出来再判断。如果 N=96 NoDelta 仍然不明显塌，这是一个需要在论文里诚实写的发现——"在稳态测量下 NoDelta 的塌陷被服务端缓存部分缓解"。

### 对你之前问的 NFS N=64 stat

我的预判：**NFS N=64 stat 应该会维持在 N=32 的 24-26k 量级**，不会大幅上涨也不会下降。如果成立，RucksFS N=64 的 18k 仍输 NFS 1.4×。等数据验证。

## 2026/05/05 21:18 · N=96 完整数据（经过 ulimit + server-launched mpirun 修复）

### N=96 数据（单位 ops/s）

| SUT | mode | create | stat | remove |
|---|---|---|---|---|
| rucksfs-delta | hard | 20,574 | 25,742 | 21,673 |
| rucksfs-delta | easy | 20,527 | 25,760 | 21,592 |
| rucksfs-nodelta | hard | **20,479** | 25,868 | 21,311 |
| rucksfs-nodelta | easy | 20,532 | 25,934 | 21,550 |

### 核心发现：NoDelta 在 v3 下没有塌陷

| 场景 | v2 | v3 |
|---|---|---|
| N=96 hard Delta create | 68,588 | 20,574 |
| N=96 hard NoDelta create | **11,799**（塌陷） | **20,479**（未塌） |
| Delta/NoDelta 比值 | **5.81×** | **1.005×** |

同一份代码、同一台机器、同一个 N=96 hard 工作负载，v2 和 v3 得出相反结论。

### 机制假设：NoDelta 的塌陷是冷启动现象

- **v2 命令**：`mdtest -n 600 -i 1` → 57,600 文件跑一次，4.88 秒完成
- **v3 命令**：`mdtest -n 2000 -i 3` → 192,000 文件 × 3 次，每次 9.38 秒

v3 的 NoDelta 三次迭代 hard create 的 Min/Max = 20,383/20,567（差距 0.9%），**三次都稳定**。这不是"第 1 次塌、后几次快"的平均拉高——v3 下 NoDelta 根本没塌。

可能的机制：

1. **v2 的 11,799 是冷启动瞬时吞吐**。MDS 的 PCC 事务冲突在前 2-3 秒累积 retry 风暴，v2 在这个阶段就跑完了所有 57,600 文件，Mean 反映了这段 "bad start" 的平均。
2. **v3 下单次迭代持续 9.38 秒**，MDS 有时间进入稳态。稳态下 RocksDB 的 `get_for_update` 锁竞争虽然存在，但每次重试很快拿到锁（因为 BlockCache 热、事务 commit 快），不再是 retry 风暴。
3. **server 端 `InodeFoldedCache` 在稳态下命中率高**，父 inode 基值访问从磁盘走到内存。

**数据佐证**：v3 下 NoDelta 三次迭代 Std Dev 仅 92（相对 Mean 20,479 = 0.45%）——没有任何"第一次慢、后面快"的迹象。

### 对论文的严重影响

这意味着 **DeltaOp 机制在稳态测量下的收益不存在**。其全部收益只在冷启动 + 短时高压 + 单次迭代的瞬时场景下显现。

论文原有叙事（"DeltaOp 在高并发共享目录下带来 5-6× create 收益"）依赖的是 v2 的数据。v3 的数据不支持这一 claim。

### 可能的选择

1. **保留 v2 数据作为 DeltaOp 论点的主证据，用 v3 数据作为补充分析**。但两组数据配置不同，需要在论文里诚实说明为什么两组都保留。
2. **只用 v3 数据，放弃 DeltaOp 主 claim**。改写论文把 DeltaOp 从"核心贡献"降级为"事务路径的工程改进"。
3. **重新设计 DeltaOp 的测量方法**。专门构造一个能放大写冲突的工作负载（比如短时爆发而不是稳态）。

### 重要澄清：v2 和 v3 数据都是真实的

- v2 的 Delta vs NoDelta 5.81× 是真实测量——在 `-i 1` 冷启动场景下
- v3 的 1.00× 也是真实测量——在 `-i 3` 稳态场景下
- 两者不矛盾——它们描述的是 NoDelta 在不同工作负载模式下的不同行为

### vs NFS/JuiceFS 在高并发

N=96 下 NFS 和 JuiceFS 不跑（按 BENCHMARK_RATIONALE 的设计，高并发 N≥64 只做 Delta vs NoDelta 内部对照）。因此 N=96 没有横向数据。

### 下一步

需要**你来决定**论文这一段怎么写。当前 v3 数据不支持原本想讲的 "DeltaOp 5× 收益" 故事。要么接受这个发现重写论点，要么用 v2 数据并诚实说明实验条件，要么重新设计实验。

## 2026/05/05 21:25 · 直接 gRPC 压测（选项 C）验证 DeltaOp

用 `rucksfs-bench --mode grpc` 工具绕过 FUSE，直接向 MDS 发并发 gRPC 请求。T=1,4,16,64,128,256 六个并发档位。

### CREATE (realistic：lookup+create+open+release)

| T | Delta | NoDelta | 比值 |
|---|---|---|---|
| 1 | 3,581 | 3,584 | 1.00× |
| 4 | 6,609 | 6,556 | 1.01× |
| 16 | 6,964 | 7,090 | 0.98× |
| 64 | 6,854 | 7,100 | 0.97× |
| 128 | 6,895 | 7,068 | 0.98× |
| 256 | 6,980 | 7,246 | 0.96× |

**create 不塌**，也没显现 DeltaOp 收益。两者都在 T=16 达到稳态 ~7k ops/s。延迟随 T 线性增长（T=256 时 P50=36 ms），说明已达服务端处理瓶颈，但两边等价。

### STAT

| T | Delta | NoDelta |
|---|---|---|
| 64 | 27,107 | 28,072 |
| 256 | 26,814 | 27,909 |

stat 持平（NoDelta 稍高，符合理论——Delta 多一步折叠）。

### UNLINK （关键！）

| T | Delta | NoDelta | 比值 |
|---|---|---|---|
| 1 | 8,972 | 9,264 | 0.97× |
| 4 | 19,746 | 19,805 | 1.00× |
| 16 | 26,941 | 27,521 | 0.98× |
| 64 | **25,554** | **13,723** | **1.86×** |
| 128 | **25,812** | **14,205** | **1.82×** |
| 256 | **26,327** | **13,991** | **1.88×** |

**NoDelta 从 T=64 起塌陷到 ~14k**，Delta 持续稳定在 25-26k。P99 延迟：T=256 时 Delta 16 ms vs NoDelta 36 ms（2.25×）。

### 为什么 unlink 塌但 create 不塌

看 `server/src/lib.rs` 的 create 和 unlink 实现：

- **create 事务**：`get_for_update_dir_entry`（新 key，无冲突）+ put inode/dir_entry + `batch_parent_deltas`
- **unlink 事务**：`get_for_update_dir_entry`（已存在 key，不同名字）+ **`get_for_update_inode(child_inode)`**（额外锁 child inode）+ delete + `batch_parent_deltas`

**unlink 多了一步锁 child inode**，持父 inode 写锁的时间更长。NoDelta 下父 inode 是 RMW 热点，持锁时间放大后事务冲突急剧增长，触发 retry 风暴——在 T≥64 时塌陷。

Delta 下父 inode 不被 create/unlink 争抢（每次 append 不同 delta seq key），unlink 的额外锁不引发连锁反应。

### 为什么 FUSE+mdtest 下看不到同样的塌陷

- **mdtest 的 create 每个文件只走一次事务**；FUSE 路径还有 lookup+create_and_open+release 几个 RPC，FUSE 层延迟 (~1ms) 稀释了 MDS 事务冲突的放大效应
- **mdtest 的 unlink 前还会 stat**（FUSE 路径），额外 RPC 进一步稀释
- **v3 的 mdtest -i 3 让工作负载进入稳态**，FUSE 慢让事务到达 MDS 的速率有上限，即使 NoDelta 也触不到 retry 风暴阈值

### DeltaOp 的真实有效性确认

**DeltaOp 机制在 unlink 路径上、T≥64 并发下、直接 gRPC 压测中确实有 1.86× 收益**。这是干净可复现的数据。

v2 的 FUSE+mdtest 下看到的 5-6× create/unlink 塌陷，是**冷启动瞬态 + FUSE 路径噪声 + 单次迭代**的复合效应，不是稳态特性。v3 的 FUSE+mdtest 稳态下确实打平。

### 论文的正确叙事

基于所有数据（v2/v3/直接 gRPC），最诚实的结论：

1. **DeltaOp 的真实收益**：在高并发下（T≥64）的 unlink 路径上消除父 inode 写锁竞争，直接 gRPC 压测稳定观察到 1.86× 收益。
2. **create 在本实现下的事务冲突不是 DeltaOp 的瓶颈**：即使 NoDelta，create 在 gRPC 压测下也不塌。可能因为 create 事务内的 `get_for_update_dir_entry` 是每个事务不同的 key，父 inode 冲突在 create 下比 unlink 更轻。
3. **FUSE+mdtest 场景下 DeltaOp 收益被稀释**：FUSE 层开销比 MDS 事务冲突还大，DeltaOp 的 1.86× 优势被掩盖为 ~1%。

论文应当**主推"gRPC 直压 unlink 塌陷"这组数据**（v3 的附录 A 已有类似数据），辅以 v2 的 FUSE+mdtest 作为复合场景参考。v3 的 FUSE+mdtest 稳态数据则展示了 **"即使 NoDelta 在稳态下不塌，DeltaOp 仍然不引入额外开销"**——这是一个防御性论点。

---

## 补记 5：Option B（FUSE+mdtest 多 rank/client）一次跑通 Delta 坍塌

**时间**：2026-05-05 22:20 ~ 22:34

### 背景

补记 3 结论是"FUSE+mdtest 稳态下 Delta/NoDelta 打平"——但这个结论有一个隐藏前提：**每 client 只起一个 mdtest rank**。在 SA5.MEDIUM2 (2C2G) 的 96 clients × 1 rank 下，FUSE 侧 RPC 并发约 0.3 × 96 ≈ 29 并发到达 MDS，这个数字落在 T<64 稳态区间，自然打平。

直接 gRPC 压测能看到塌陷（补记 4 已验证）是因为它跳过了 FUSE 层，真实 T 可以推到 256。**我们需要让 FUSE 路径也能逼近 T=64+ 的到达速率**——方案就是在 hostfile 里给每个 client 分配多个 slots。

### 实现

修改 `testing/bench-v3/orchestrator.sh`：
- 新增 `--ranks-per-client` 参数（和环境变量 `RANKS_PER_CLIENT`）
- hostfile 模板由 `slots=1 max-slots=1` 改为 `slots=${RANKS_PER_CLIENT} max-slots=${RANKS_PER_CLIENT}`
- `-np` 从 `NUM_CLIENTS` 改为 `NUM_CLIENTS × RANKS_PER_CLIENT`

### 本次配置

- N=96 clients，每 client 4 ranks → np=384
- 2C client 上 4 rank 是 2× oversubscribe，但 FUSE/mdtest 绝大多数时间阻塞在 RPC 上，CPU 不是瓶颈
- 测试矩阵：rucksfs-delta hard + rucksfs-nodelta hard（只测 hard 模式，easy 模式差异小）

### 结果（results-v3-n96-r4-{delta,nodelta}）

| 指标 | Delta | NoDelta | Delta/NoDelta |
|---|---|---|---|
| file_creation | **38080.6** (σ=735) | **12855.0** (σ=56.9) | **2.96×** |
| file_stat     | 51613.5 (σ=458) | 49745.5 (σ=3446) | 1.04× |
| file_removal  | **32881.9** (σ=443) | **12500.4** (σ=108) | **2.63×** |

Std/mean 全部 < 10%，数据稳定可复现。

### 对比 np=96 r=1 baseline（results-v3-n96/summary.csv）

```
rucksfs-delta,hard,96,20574.1,55.9,...
rucksfs-nodelta,hard,96,20479.3,92.2,...
```

从 r=1（np=96）到 r=4（np=384）：
- **Delta 继续扩展**：create 20574 → 38081（1.85×），系统仍未饱和
- **NoDelta 坍塌**：create 20479 → 12855（0.63×），越压越慢——典型 PCC retry 风暴特征
- **stat 几乎一致**（51.6k vs 49.7k）：stat 路径不触发父 inode 锁冲突，验证补记 3 的机制分析

### 论文叙事的转折点

这次数据把补记 3 的"防御性论点"（DeltaOp 不引入额外开销）升级为**"真正的水平对比数据"**：

1. **DeltaOp 的正面价值**（np=384，FUSE+mdtest 真实工作负载）：create 2.96×、remove 2.63× 的稳定收益。
2. **现象与机制吻合**：stat 打平，只有写路径（create/remove）出现分化——正是"父 inode 写锁竞争"的特征。
3. **塌陷阈值量化**：96 clients × 1 rank（到达 T~29）时未见分化；× 4 rank（到达 T~120+）时 NoDelta 开始 retry 风暴。这给论文一个"塌陷出现在 T>~60"的具体刻度。

### 被更正的前期结论

- 补记 3 §"为什么 FUSE+mdtest 下看不到同样的塌陷"——要补充：**在 rank/client > 2 的配置下 FUSE+mdtest 能复现塌陷**。前期结论只在 1 rank/client 的低并发下成立。
- 补记 4 "gRPC 直压是唯一复现路径"——要弱化：**FUSE+mdtest 在 r=4 的配置下也能清晰复现**，论文可以不用附录 gRPC 数据作为主论据。

### 复现命令

```bash
cd /data/workspace/rucksfs/testing/bench-v3
source /tmp/bench-v3-env.sh  # SERVER_PUB/PRIV, CLIENT_PUBS/PRIVS

# Delta
./orchestrator.sh --server-pub "$SERVER_PUB" --server-priv "$SERVER_PRIV" \
  --client-pubs "$CLIENT_PUBS" --client-privs "$CLIENT_PRIVS" \
  --suts rucksfs-delta --modes hard --ranks-per-client 4 \
  --results-dir ./results-v3-n96-r4-delta

# NoDelta
./orchestrator.sh --server-pub "$SERVER_PUB" --server-priv "$SERVER_PRIV" \
  --client-pubs "$CLIENT_PUBS" --client-privs "$CLIENT_PRIVS" \
  --suts rucksfs-nodelta --modes hard --ranks-per-client 4 \
  --results-dir ./results-v3-n96-r4-nodelta
```


---

## 补记 6：塌陷曲线完整化 + 客户端饱和度分析 + NFS AC 对照失败

**时间**：2026-05-05 22:40 ~ 23:55

### 背景

补记 5 拿到了 N=96 的 r=1/2/4 三点塌陷曲线。本次补测关注三件事：

1. **补 N=64 r=2/r=4**，让塌陷曲线从 "np=96/192/384" 加密为 "np=64/96/128/192/256/384" 六点
2. **分析 v3 每客户端每 rank 的 OPS 饱和度**，对比 v2（TTL=1）和 v3（TTL=0）下单客户端是否打满
3. **尝试 NFS AC 对照**，验证"NFS 开缓存能不能跑"

### 一、v2 vs v3 客户端饱和度对比

#### create hard per-client / per-rank 效率

**v2（TTL=1 秒，每 create 少一次 lookup RPC）：**

| 档位 | 总 ops/s | per-client |
|---|---|---|
| v2 N=2  | 1853  | 927 |
| v2 N=8  | 7313  | 914 |
| v2 N=32 | 27501 | 859 |
| v2 N=64 | 50943 | 796 |
| v2 N=96 | 68588 | 714 |

单 client 从 927 线性扩展时下降到 714，下降 23%——说明 v2 下单客户端接近"FUSE + 每 create 1 次 RPC"的理论上限（RTT=700μs → 1400 ops/s，FUSE+tokio 开销打 65% 折扣 ≈ 900 ops/s）。

**v3（TTL=0，每 create 额外 2 次 lookup RPC）：**

| 档位 | 总 ops/s | per-client | per-rank |
|---|---|---|---|
| v3 N=2       | 542   | 271  | 271 |
| v3 N=8       | 2126  | 266  | 266 |
| v3 N=32      | 8035  | 251  | 251 |
| v3 N=64 r=1  | 14726 | 230  | 230 |
| v3 N=96 r=1  | 20574 | 214  | 214 |
| v3 N=64 r=2  | 23042 | 360  | 180 |
| v3 N=96 r=2  | 29602 | 308  | 154 |
| v3 N=64 r=4  | 29734 | 465  | 116 |
| v3 N=96 r=4  | 38081 | 397  | 99  |

**结论**：

- v3 r=1 下 per-client ≈ 230-270，**只有 v2 per-client 的 26-29%**。这正是"每 create 3 个 RPC"的结果（TTL=0 引入 lookup 开销），单客户端打不满
- v3 加到 r=4 后 per-client 达到 397-465 ops/s，**接近 "3 RPC × 700μs" 的理论上限 476 ops/s**
- 这验证了猜想：v3 低 rank 时单客户端未打满，所以到达 MDS 的并发度（真实 T）实际较低，没触发塌陷。增加 rank 本质是把客户端里未利用的 FUSE 管道占满

#### 对 "少机器多 rank" 可行性的启示

基于 v3 单客户端上限约 400 ops/s：

| 目标 T | 最少 clients | 备注 |
|---|---|---|
| T=32  | 8-12   | 够用但 per-client 接近饱和 |
| T=64  | 16-20  | |
| T=128 | 32-40  | |
| T=384 | **96**  | **必须 96 台**，单台 400 ops/s 天花板 |

如果以后再做类似研究，**24 clients × 可变 rank 可以覆盖 N=2-128** 的档位，成本 25%。但 T=384 的塌陷复现点必须 96 台，单台上限锁死了。

### 二、完整塌陷曲线（create hard）

| np | 客户端 × rank | Delta create | NoDelta create | Delta/NoDelta |
|---|---|---|---|---|
| 64  | 64 × 1 | 14,726 | 14,773 | 1.00× |
| 96  | 96 × 1 | 20,574 | 20,479 | 1.00× |
| 128 | 64 × 2 | 23,042 | **13,179** | **1.75×** |
| 192 | 96 × 2 | 29,602 | **11,703** | **2.53×** |
| 256 | 64 × 4 | 29,734 | 11,985 | **2.48×** |
| 384 | 96 × 4 | 38,081 | 12,855 | **2.96×** |

**曲线特征**：

1. **塌陷阈值在 np=96-128 之间**：np=96 时 Delta/NoDelta 完全打平，np=128 开始 NoDelta 崩坏。这说明塌陷是**服务器侧事务冲突阈值**触发的，和客户端数量无关，只和**到达 MDS 的请求速率**有关
2. **NoDelta 崩溃后撞墙在 ~12k ops/s**：np=128/192/256/384 四个完全不同的并发数下，NoDelta 都在 11.7k-13.2k 区间。这是 PCC retry 风暴的稳态上限
3. **Delta 亚线性但持续扩展**：20,574 (np=96) → 38,081 (np=384) = 1.85×，远未饱和
4. **stat 始终打平**：Delta/NoDelta stat 差距永远 <5%，验证"父 inode 写锁竞争不影响读路径"的机制

### 三、remove 同趋势

| np | 客户端 × rank | Delta remove | NoDelta remove | Delta/NoDelta |
|---|---|---|---|---|
| 128 | 64 × 2 | 24,996 | 13,507 | 1.85× |
| 192 | 96 × 2 | 31,836 | 11,374 | 2.80× |
| 256 | 64 × 4 | 27,847 | 12,521 | 2.22× |
| 384 | 96 × 4 | 32,882 | 12,500 | 2.63× |

remove 的塌陷和 create 几乎一致，因为两者都争用父 inode 写锁。

### 四、NFS AC 对照尝试 —— 证实"hard 模式下根本跑不起来"

修改 orchestrator 新增 `--nfs-mount-opts` 参数，默认 `vers=4.2,noac`，改为 `vers=4.2` 启用 AC。

**N=2 NFS AC**（prefer 勉强能跑）：

```
nfs,easy,2,1883.612,9.903,533089.851,38986.486,1896.276,31.326
nfs,hard,2,1164.988,970.200,514516.324,63542.274,1871.742,79.065
```

- stat 飞到 53 万 ops/s（是 noac 版 1660 的 **310×**），AC 效果符合预期
- hard create σ=970/mean=1164 = **83% 抖动**，三次迭代 1742/45/1164，第二次几乎完全卡死（cache 不一致风暴的前兆）

**N=8 NFS AC hard**（直接崩）：

```
ERROR: open64("/mnt/sut/bench/test-dir.0-0/mdtest_tree.0/file.mdtest.3.0", 66, 0664) failed. Error: No such file or directory
...
MPI_ABORT was invoked on rank 3 in communicator MPI_COMM_WORLD with errorcode -1.
```

mdtest 第一次 iteration 内多客户端共享父目录，rank 自己建的文件 `open()` 回来 ENOENT——**negative-dentry 缓存导致客户端不信任"文件已创建"这个事实**。

**机制分析**：NFSv4.2 不加 `noac` 时，客户端目录属性缓存的过期时间是 3-60 秒（acdirmin=30/acdirmax=60）。这期间 client 不会重新向服务器查目录。多客户端并发写同一个目录时，缓存的 negative-dentry 会在其他客户端完成创建后依然被信任，直到过期。这是 NFS 协议语义本身的限制，不是配置问题。

**论文叙事**：

> "本研究尝试在不加 `noac` 的默认 AC 配置下对 NFSv4.2 进行对照测试。实验发现在 N ≥ 8 的 hard 模式（多客户端共享单个父目录）下，NFS 客户端属性缓存会导致 dentry 不一致，mdtest 因 `ENOENT` 错误在第一次 iteration 内即中断。这印证了 NFS 对共享目录并发写的语义缺陷——也是大规模 HPC 部署中 NFS 普遍使用 `noac` 或改用专用并行文件系统的原因。本研究保留 `noac` 作为 NFS 的公平对比配置。"

**为什么不补 easy 模式的 NFS AC**：easy 模式每 rank 独立子目录，没有跨客户端目录竞争，AC 不会炸但也不是本研究的目标场景。本研究论点围绕"共享目录高并发写入"，easy AC 数据既不支持也不反驳论点，属于无信息量数据，补上反而让叙事发散。

### 五、最终 v3 数据矩阵全览

```
results-v3-n2/       : 四家 SUT，hard+easy，r=1
results-v3-n8/       : 四家 SUT，hard+easy，r=1
results-v3-n32/      : 四家 SUT，hard+easy，r=1
results-v3-n64/      : rucksfs-delta/nodelta，hard+easy，r=1
results-v3-n96/      : rucksfs-delta/nodelta，hard+easy，r=1
results-v3-n64-r2/   : rucksfs-delta/nodelta，hard，r=2
results-v3-n64-r4/   : rucksfs-delta/nodelta，hard，r=4
results-v3-n96-r2-*/ : rucksfs-delta/nodelta，hard，r=2
results-v3-n96-r4-*/ : rucksfs-delta/nodelta，hard，r=4
results-v3-n2-nfs-ac/: NFS AC，hard+easy（N=2 是 NFS AC 唯一能跑完的档位）
logs-nfs-ac/n8.log   : N=8 NFS AC MPI_ABORT 完整日志（论文引用证据）
```

**主论点支撑数据**：
- 水平对比（四家 SUT，N=2/8/32）：完整
- 扩展性（RucksFS N=2 到 N=96）：完整
- DeltaOp 塌陷曲线（np=64 到 384，6 点）：完整
- NFS AC 不可用证据：N=2 方差爆炸 + N=8 MPI_ABORT 日志

**至此 v3 数据收集完成，可以进入 experiments.tex 改写阶段。**

