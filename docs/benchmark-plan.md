# RucksFS 性能测试方案

## 1. 测试目标

验证 RucksFS 在元数据操作上的性能，与业界主流方案做横向对比，证明架构设计的合理性。

测试分两个维度：

| 维度 | 工具 | 目的 |
|------|------|------|
| **正确性** | pjdfstest (8800+ tests) | POSIX 合规性验证 |
| **性能** | mdtest + rucksfs-bench | 元数据 ops/sec，横向对比 |

---

## 2. 对比对象

### 必选对比项

| 对比对象 | 为什么选它 | 代表什么 |
|---------|-----------|---------|
| **ext4 (本地)** | 单机本地文件系统的性能天花板 | 基线参照 |
| **JuiceFS + Redis** | 同赛道的分布式元数据文件系统 | 直接竞品 |
| **RucksFS 嵌入式模式** | 单进程，无网络开销 | 自身理论上限 |
| **RucksFS 分布式模式** | 通过 gRPC 通信 | 实际部署性能 |

### 可选对比项（如有条件）

| 对比对象 | 说明 |
|---------|------|
| CephFS | 分布式文件系统，元数据性能一般但知名度高 |
| JuiceFS + TiKV | JuiceFS 的分布式元数据引擎方案 |
| NFS (kernel nfsd + ext4) | 传统网络文件系统基线 |

---

## 3. 测试工具

### 3.1 mdtest（业界标准元数据 benchmark）

mdtest 是 HPC 领域的标准元数据性能测试工具，由 LLNL 开发，现属于 IOR 项目。
JuiceFS、CephFS、Lustre 的官方 benchmark 数据都是用 mdtest 出的，审稿人和评委都认。

**安装方法**：

```bash
# 安装依赖
sudo apt install -y mpich build-essential automake

# 编译 IOR + mdtest
git clone https://github.com/hpc/ior.git
cd ior
./bootstrap
./configure --prefix=/usr/local/ior
make -j$(nproc)
sudo make install

# 验证
mdtest -h
```

**测试的操作**：

| 操作 | 含义 |
|------|------|
| Directory creation | mkdir 性能 |
| Directory stat | 目录 getattr 性能 |
| Directory removal | rmdir 性能 |
| File creation | create + close 性能 |
| File stat | 文件 getattr 性能 |
| File read | open + read + close 性能 |
| File removal | unlink 性能 |
| Tree creation/removal | 递归创建/删除目录树 |

### 3.2 rucksfs-bench（自研工具）

项目已有的 Rust 原生 benchmark 工具（`benchmark/bench-tool/`）。

优势：
- 直接测 FUSE 挂载点，不需要 MPI
- 支持 easy/hard 两种模式（私有目录 vs 共享目录）
- CSV 输出，方便自动化对比
- 可以跑 chained 模式（create→stat→rename→unlink 全链路）

**两个工具互补**：rucksfs-bench 用于快速内部迭代找瓶颈，mdtest 出最终论文数据。

### 3.3 pjdfstest（正确性）

```bash
# 安装
git clone https://github.com/pjd/pjdfstest.git
cd pjdfstest
autoreconf -ifs
./configure
make

# 运行（在 FUSE 挂载点下）
cd /mnt/rucksfs
prove -rv /path/to/pjdfstest/tests/
```

---

## 4. 机器需求

### 最小配置（2 台机器）

