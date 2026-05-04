# RucksFS Benchmark Plan v2（正式）

> 本方案是 RucksFS 论文数据的**正式采集方案**。所有论文章节的数字以本方案产出为准。
> 之前的数据见 `benchmark-archive-legacy.md`（仅供参考）。

## 0. 测试目标

1. **论文主结论**：RucksFS 在元数据密集型工作负载下的性能
2. **消融实验**：delta vs nodelta，量化 Delta Record 机制的收益
3. **横向对比**：RucksFS vs NFS vs JuiceFS+TiKV，同硬件、同工具、同参数

## 1. 硬件拓扑

```
1× Server 节点（SA5.16XLARGE256, 64C256G, Hong Kong AZ2）
    /tmp/rucksfs-metaserver (端口 8001)       // RucksFS MDS
    /tmp/rucksfs-dataserver (端口 8002)       // RucksFS DS（数据），与 MDS 同机
    /data/rucksfs-*                            // 本地 ext4 on CLOUD_SSD 200GB

    NFS server (端口 2049)                     // 按阶段启动
    JuiceFS + TiKV 节点（端口 2379/20160）     // 按阶段启动

N× Client 节点（SA5.MEDIUM2, 2C2G, 同 AZ）
    N ∈ {2, 8, 32, 128}
    每节点 1 个 FUSE mount（标准部署，禁止多 mount 技巧）
    每节点通过 mpirun 跑 1 rank mdtest
```

**所有 client 规格完全一致，数据可比。**

## 2. 软件栈

### RucksFS
- Git: 主分支最新（包含 Step A backoff fix 9e0f495）
- delta 版：默认 build
- nodelta 版：`cargo build --features rucksfs-server/no_delta`
- Client：`rucksfs-remote-client` FUSE mount, `RUCKSFS_CLIENT_POOL_SIZE=4`

### NFS
- `nfs-kernel-server` 装在 server 节点
- 导出目录：`/data/nfs-export` (ext4)
- 客户端挂载：`mount -t nfs -o vers=4.2,noac`
  - `noac` 关闭客户端 attr cache，测 server 真实元数据性能（见 Phase 1 的 NFS 公平性校准）

### JuiceFS + TiKV
- TiKV 单节点跑 server 机器（port 2379 PD + 20160 TiKV）
- JuiceFS v1.2，`juicefs format` 指向 TiKV
- 底层存储：同机 `/data/jfs-data` (ext4)
- 客户端挂载：`juicefs mount tikv://server:2379/bench /mnt/jfs`

## 3. 测试矩阵

### 维度
- **SUT (system under test)**: rucksfs-delta, rucksfs-nodelta, nfs, juicefs-tikv
- **Client 数 N**: 2, 8, 32, 128
- **模式**: hard (共享父目录), easy (`-u`，每 rank 独立子目录)
- **操作**: create + stat + remove
- **有数据 vs 无数据**: `-w 0`（无数据，纯元数据）vs `-w 4096`（4KB/文件）

### 完整矩阵

**主表（仅测重要组合）**：

| SUT | N | hard -w 0 | easy -w 0 | hard -w 4096 | easy -w 4096 |
|---|---|---|---|---|---|
| rucksfs-delta   | 2, 8, 32, 128 | ✓ | ✓ | ✓ | ✓ |
| rucksfs-nodelta | 2, 8, 32, 128 | ✓ | ✓ | ✓ | ✓ |
| nfs             | 2, 8, 32, 128 | ✓ | ✓ | ✓ | ✓ |
| juicefs-tikv    | 2, 8, 32, 128 | ✓ | ✓ | ✓ | ✓ |

**总组合**：4 SUT × 4 N × 2 mode × 2 data = 64 组 mdtest 运行

每组跑 **1 iteration**（`-i 1`），files/rank 视 N 递减（N=2: 2000, N=8: 1500, N=32: 1000, N=128: 500）。

**预计单次 run 时长**：10-60 秒 → 64 组 × 平均 30s ≈ **~32 分钟**（不含集群起停和 SUT 切换）。

## 4. 测试参数

### mdtest 命令

**hard 模式（shared dir，测锁竞争）**：
```bash
mpirun --allow-run-as-root --hostfile hostfile-N -np N \
    mdtest -d $MNT/bench -n FILES -F -C -T -r -w DATA -i 1
```

**easy 模式（unique dir per rank，测无冲突上限）**：
```bash
mpirun ... mdtest -d $MNT/bench -n FILES -F -C -T -r -w DATA -u -i 1
```

### 参数含义

- `-F`：只测文件（不测目录）
- `-C`：执行 create 阶段
- `-T`：执行 stat 阶段
- `-r`：执行 remove 阶段
- `-w`：每文件写入的字节数（0 = 纯 metadata; 4096 = 4KB）
- `-u`：easy 模式（每 rank 自己的子目录）
- `-i 1`：1 次迭代（论文报告单次结果，用 3 次取平均会让矩阵太大）

### 每 rank 文件数（避免 N 大时跑太久）

| N | files/rank | 总文件数 |
|---|---|---|
| 2   | 2000 | 4,000 |
| 8   | 1500 | 12,000 |
| 32  | 1000 | 32,000 |
| 128 | 500  | 64,000 |

## 5. 测试流程

