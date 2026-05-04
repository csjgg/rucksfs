# RucksFS Legacy Benchmark Archive（仅供参考）

> ⚠️ **本文档仅供参考。所有核心论文结论以新一轮统一对照测试为准**。
>
> 本文档汇总了 2026-03 到 2026-04 期间在不同代码分支、不同集群规格、不同工具下产生的测试数据。由于历史原因，这些数据的采集条件不完全可比，**不能作为论文最终数字来源**。
>
> 新一轮测试的方案见 `benchmark-plan-v2.md`。

---

## 1. 归档原因与使用说明

| 数据集 | 采集时间 | 测试工具 | Server 规格 | Client 规格 | 为何不能直接用 |
|---|---|---|---|---|---|
| Round 2 controlled | 2026-03 | mdtest, fs_mark, bonnie++ | 本地 KVM | 单机 | 非云环境，硬件抖动 |
| Round 3 Phase 1 | 2026-04-24 | mdtest hard | SA5.2XLARGE16 (8C16G) | 6×SA5.2XLARGE16 | server 被 client 限流到 5.8k ops/s，未观察到 delta 优势 |
| gRPC 直压 (8C server) | 2026-04-25 | 内部 rucksfs-bench | SA5.2XLARGE16 (8C16G) | 1×同规格 | 消融实验，不是端到端，仅能代表 server 上限 |
| FUSE 多 mount (8C server) | 2026-04-25 | fs_mark，每 client 32 mount | SA5.2XLARGE16 (8C16G) | 6×同规格 | 多 mount 非标准部署方式，审稿人可能质疑 |
| FUSE 多 mount (64C server) | 2026-04-25 | fs_mark，每 client 32 mount | SA5.16XLARGE256 (64C256G) | 6×SA5.2XLARGE16 | 同上，且规格不均衡 |
| realistic gRPC 模拟 FUSE (64C) | 2026-04-25 | 内部 rucksfs-bench --realistic | SA5.16XLARGE256 | 1×SA5.2XLARGE16 | 同上，仅 server 侧消融 |

---

## 2. Round 3 Phase 1 —— 原始 Phase 1 hard mode 数据

### 配置
- Server：1×SA5.2XLARGE16（8C16G），跑 MetadataServer + DataServer
- Client：6×SA5.2XLARGE16（8C16G）
- 工具：mdtest 4.1.0，MPI，hard 模式（shared parent dir）
- 参数：`-F -C -T -r -i 1`，files/rank 视 N 递减

### 结果（File creation ops/sec，3 runs per N）

| N   | rucksfs-delta | rucksfs-nodelta | ratio |
|-----|---|---|---|
| 8   | 1048.7 / 1040.6 / 1053.5 | 1037.5 / 1037.5 / 1051.2 | 1.01 |
| 16  | 2015.9 / 2015.6 / 2012.7 | 2015.9 / 2010.1 / 2012.5 | 1.00 |
| 32  | 3951.3 / 3948.8 / 3951.4 | 3942.7 / 3932.1 / 3923.4 | 1.01 |
| 64  | 5676.7 / 5669.8 / 5664.4 | 5667.9 / 5660.9（cold-run 丢弃）| 1.00 |
| 96  | 5820.3 / 5803.3 / 5786.3 | 5794.6 / 5804.6 / 5794.8 | 1.00 |
| 128 | ~5800 | ~5800 | 1.00 |
| 192 | ~5800 | ~5800 | 1.00 |

### 发现

- 吞吐在 N=96 之后饱和在 5800 ops/s
- server CPU 仅用 1.1 核（8 核中），说明瓶颈在 client 端
- 原始解释："metaserver 单 mutex / 单 RocksDB write path 序列化"
- **后来发现真正原因**：FUSE 每个 client 的 VFS `i_rwsem` 把同目录 32 个 rank 串行到单流，每 client 贡献 ~970 ops/s；6 client × 970 ≈ 5800。server 实际并发 CREATE ≈ 6/3 = 2，远低于 delta 激活阈值。

---

## 3. gRPC 直压消融数据（8C server）

### 配置
- Server：SA5.2XLARGE16（8C16G），仅跑 MDS
- Client：1×同规格，跑内部 rucksfs-bench（非 FUSE）
- 工具：rucksfs-bench `--mode grpc`，绕过 FUSE 直连 MDS
- 每个 thread 一个 tokio task 发 create RPC

### 纯 CREATE 吞吐（1 RPC/op）

| T (threads) | delta ops/s | nodelta ops/s | ratio |
|---|---|---|---|
| 1  | 3,519 | 3,514 | 1.00 |
| 2  | 6,834 | 6,851 | 1.00 |
| 4  | 12,841 | 12,776 | 1.00 |
| 8  | 23,469 | 22,371 | 1.05 |
| 16 | 41,288 | 28,906 | **1.43** |
| 32 | 60,131 | 30,359 | **1.98** |
| 64 | 68,506 | 32,587 | **2.10** |

