# RucksFS 分布式高并发对比实验方案 (Round 3) — 最终版

**目标**：在分布式集群上用**高并发 mdtest-hard** 重测 RucksFS (delta / no-delta) + NFS + JuiceFS，产出论文级对比数据，**证明 delta 机制在高争用下的 E2E 优势**。

---

## 一、诊断回顾（为什么要 Round 3）

| 场景 | T=16 create | T=32 no-delta 延迟 | T=32 with-delta 延迟 |
|---|---|---|---|
| 进程内 (in-process) | delta: 379k, no-delta: 73k | **589μs** | 77μs |
| 本机 E2E (单 client) | ~3400 ops/s（两者持平） | 看不到 | 看不到 |

**本机 E2E 看不到 delta 优势的根因**：单 client 的单 gRPC HTTP/2 连接在出口就被串行帧化（~30μs/op 封顶 34k ops/s），请求排在出口队列而不是服务端争用段。服务端 CPU 只用了 4 核（共 32 核）。

**解决方案**：**6 个客户端节点独立 gRPC 连接** → 服务端真正并发处理 → 把 no-delta 压进"每 op 500-600μs"的事务冲突区 → delta 优势浮现。

---

## 二、集群拓扑（最终）

**7 节点**（1 服务器 + 6 客户端）：

```
                  ┌─────────────────┐
                  │  服务器 (1 台)    │  SA3.4XLARGE32 (16C32G, 200GB SSD)
                  │  按 SUT 轮换：    │
                  │  - rucksfs-metaserver + rucksfs-dataserver
                  │  - nfsd + exportfs
                  │  - redis + juicefs mount
                  │  - PD + TiKV + juicefs mount
                  └────────▲────────┘
                           │ 10Gbps 内网 (同 VPC / 同 AZ)
      ┌────────┬───────────┼───────────┬────────┐
   ┌──▼──┐ ┌──▼──┐ ┌──▼──┐ ┌──▼──┐ ┌──▼──┐ ┌──▼──┐
   │ c0  │ │ c1  │ │ c2  │ │ c3  │ │ c4  │ │ c5  │
   └─────┘ └─────┘ └─────┘ └─────┘ └─────┘ └─────┘
   6 × SA3.2XLARGE16 (8C16G, 100GB SSD) 每台跑 32 MPI rank = 共 192 并发
```

**关键设计决策**：
- **服务器合并 meta+data**：跟 NFS 对齐（NFS 本来就是一体），保证 SUT 间公平
- **服务器 16C32G**：本机 32C 都跑不满，升到 16C 保证服务端 CPU 不成为新瓶颈
- **6 客户端**：打破本机单 client 的 HTTP/2 串行瓶颈，达到 192 rank 高争用

---

## 三、被测系统 (5 个 SUT)

| SUT | 元数据 | 数据 | 客户端 | 优先级 |
|---|---|---|---|---|
| **RucksFS-delta** | RocksDB | RawDisk | gRPC+FUSE | **P0** 必测 |
| **RucksFS-nodelta** | RocksDB | RawDisk | gRPC+FUSE | **P0** 必测（消融对照）|
| **NFS v3** | kernel nfsd × 64 threads | 本地 ext4 | NFS v3 | P1 |
| **JuiceFS+Redis** | Redis | 本地 ext4 (file backend) | FUSE | P2 |
| **JuiceFS+TiKV** | 单机 TiKV+PD | 本地 ext4 | FUSE | P3（可选）|

**关于 JuiceFS 的 ext4 file backend**：论文中明确标注 "file backend is used to isolate the metadata bottleneck; production JuiceFS uses S3-compatible object storage"。mdtest `-F` 0 字节文件不触发 chunk 写入，ext4 后端对 create/stat/remove 的元数据吞吐几乎无影响。

**关于 JuiceFS+TiKV 单机**：标注"non-production; TiKV is designed for 3-node minimum deployment"。

---

## 四、实验矩阵

### 主实验：mdtest-hard scaling

```
mpirun --hostfile hosts.txt -np $N \
  mdtest -d $MNT/hard_$N -n 2000 -F -C -T -r -i 3
```

（无 `-u` = 共享父目录 = hard 模式）