```
┌─────────────────────────────────────────────────────────────┐
│  机器 A: 客户端 + 测试驱动                                     │
│  角色: 运行 mdtest / rucksfs-bench / pjdfstest                │
│  配置: 4 core CPU, 8GB RAM, SSD                              │
│  软件: FUSE, MPI (mpich), mdtest, pjdfstest                  │
│                                                              │
│  同时也部署:                                                   │
│  - ext4 本地测试 (不挂 FUSE, 直接 mdtest → 本地目录)            │
│  - JuiceFS client (FUSE mount)                               │
│  - RucksFS client (FUSE mount, 分布式模式)                     │
│  - RucksFS embedded (FUSE mount, 一体化模式)                   │
└─────────────────────────────────────────────────────────────┘
        │
        │ gRPC / JuiceFS 协议
        ▼
┌─────────────────────────────────────────────────────────────┐
│  机器 B: 服务端                                               │
│  角色: 运行各个文件系统的后端                                    │
│  配置: 4 core CPU, 8GB RAM, SSD                              │
│  软件:                                                       │
│  - RucksFS MetadataServer + DataServer                       │
│  - Redis (for JuiceFS metadata)                              │
│  - MinIO / 本地 S3 模拟 (for JuiceFS data)                    │
│  - JuiceFS format + mount helper                             │
└─────────────────────────────────────────────────────────────┘
```

### 推荐配置（3 台机器，更贴近生产）

```
机器 A: 纯客户端 (mdtest, FUSE mounts)
机器 B: RucksFS MetadataServer + JuiceFS Redis
机器 C: RucksFS DataServer + JuiceFS MinIO/S3
```

### 硬件要求

| 项目 | 最低要求 | 推荐 |
|------|---------|------|
| CPU | 4 core | 8 core |
| 内存 | 8 GB | 16 GB |
| 磁盘 | SSD 100GB | NVMe SSD 200GB |
| 网络 | 1 Gbps（同局域网） | 10 Gbps 或同机房内网 |
| OS | Ubuntu 22.04+ | Ubuntu 22.04 LTS |

**关键**：所有机器必须在同一局域网，网络延迟 < 1ms，否则网络延迟会淹没文件系统本身的性能差异。

---

## 5. 测试参数

### 5.1 mdtest 参数设计

为了和 JuiceFS 的公开数据可比，参数参考 JuiceFS 官方 benchmark：

```bash
# 参数组 1: 单进程基准（摸底）
mdtest -d /mnt/<target> -i 3 -n 10000 -F -C -T -r -u

# 参数组 2: JuiceFS 标准参数（横向对比）
mdtest -d /mnt/<target> -b 6 -I 8 -z 4

# 参数组 3: 大规模文件创建
mdtest -d /mnt/<target> -n 100000 -F -C -T -r -u

# 参数组 4: 多进程并发（需 MPI）
mpirun -np 1  mdtest -d /mnt/<target> -n 10000 -F -C -T -r -u
mpirun -np 2  mdtest -d /mnt/<target> -n 10000 -F -C -T -r -u
mpirun -np 4  mdtest -d /mnt/<target> -n 10000 -F -C -T -r -u
mpirun -np 8  mdtest -d /mnt/<target> -n 10000 -F -C -T -r -u
```

**参数说明**：

| 参数 | 含义 | 值 |
|------|------|---|
| `-d` | 测试目录 | 各文件系统的挂载点 |
| `-n` | 每进程文件数 | 10000（基准）/ 100000（大规模） |
| `-i` | 迭代次数 | 3（取中位数） |
| `-F` | 只测文件操作 | 与 `-D` 互斥 |
| `-D` | 只测目录操作 | |
| `-C` | 只测 create | 可选，拆分测量 |
| `-T` | 只测 stat | |
| `-r` | 只测 remove | |
| `-u` | unique dir per task | 每进程用独立目录，减少锁争用 |
| `-b` | branching factor | 目录树分支数 |
| `-z` | depth | 目录树深度 |
| `-I` | items per dir | 每目录条目数 |

### 5.2 rucksfs-bench 参数

```bash
# 基准测试
rucksfs-bench -m /mnt/<target> -t 1 -n 10000 -o results/<target>/

# 并发 scaling 测试
rucksfs-bench -m /mnt/<target> -t 1,2,4,8 -n 10000 -o results/<target>/

# chained 全链路
rucksfs-bench -m /mnt/<target> -t 1,2,4,8 -n 10000 --mode all -o results/<target>/
```

### 5.3 pjdfstest 参数

```bash
cd /mnt/<target>
prove -rv /path/to/pjdfstest/tests/ 2>&1 | tee pjdfstest_results.txt
```

