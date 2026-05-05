# bench-v3: MDS 纯路径性能测试

本目录是第二轮完整基准测试（bench-v3）的脚本和结果归属目录。测试目标、论述结构、预期数据形态均在 `docs/thesis-template/BENCHMARK_RATIONALE.md` 中锁定，本文件仅记录脚本层面的使用方式和差异。

## 与 bench-v2 的差异

| 维度 | bench-v2 | bench-v3 |
|---|---|---|
| RucksFS FUSE TTL | 1 秒（entry/attr）| **0 秒**（client/src/fuse.rs 修改后编译） |
| JuiceFS 挂载参数 | 默认 | **`--attr-cache=0 --entry-cache=0 --dir-entry-cache=0 --open-cache=0`** |
| NFS 挂载参数 | `vers=4.2,noac` | `vers=4.2,noac`（无变化） |
| 每 rank 文件数 | 递减（2000/1500/1000/800/600）| **统一 2000** |
| mdtest 迭代 | `-i 1` | **`-i 3`** |
| 指标主列 | Max（summary.csv 第 3 列）| **Mean + Std Dev** |
| 测试矩阵 | 所有 SUT 全跑 N=2/8/32/64/96 | **rucksfs-delta/nodelta 跑全档；juicefs-tikv/nfs 只跑 N=2/8/32** |
| Terraform 目录 | `infra/tencent-bench/` | `infra/tencent-bench-v3/` |

## 前置条件

1. **代码改动**：`client/src/fuse.rs:20` 的 `const TTL` 必须已改为 `Duration::from_secs(0)`，并重新 `cargo build --release -p rucksfs-client --bin rucksfs-remote-client`，产物分发到每个客户端节点的 `/tmp/rucksfs-remote-client`。这一步没做，测试结果不成立。
2. **基础设施**：`infra/tencent-bench-v3/` 已 `terraform apply`，97 台 VM 都完成了 cloud-init。

## 使用方式

```bash
cd testing/bench-v3

# 1. 从 Terraform 取出 IP 列表（在 infra/tencent-bench-v3/ 执行）
export SERVER_PUB=$(cd ../../infra/tencent-bench-v3 && terraform output -raw server_rucksfs_public_ip)
export SERVER_PRIV=$(cd ../../infra/tencent-bench-v3 && terraform output -raw server_rucksfs_private_ip)
export CLIENT_PUBS=$(cd ../../infra/tencent-bench-v3 && terraform output -json client_public_ips | jq -r 'join(",")')
export CLIENT_PRIVS=$(cd ../../infra/tencent-bench-v3 && terraform output -json client_private_ips | jq -r 'join(",")')

# 2. 中低并发档位（N=2, 8, 32）：跑所有四家 SUT
for n in 2 8 32; do
    pubs=$(echo "$CLIENT_PUBS"  | cut -d, -f1-$n)
    privs=$(echo "$CLIENT_PRIVS" | cut -d, -f1-$n)
    ./orchestrator.sh \
        --server-pub "$SERVER_PUB" --server-priv "$SERVER_PRIV" \
        --client-pubs "$pubs" --client-privs "$privs" \
        --suts "rucksfs-delta,rucksfs-nodelta,nfs,juicefs-tikv" \
        --modes "hard,easy" \
        --results-dir "./results-v3-n${n}"
done

# 3. 高并发档位（N=64, 96）：只跑 Delta vs NoDelta
for n in 64 96; do
    pubs=$(echo "$CLIENT_PUBS"  | cut -d, -f1-$n)
    privs=$(echo "$CLIENT_PRIVS" | cut -d, -f1-$n)
    ./orchestrator.sh \
        --server-pub "$SERVER_PUB" --server-priv "$SERVER_PRIV" \
        --client-pubs "$pubs" --client-privs "$privs" \
        --suts "rucksfs-delta,rucksfs-nodelta" \
        --modes "hard,easy" \
        --results-dir "./results-v3-n${n}"
done
```

## Sanity Check（大跑前必做）

按 `BENCHMARK_RATIONALE.md` §6.5 执行三项检查：

1. **RucksFS TTL=0 生效**：任取一个 client，`stat /mnt/sut/somefile; sleep 2; stat /mnt/sut/somefile`，在 server 的 MDS log 里确认两次 stat 都出现。
2. **JuiceFS 缓存全关**：同上。挂载后重复 stat，确认每次都穿透到 TiKV（可通过 `juicefs stats` 或 TiKV metrics 观察）。
3. **mdtest -i 3 解析通过**：跑一组 N=2 小负载，检查 summary.csv 的 Mean 和 Std Dev 两列都有值，Std Dev 在合理范围（< Mean × 10%）。

## 运行中异常信号

跑完立刻检查，不要等统一分析：

- `summary.csv` 中某行 `*_std / *_mean > 0.1`：该组不稳定，需重跑
- Delta 与 NoDelta 在 N ≤ 32 档 create/remove/stat 差距 > 5%：变量不干净，排查
- RucksFS stat < 100 ops/s：TTL 改动可能没生效或路径异常

## 结果目录结构（预期）

```
testing/bench-v3/
├── orchestrator.sh
├── README.md（本文件）
├── results-v3-n2/
│   ├── rucksfs-delta_hard_np2.txt
│   ├── rucksfs-delta_easy_np2.txt
│   ├── ... (共 8 个，四 SUT × 二 mode)
│   └── summary.csv
├── results-v3-n8/   (同上)
├── results-v3-n32/  (同上)
├── results-v3-n64/
│   ├── rucksfs-delta_hard_np64.txt
│   ├── rucksfs-delta_easy_np64.txt
│   ├── rucksfs-nodelta_hard_np64.txt
│   ├── rucksfs-nodelta_easy_np64.txt
│   └── summary.csv
└── results-v3-n96/ (同上 4 文件 + summary)
```

## 预计耗时

- 中低并发三档（四 SUT × 二 mode × 三档）：约 35-40 分钟集群时间
- 高并发两档（两 SUT × 二 mode × 两档）：约 15-20 分钟集群时间
- 加上 sanity check 8 分钟、TiKV 启停等额外开销
- **总计约 60-70 分钟**

## 销毁资源

测试完成、数据汇总完毕、异常信号已排除之后：

```bash
cd ../../infra/tencent-bench-v3
terraform destroy
```