| 参数 | 值 |
|---|---|
| 并发 N | **8, 16, 32, 64, 96, 128, 192** |
| 每 rank 文件数 | 2000（N≥128 时降到 1000） |
| 重复 | 每配置 3 次，取中位数 |
| 指标 | create/stat/remove ops/s，P50/P99 |

共 **7 × 5 × 3 = 105** 次测试。

### 辅助实验（仅在阶段 1 数据符合预期后进行）

- **Exp-A mdtest-easy**（`-u` 独立子目录）：N=8/32/128，5 SUT，验证"争用"是 delta 优势的来源
- **Exp-B 服务端 profile**：在高并发测试期间用 pidstat + perf 采样服务端

---

## 五、执行顺序（按你的建议，分阶段+熔断）

```
┌─────────────────────────────────────────────────────────┐
│  Phase 0 (本地, 0 成本)                                  │
│  - gRPC 连接池改造（client 侧）                          │
│  - Terraform 多客户端化 (for_each, num_clients=6)        │
│  - Orchestrator v3 脚本                                  │
│  - SUT 切换 + 清理脚本 (下次单独讨论完整版)              │
│  - 本地 2-client 冒烟测试                                │
│                                                          │
│  验收标准：冒烟测试通过，一条命令跑一轮完整 SUT          │
└─────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────┐
│  Phase 1 (上云, ~¥50)  【熔断点】                        │
│  - terraform apply                                       │
│  - 跑 RucksFS-delta 全矩阵 (7 并发 × 3 次 = 21 次)       │
│  - 跑 RucksFS-nodelta 全矩阵                             │
│  - 立即解析这 42 次结果                                  │
│                                                          │
│  熔断判定（任一条触发就销毁集群、回本地诊断）：          │
│    - N=64 时 delta/nodelta < 1.5x                        │
│    - N=128 时 delta/nodelta < 2x                         │
│    - 服务端 CPU 使用率 < 50%（没被压到争用段）           │
│                                                          │
│  ★ 如果不达标：terraform destroy → 回本地改架构          │
│  ★ 如果达标：继续 Phase 2                                │
└─────────────────────────────────────────────────────────┘
                            ↓ (达标)
┌─────────────────────────────────────────────────────────┐
│  Phase 2 (上云, ~¥17)                                    │
│  - 切换服务器为 NFS 模式（清理 + 启动 nfsd）             │
│  - 跑 NFS 全矩阵                                         │
└─────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────┐
│  Phase 3 (上云, ~¥17)                                    │
│  - 切换服务器为 JuiceFS+Redis 模式                       │
│  - 跑 JuiceFS+Redis 全矩阵                               │
└─────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────┐
│  Phase 4 (上云, ~¥17, 可选)                              │
│  - 切换服务器为 JuiceFS+TiKV 模式                        │
│  - 跑 JuiceFS+TiKV 全矩阵                                │
└─────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────┐
│  Phase 5 (本地, 0 成本)                                  │
│  - terraform destroy 【强制执行】                        │
│  - 结果解析 + CSV + 画图                                 │
│  - 更新 docs/thesis-template/body/experiments.tex        │
│  - 写 docs/round3-findings.md                            │
└─────────────────────────────────────────────────────────┘
```

**关键原则**：
1. **一次只跑一个 SUT**，绝不并发
2. **每个阶段结束立即看数据**，不达标就停
3. **集群按需销毁**：如果两天之内跑不完，中途也销毁，下次 apply 重跑
4. **所有 SUT 共用同一台服务器轮换**，保证硬件公平

---

## 六、成本（真实，已用 tccli 查过）

**价格基准**（ap-guangzhou 按量计费，2026/04/25）：
- SA3.4XLARGE32 (16C32G): **¥1.88/h**
- SA3.2XLARGE16 (8C16G): **¥0.94/h**
- 200GB CLOUD_SSD: **¥0.11/h**

| 阶段 | 墙钟 | 成本 | 累计 |
|---|---|---|---|
| Phase 0 本地 | 1 天 | ¥0 | ¥0 |
| Phase 1 (RucksFS×2) | 6h | ¥50 | ¥50 |
| Phase 2 (NFS) | 2h | ¥17 | ¥67 |
| Phase 3 (JuiceFS+Redis) | 2h | ¥17 | ¥84 |
| Phase 4 (JuiceFS+TiKV) | 2h | ¥17 | ¥101 |
| Phase 5 本地 | 0.5 天 | ¥0 | ¥101 |

