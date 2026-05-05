# RucksFS Benchmark v3 — Tencent Cloud Infrastructure

本目录是 **bench-v3 实验专用** 的 Terraform 配置，复制自同级 `../tencent-bench/`。v3 与 v2 在基础设施层面完全一致（机型、镜像、AZ、SSH key、AK/SK 全部沿用），**仅在 `name_prefix` 上加 `-v3` 后缀以避免资源命名冲突**；客户端数量 `num_clients` 默认值改为 96，对应 v3 测试矩阵最高并发档位。

bench-v3 的测试目标与 v2 不同：v3 关闭三套被测系统的客户端 VFS 缓存，测量 MDS 纯路径性能。详细测试方案见 `docs/thesis-template/BENCHMARK_RATIONALE.md`。

## 与 v2 相比的差异

| 项目 | v2 (`tencent-bench/`) | v3 (`tencent-bench-v3/`) |
|---|---|---|
| `name_prefix` | `rucksfs-bench` | `rucksfs-bench-v3` |
| `num_clients` 默认值 | 6 | 96 |
| 其余字段（region、AZ、image_id、instance types、SSH key、AK/SK、VPC/subnet CIDR、security group 规则等） | — | 完全一致，未修改 |

## 架构（与 v2 相同）

```
Client fleet (×96, SA5.MEDIUM2, 2C2G)    Server (SA5.16XLARGE256, 64C256G)
┌──────────────────────────┐             ┌─────────────────────────────┐
│ mdtest + OpenMPI         │── gRPC ────→│ RucksFS MDS (8001)           │
│ FUSE mount (/mnt/sut)    │── gRPC ────→│ RucksFS DataServer (8002)    │
│ NFS client               │── NFS  ────→│ NFS kernel server (2049)     │
│ JuiceFS client           │── TiKV ────→│ TiKV + PD (2379/20160)       │
└──────────────────────────┘             └─────────────────────────────┘
                  All in ap-hongkong-2, same VPC
```

## 为什么选香港节点

v2 验证过香港二区（`ap-hongkong-2`）的外网能稳定下载 TiUP、JuiceFS、ior/mdtest 等工具包，国内节点在下载 GitHub release 资源时不稳定。v3 沿用此配置，不做更换。

## 使用方式（与 v2 一致）

```bash
cd infra/tencent-bench-v3

# 1. 初始化（首次）
terraform init

# 2. Review
terraform plan

# 3. Apply（3-5 分钟）
terraform apply

# 4. 等待 cloud-init 完成（5-10 分钟）
ssh -i ./shunjiecuitest.pem ubuntu@<client_public_ip> 'cloud-init status --wait'

# 5. 取输出，跑 bench-v3 orchestrator
terraform output

# 6. 跑完销毁
terraform destroy
```

## 凭证与密钥

- AK/SK 写在 `terraform.tfvars`（不提交到 git）
- SSH key `skey-0z1o99p5` 对应本目录下的 `shunjiecuitest.pem`
- 两者均与 v2 相同，直接复用

## 销毁

测试完成后运行 `terraform destroy` 释放 97 台 VM，否则按小时继续计费。