### 解读（历史视角）

- T≥16 后 delta/nodelta 分叉，T=32-64 稳定在 2×
- 这是 **server 端孤立条件下的 delta 理论上限**
- 不反映真实 FUSE 端到端体验

---

## 4. FUSE 多 mount 实验（fs_mark 工具）

### 背景

为了在 FUSE 环境下逼近 gRPC 直压的效果，尝试在每台 client 起多个 FUSE mount，让每个 mount 成为一个独立 VFS 实例（独立 `i_rwsem`），绕过单 mount 的串行化。

### 8C server 结果

| K (mount/client) | 总 mount 数 | delta ops/s | nodelta ops/s | ratio |
|---|---|---|---|---|
| 16 | 96 | 25,474 | 21,318 | 1.20 |
| 32 | 192 | 28,262 | 21,326 | 1.32 |
| 48 | 288 | 25,035 | 20,360 | 1.23 |

### 64C server 结果

| K (mount/client) | 总 mount 数 | delta ops/s | nodelta ops/s | ratio |
|---|---|---|---|---|
| 32 | 192 | 66,184 | 13,490 | **4.88** |

### 为何不能作为论文主要数据

- **"每 client 32 mount" 非标准部署**：真实生产 FUSE 文件系统每客户端只挂载一次
- kernel 层面，192 mount 等效于 192 个独立 VFS client —— **实际上等同于 192 台单 mount 机器**，但多出"同机多进程"不确定因素
- mdtest 是 HPC 业界标准，fs_mark 在分布式文件系统论文中较少引用
- 论文若 claim 这个数据，需要清楚解释"多 mount ≈ 多 client"等价性

### 为何保留参考价值

- 作为**消融上限指示**：在 FUSE 协议约束下，如果有足够多独立 VFS，delta 理论能放大多少
- 验证了一个论断：**server 端 CPU 和并发资源充足时，delta 的收益会从 2× 进一步放大**（因为 nodelta 在高并发时会因为 RMW 冲突导致吞吐崩溃：8C server 时 nodelta 21k，64C server 时反降到 13.5k，而 delta 持续上升到 66k）

---

## 5. realistic gRPC 直压（64C server）

### 配置
- rucksfs-bench `--realistic`：每"逻辑 create"模拟 FUSE 发 4 个 RPC（LOOKUP + GETATTR + CREATE_AND_OPEN + async RELEASE）
- Server：64C256G

### 结果

| T | delta ops/s | nodelta ops/s | ratio |
|---|---|---|---|
| 1   | 1,108 | 1,137 | 0.97 |
| 16  | 15,067 | 14,998 | 1.00 |
| 32  | 21,818 | 21,603 | 1.01 |
| 64  | 28,510 | 24,592 | 1.16 |
| 128 | **35,465** | **13,219** | **2.68** |
| 192 | 38,555 | 13,643 | 2.83 |

### 历史价值

- 证明 nodelta 在 T≥128 时会因 RMW 冲突重试风暴而崩溃（从 T=64 的 24.6k 跌到 T=128 的 13.2k）
- 预测出"标准 mdtest 需要约 128 台独立 client 才能看到 delta 优势"
- 是新方案 client 数设计的理论依据

---

## 6. 工具与测试方法对照

| 场景 | 工具 | 语义 | 是否业界标准 |
|---|---|---|---|
| Phase 1 | mdtest MPI hard | 多节点、共享目录、MPI barrier | ✅ HPC 事实标准 (IO500) |
| gRPC 直压 | rucksfs-bench | 内部压测，绕过 FUSE | ❌ 自研工具 |
| gRPC realistic | rucksfs-bench --realistic | 模拟 FUSE RPC 序列 | ❌ 自研工具 |
| 多 mount | fs_mark | 本地 FS benchmark (ext4/XFS 社区) | ⚠️ 不是分布式 FS 论文标准 |

---

## 7. 已知问题 / 数据不可比的原因总结

1. **server 规格变化**：从 8C 升到 64C，不能直接跨硬件对比
2. **client 规格变化**：8C/2C 混用
3. **测试工具异质**：mdtest / fs_mark / rucksfs-bench 三种
4. **代码分支变化**：期间经过 commit 2028e32（commlayer optim）、9e0f495（Step A backoff fix）
5. **集群架构变化**：6 client → 2 client 小规模 → 即将扩到 32/128

---

## 8. 新方案（v2）指向

新方案见 `benchmark-plan-v2.md`，核心原则：

- 统一 server（64C256G）
- 统一 client（2C2G SA5.MEDIUM2）
- 统一工具（mdtest 标准 hard/easy）
- 覆盖 RucksFS delta / RucksFS nodelta / NFS / JuiceFS
- 覆盖 create / stat / read / remove
- Client 数从 2 扩到 128，得到 scaling 曲线
- 所有数据在同一集群、同一次会话中采集，完全可比

---

**生成时间**：2026-04-25
**状态**：归档，仅供参考