**全部跑完 ~¥100。** 如果 Phase 1 熔断只付 ¥50 就止损。24h 留足余量的极限预算 ¥199。

---

## 七、预期数据（理论预测，供熔断判定参考）

### mdtest-hard create 吞吐 (ops/s)

| N | RucksFS-delta | RucksFS-nodelta | **delta 倍率** | NFS | JuiceFS-Redis | JuiceFS-TiKV |
|---|---|---|---|---|---|---|
| 8 | 8k | 7.5k | 1.1x | 6k | 5k | 4k |
| 16 | 14k | 11k | 1.3x | 8k | 7k | 5.5k |
| 32 | 22k | 12k | 1.8x | 10k | 8k | 6.5k |
| **64** | **28k** | **10k** | **2.8x** ★ | 11k | 8.5k | 7k |
| 96 | 30k | 9.5k | 3.2x | 10.5k | 8.5k | 7k |
| 128 | 30k | 9k | 3.3x | 10k | 8.5k | 7k |
| 192 | 30k | 8k | 3.8x | 9.5k | 8k | 6.5k |

**熔断阈值**：
- N=64 时 delta/nodelta ≥ 1.5x ← 最低要求
- N=128 时 delta/nodelta ≥ 2.0x ← 最低要求
- 理想值是上表预测，预测偏差 < 50% 算达标

---

## 八、Phase 0 交付物清单（上云前必做）

- [ ] `rpc/src/metadata_client.rs` 连接池（`PooledMetadataRpcClient`，4 连接 round-robin）
- [ ] `client/src/lib.rs` 集成连接池
- [ ] `infra/tencent-bench/instances.tf` 多客户端 `for_each`
- [ ] `infra/tencent-bench/outputs.tf` 输出客户端 IP 列表
- [ ] `infra/tencent-bench/scripts/init-client.sh` 加 SSH 互信
- [ ] `testing/round3_scripts/master-orchestrator-v3.sh` 多客户端编排
- [ ] `testing/round3_scripts/switch-sut.sh` SUT 切换+清理脚本（强保证）
- [ ] `testing/round3_scripts/parse-results.py` mdtest 输出 → CSV
- [ ] 本地 2-client 冒烟测试通过（mpirun 能跨节点、结果能收集）

---

## 九、清理方案（后续单独讨论，但这里先列核心原则）

每次切换 SUT 时：
1. **进程清理**：pkill 所有 SUT 相关进程 + 验证 `ps` 无残留
2. **挂载清理**：umount 所有 FUSE / NFS + 验证 `mount` 无残留
3. **数据清理**：`rm -rf` 每个 SUT 专属 data dir
4. **端口清理**：验证 `netstat` 无旧端口占用
5. **缓存清理**：sync + drop_caches 服务端 + 所有客户端
6. **兜底**：如有任何疑问直接重启服务器（60s 恢复）

**完整的清理脚本 + 每步验证 + trap 日志**会在 Phase 0 写出来，下次单独对方案。

---

## 十、可交付物

1. **原始数据**: `testing/results/round3_<timestamp>/raw/` 所有 mdtest 输出
2. **解析 CSV**: create/stat/remove/P99 四份
3. **PDF 图**: 4 张（scaling 曲线、delta 倍率、P99、easy vs hard）
4. **论文更新**: `docs/thesis-template/body/experiments.tex`
5. **脚本可复现**: `testing/round3_scripts/`
6. **发现文档**: `docs/round3-findings.md`

---

## 十一、确认事项（给人看的 summary）

✅ 会**完整重测**之前 NFS / JuiceFS / RucksFS 的全部对比（mdtest-hard 为主）
✅ 会**清晰体现 delta vs no-delta 的差距**（消融实验在 Phase 1，优先级最高）
✅ **先跑 RucksFS delta/no-delta**，不达标就停下来优化，不浪费后面的钱
✅ **所有 SUT 共享同一台服务器**，公平对比
✅ 成本真实约 **¥100**（非 ¥1000），Phase 1 只付 ¥50 就有熔断保护
✅ 每个 Phase 都是独立命令，集群可以随时销毁/重建
✅ 清理方案会另行细化确认