---

## 6. 部署流程

### 6.1 ext4（基线，机器 A 本地）

```bash
# 不需要额外部署，直接用本地 SSD 上的 ext4 分区
mkdir -p /tmp/ext4-bench
# 直接对本地目录跑 mdtest
mdtest -d /tmp/ext4-bench -n 10000 -F -C -T -r -u -i 3
```

### 6.2 RucksFS 嵌入式模式（机器 A 本地）

```bash
# 编译
cargo build --release -p rucksfs
# 挂载
mkdir -p /mnt/rucksfs-embedded
./target/release/rucksfs --mount /mnt/rucksfs-embedded --data-dir /tmp/rucksfs-data
# 跑测试
mdtest -d /mnt/rucksfs-embedded -n 10000 -F -C -T -r -u -i 3
```

### 6.3 RucksFS 分布式模式

```bash
# 机器 B: 启动 MetadataServer + DataServer
./rucksfs-metaserver --listen 0.0.0.0:8001 --data-dir /var/rucksfs
./rucksfs-dataserver --listen 0.0.0.0:8002 --data-dir /var/rucksfs

# 机器 A: 启动远程 client
./rucksfs-remote-client \
    --mount /mnt/rucksfs-dist \
    --meta-addr http://<机器B_IP>:8001 \
    --data-addr http://<机器B_IP>:8002

# 跑测试
mdtest -d /mnt/rucksfs-dist -n 10000 -F -C -T -r -u -i 3
```

### 6.4 JuiceFS + Redis（对比项）

```bash
# 机器 B: 部署 Redis + MinIO
sudo apt install -y redis-server
redis-server --bind 0.0.0.0 --port 6379 --daemonize yes

# MinIO (S3 兼容对象存储)
wget https://dl.min.io/server/minio/release/linux-amd64/minio
chmod +x minio
MINIO_ROOT_USER=minioadmin MINIO_ROOT_PASSWORD=minioadmin \
    ./minio server /var/minio-data --address :9000 &

# 机器 A: 安装 JuiceFS 并格式化
curl -sSL https://d.juicefs.com/install | sh -
juicefs format \
    --storage minio \
    --bucket http://<机器B_IP>:9000/jfs-bench \
    --access-key minioadmin \
    --secret-key minioadmin \
    redis://<机器B_IP>:6379/1 \
    jfs-bench

# 挂载
mkdir -p /mnt/juicefs
juicefs mount redis://<机器B_IP>:6379/1 /mnt/juicefs -d

# 跑测试
mdtest -d /mnt/juicefs -n 10000 -F -C -T -r -u -i 3
```

---

## 7. 测试执行矩阵

每个 target 跑同样的参数，结果放到统一格式里对比。

### 单进程基准

| 操作 | ext4 | RucksFS-嵌入式 | RucksFS-分布式 | JuiceFS+Redis |
|------|------|---------------|--------------|---------------|
| File creation (ops/s) | | | | |
| File stat (ops/s) | | | | |
| File removal (ops/s) | | | | |
| Dir creation (ops/s) | | | | |
| Dir stat (ops/s) | | | | |
| Dir removal (ops/s) | | | | |

### 并发 scaling（文件 create ops/s）

| 进程数 | ext4 | RucksFS-嵌入式 | RucksFS-分布式 | JuiceFS+Redis |
|--------|------|---------------|--------------|---------------|
| 1 | | | | |
| 2 | | | | |
| 4 | | | | |
| 8 | | | | |

### 正确性

| 测试套件 | RucksFS | JuiceFS |
|---------|---------|---------|
| pjdfstest 通过率 | /8800 | /8800 |
| 已知跳过项 | link, mkfifo, mknod | (查 JuiceFS 文档) |

---

## 8. 测试流程自动化脚本

建议创建一个一键跑完所有对比测试的脚本：