### 阶段 A：先导验证（已完成 ✅）

- 2 client × SA5.MEDIUM2 (2C2G) × 1 rank
- 跑 rucksfs-nodelta 单组 hard -w 0
- 目标：验证 2C2G 规格不成为瓶颈
- **结果**：925 ops/s / client，线性扩展到 2 × 925 = 1841（符合预期）

### 阶段 B：小规模完整矩阵（下一步）

- 保持 2 client（暂时）
- 跑 **完整 64 组矩阵**
- 目标：
  - 验证测试脚本正确性
  - 拿到 RucksFS/NFS/JuiceFS 的相对位置
  - 可能 2 client 就能看到 delta 在 nodelta 开始走下坡时的分叉苗头
- 成本：SA5.MEDIUM2 2 台 × ¥0.26/h + 64C server × ¥16.78/h × 1h ≈ **¥17**

### 阶段 C：中规模（8 client）

- 扩到 8 client
- 跑同样 64 组矩阵
- 目标：看 scaling 曲线开始出现的点
- 成本：8 × ¥0.26 + ¥16.78 ≈ **¥19/h**，跑 30min ≈ **¥10**

### 阶段 D：大规模（32 client）

- 扩到 32 client
- 同样矩阵
- 目标：delta/nodelta 在 hard 模式下应该看到 1.1-1.4× 的分叉
- 成本：32 × ¥0.26 + ¥16.78 ≈ **¥25/h**

### 阶段 E：终极（128 client）

- 扩到 128 client
- 同样矩阵
- 目标：delta/nodelta 在 hard 模式下达到 2×+
- 成本：128 × ¥0.26 + ¥16.78 ≈ **¥50/h**

### 总成本估算

| 阶段 | 机器 | 时长 | 成本 |
|---|---|---|---|
| B (2c) | 2 | 1h（包含脚本调试、SUT 切换）| ¥17 |
| C (8c) | 8 | 1h | ¥19 |
| D (32c) | 32 | 1h | ¥25 |
| E (128c) | 128 | 1h | ¥50 |
| **合计** | | **~4h** | **~¥111** |

还要加上 JuiceFS + TiKV 部署和调试的额外 1-2h（可能再 ¥30-60）。

**总预算上限：¥200 以内**，得到完整的 4 SUT × 4 N × 2 mode × 2 data = 64 组数据矩阵。

## 6. 测试执行顺序（单次完整运行）

在 N client 就绪、64C server 就绪后：

```
for SUT in rucksfs-delta, rucksfs-nodelta, nfs, juicefs-tikv:
    setup_sut(SUT)          # 启动 SUT server，mount 到所有 client
    for mode in hard, easy:
        for data in 0, 4096:
            mdtest(N, mode, data)    # 一次跑完 create+stat+remove
    teardown_sut(SUT)
```

每个 SUT 切换时完全 tear down 上一个（重启 server DB, umount FUSE/NFS），避免串扰。

## 7. 监控和采集

### 运行时
- server CPU（`top -bn1 -p <mds_pid>,<ds_pid>`）
- client CPU（`top -bn1`）
- 每 SUT 运行时采样 3 次（开始 / 中间 / 结束）

### 结果文件
- `results/<SUT>/<mode>_w<data>_np<N>_<run>.txt`：mdtest 原始输出
- `results/summary.csv`：汇总表（SUT, N, mode, data, create_ops, stat_ops, remove_ops）
- `results/cpu_<SUT>_<config>.txt`：CPU 采样

## 8. 成功标准

**最低可接受输出（论文能用）**：
- 每个 SUT × N × mode × data 至少 1 组有效数据
- create / stat / remove 三个 metric 都有报告
- delta vs nodelta 的 create ratio 在 hard 模式下至少 N=128 时 > 1.5

**理想输出**：
- delta/nodelta 的 ratio 曲线（N = 2, 8, 32, 128）：预期从 1.00 单调增长到 2×+
- RucksFS-delta 在所有 SUT 中 create + remove 吞吐最高
- easy 模式下各 SUT 接近（证明 delta 优势只在 hard 时出现，符合设计意图）
- 有数据（-w 4096）vs 无数据的差异：预期 DS 的 flush 开销让整体慢 10-30%，不影响 SUT 相对关系

## 9. 风险与对策

| 风险 | 对策 |
|---|---|
| 某台 client 创建失败 | terraform 支持重建；允许 N-1 跑 |
| mdtest 在 N=128 hard 时跑崩 | 减少 files/rank 到 200 |
| JuiceFS + TiKV 调试卡住 | 优先跑 RucksFS + NFS，JuiceFS 作为独立阶段 |
| SSH mesh 在 128 台上太慢 | 用 pdsh 或并行 scp/ssh |
| 预算超支 | 数据每阶段 destroy 集群，下次从 terraform 快速重建 |

## 10. 不做的事

- **不跑 3 iterations 取平均**：时间成本太高，1 iteration 加事后方差分析已够
- **不测 bonnie++/dbench/iozone**：这些是数据面 benchmark，和我们元数据主线关系小
- **不测 rename/fsync/large directory scan**：论文主线聚焦 create/stat/remove，这些作为 Future work
- **不测多 server**：单 MDS + 单 DS 是本文 scope，多副本/分片属于下游扩展