```bash
#!/bin/bash
# scripts/benchmark/run_comparison.sh

TARGETS=("ext4:/tmp/ext4-bench" 
         "rucksfs-embedded:/mnt/rucksfs-embedded"
         "rucksfs-dist:/mnt/rucksfs-dist"
         "juicefs:/mnt/juicefs")

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_DIR="test-results/comparison_${TIMESTAMP}"
mkdir -p "$RESULT_DIR"

for entry in "${TARGETS[@]}"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    echo "=== Testing $name at $path ==="
    
    # 1. 清理
    rm -rf "$path"/* 2>/dev/null
    
    # 2. mdtest 单进程
    mdtest -d "$path" -n 10000 -F -C -T -r -u -i 3 \
        | tee "$RESULT_DIR/${name}_mdtest_single.txt"
    rm -rf "$path"/*
    
    # 3. mdtest 多进程
    for np in 1 2 4 8; do
        mpirun -np $np mdtest -d "$path" -n 10000 -F -u -i 3 \
            | tee "$RESULT_DIR/${name}_mdtest_np${np}.txt"
        rm -rf "$path"/*
    done
    
    # 4. rucksfs-bench (如果有)
    if command -v rucksfs-bench &>/dev/null; then
        rucksfs-bench -m "$path" -t 1,2,4,8 -n 10000 \
            -o "$RESULT_DIR/${name}_bench/"
        rm -rf "$path"/*
    fi
done

echo "Results saved to $RESULT_DIR"
```

---

## 9. 预期结论（根据架构分析）

| 场景 | 预期结果 |
|------|---------|
| ext4 vs RucksFS-嵌入式 | ext4 快 2-5x（无 FUSE 开销、无序列化） |
| RucksFS-嵌入式 vs JuiceFS | RucksFS 可能更快（RocksDB 本地 vs Redis 网络 + S3） |
| RucksFS-嵌入式 vs RucksFS-分布式 | 嵌入式快 2-10x（无 gRPC 网络开销） |
| RucksFS-分布式 vs JuiceFS | 核心对比项——同是网络文件系统，看元数据引擎效率 |
| 并发 scaling | RucksFS 的 PCC 事务在高并发下的冲突率是关键指标 |

**毕设论文的核心论点**：
1. RucksFS 嵌入式模式在元数据性能上接近或优于 JuiceFS（验证 RocksDB 元数据引擎的效率）
2. 分布式模式 gRPC 开销可控，和 JuiceFS 在同一数量级
3. PCC 事务 + delta compaction 在并发场景下的 scaling 特性

---

## 10. 注意事项

1. **预热**：每轮测试前跑一次 warmup（不计入结果），避免冷启动偏差
2. **清理**：每轮测试后清空测试目录，确保不受残留数据影响
3. **迭代**：mdtest 用 `-i 3`，取中位数，排除偶然波动
4. **drop caches**：每轮测试前执行 `echo 3 > /proc/sys/vm/drop_caches`，避免 page cache 干扰
5. **固定 CPU 频率**：`cpupower frequency-set -g performance`，避免动态调频影响结果
6. **关闭无关服务**：测试期间关闭 cron、apt 自动更新等后台服务
7. **记录环境**：保存 `uname -a`、`lscpu`、`free -h`、`lsblk`、`ip a` 输出
8. **网络验证**：测试前用 `ping` 和 `iperf3` 确认网络延迟 < 1ms、带宽 >= 1Gbps

---

## 参考资料

- [mdtest (IOR project) GitHub](https://github.com/hpc/ior)
- [JuiceFS 官方 mdtest 文档](https://www.juicefs.com/docs/zh/community/mdtest)
- [JuiceFS Benchmark 数据](https://github.com/juicedata/juicefs/blob/main/docs/en/benchmark/benchmark.md)
- [pjdfstest GitHub](https://github.com/pjd/pjdfstest)
- [MDTest - Lustre Wiki](https://www.wiki.lustre.org/index.php?oldid=5315&title=MDTest)
- [BeeGFS Benchmark Guide](https://doc.beegfs.io/8.1/advanced_topics/benchmark.html)
