# RucksFS 性能测试方案

## 1. 测试目标

验证 RucksFS 在元数据操作上的性能，与业界主流方案做横向对比，证明架构设计的合理性。

测试分两个维度：

| 维度 | 工具 | 目的 |
|------|------|------|
| **正确性** | pjdfstest (8800+ tests) | POSIX 合规性验证 |
| **性能** | mdtest (IOR/LLNL) | 元数据 ops/sec，横向对比 |

---

## 2. 对比对象与控制变量

### 对比原则

不同层次的文件系统之间不能直接对比（ext4 本地裸跑 vs FUSE 网络文件系统没有意义）。
必须控制变量，按层次分组：

```
层次 1（核心对比）: 都走 FUSE + 都走网络 + 都持久化到磁盘 → 变量只有元数据引擎
层次 2（内存优势量化）: JuiceFS+Redis vs JuiceFS+MySQL → 变量只有内存 vs 磁盘
层次 3（网络开销量化）: RucksFS 嵌入式 vs 分布式 → 变量只有函数直调 vs gRPC
层次 4（参照线）:   ext4 本地 / NFS+ext4 → 标注为 reference，不做直接对比
```

### 正式对比矩阵

| 对比组 | 对象 A | 对象 B | 控制变量 | 证明什么 |
|--------|--------|--------|---------|---------|
| **核心对比** | RucksFS 分布式 (FUSE+gRPC+RocksDB) | JuiceFS + MySQL (FUSE+网络+磁盘持久化) | 都走 FUSE + 网络，都用磁盘持久化元数据引擎 | RocksDB vs MySQL 元数据引擎的竞争力 |
| **内存优势量化** | JuiceFS + Redis | JuiceFS + MySQL | 同一 JuiceFS 代码，变量是内存引擎 vs 磁盘引擎 | 内存型引擎的加速比（帮助读者理解公平性） |
| **网络开销量化** | RucksFS 嵌入式 | RucksFS 分布式 | 同一代码，变量是函数直调 vs gRPC | gRPC 通信开销可控 |
| **传统网络FS参照** | NFS (kernel nfsd + ext4) | — | 内核态 NFS，无 FUSE | 传统方案的性能参照线 |
| **本地天花板参照** | ext4 (本地, 无 FUSE, 无网络) | — | 不参与任何对比 | 图表中标注为 "local ext4 ceiling"，让读者理解上限 |

### 为什么用 JuiceFS+MySQL 而非 JuiceFS+Redis 做核心对比？

| 维度 | Redis | MySQL | RocksDB (RucksFS) |
|------|-------|-------|-------------------|
| 数据持久化 | 默认异步 (AOF/RDB) | WAL + B-Tree，同步写 | WAL + LSM-Tree，同步写 |
| 存储介质 | **内存**为主 | **磁盘**为主 | **磁盘**为主 |
| 延迟量级 | µs 级 | ms 级 | ms 级 |

Redis 的内存操作天然比磁盘快 1-2 个数量级，直接对比不公平。
JuiceFS+MySQL 与 RucksFS+RocksDB 都是磁盘持久化引擎，延迟在同一量级，是最公平的对比基础。

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

### 3.2 pjdfstest（正确性验证）

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

## 4. 测试环境：腾讯云 CVM 配置

### 4.1 机器规划（3 台）

```
┌──────────────────────────────────────────────────────────────────┐
│  机器 A: 测试驱动 + 客户端                                         │
│  角色: 运行 mdtest / pjdfstest，承载所有 FUSE 挂载点                 │
│                                                                   │
│  部署:                                                            │
│  - ext4 本地测试 (直接 mdtest → 本地 SSD 目录)                      │
│  - RucksFS embedded (FUSE mount, 一体化模式)                       │
│  - RucksFS distributed client (FUSE mount → 机器 B)               │
│  - JuiceFS client (FUSE mount → 机器 B Redis/MySQL + 机器 C MinIO) │
│  - NFS client (mount → 机器 C nfsd)                               │
│                                                                   │
│  软件: FUSE, MPI (mpich), mdtest, pjdfstest, JuiceFS CLI, NFS    │
│        client, RucksFS binaries                                   │
└──────────────────────────────────────────────────────────────────┘
        │
        │ gRPC / MySQL / Redis / NFS / JuiceFS 协议
        ▼
┌──────────────────────────────────────────────────────────────────┐
│  机器 B: 元数据服务端                                               │
│  角色: 运行所有元数据引擎后端                                         │
│                                                                   │
│  部署:                                                            │
│  - RucksFS MetadataServer (gRPC :8001)                            │
│  - MySQL 8.0 (for JuiceFS, :3306)                                 │
│  - Redis 7.x (for JuiceFS 内存对比, :6379)                         │
│                                                                   │
│  关键: 所有元数据引擎运行在同一台机器上，硬件条件完全一致               │
└──────────────────────────────────────────────────────────────────┘
        │
        │ gRPC / S3 / NFS
        ▼
┌──────────────────────────────────────────────────────────────────┐
│  机器 C: 数据服务端                                                 │
│  角色: 运行所有数据存储后端                                          │
│                                                                   │
│  部署:                                                            │
│  - RucksFS DataServer (gRPC :8002)                                │
│  - MinIO (S3 兼容, for JuiceFS, :9000)                             │
│  - NFS Server (kernel nfsd + ext4, :2049)                         │
│                                                                   │
│  关键: 所有数据服务运行在同一台机器上                                  │
└──────────────────────────────────────────────────────────────────┘
```

### 4.2 腾讯云 CVM 购买规格

| 机器 | 规格 | 实例类型建议 | 系统盘 | 数据盘 | 说明 |
|------|------|------------|--------|--------|------|
| A (客户端) | 8 核 16GB | SA3.2XLARGE16 (标准型) | 50GB 高性能云硬盘 | 200GB SSD 云硬盘 | 运行 mdtest + FUSE 挂载，8 核确保多进程测试不成瓶颈 |
| B (元数据) | 8 核 16GB | SA3.2XLARGE16 (标准型) | 50GB 高性能云硬盘 | 200GB SSD 云硬盘 | MySQL + Redis + RucksFS MetaServer 共用，16GB 够 Redis 缓存 |
| C (数据) | 4 核 8GB | SA3.XLARGE8 (标准型) | 50GB 高性能云硬盘 | 500GB SSD 云硬盘 | MinIO + DataServer + NFS 共用，数据盘大一些 |

**关键配置项**：

| 项目 | 要求 |
|------|------|
| **地域** | 同一可用区（如广州三区） |
| **VPC** | 三台机器在同一 VPC 子网 |
| **安全组** | 互相放通全部端口（或至少 8001, 8002, 3306, 6379, 9000, 2049） |
| **操作系统** | Ubuntu 22.04 LTS |
| **网络** | 内网带宽 ≥ 3Gbps（SA3 默认即可），延迟 < 0.3ms |
| **购买方式** | 按量计费（测试完即释放，预计测试 2-3 天） |

**预估费用**（按量计费，广州区参考价）：

| 机器 | 单价约 | 3 天费用约 |
|------|--------|-----------|
| A: 8C16G + 200G SSD | ~1.5 元/小时 | ~108 元 |
| B: 8C16G + 200G SSD | ~1.5 元/小时 | ~108 元 |
| C: 4C8G + 500G SSD | ~1.0 元/小时 | ~72 元 |
| **合计** | | **~288 元** |

> 提示：如有教育优惠或新用户优惠券可进一步降低成本。也可用竞价实例（Spot）节省 50-80%。

### 4.3 网络验证

测试开始前必须验证网络质量：

```bash
# 在机器 A 上执行
# 1. 延迟验证（预期 < 0.3ms）
ping -c 100 <机器B内网IP>
ping -c 100 <机器C内网IP>

# 2. 带宽验证（预期 > 3Gbps）
# 机器 B/C 上: iperf3 -s
iperf3 -c <机器B内网IP> -t 30 -P 4
iperf3 -c <机器C内网IP> -t 30 -P 4
```

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

### 5.2 pjdfstest 参数

```bash
cd /mnt/<target>
prove -rv /path/to/pjdfstest/tests/ 2>&1 | tee pjdfstest_results.txt
```

---

## 6. 部署流程

### 6.1 基础环境（所有机器）

```bash
sudo apt update && sudo apt upgrade -y
sudo apt install -y build-essential git curl wget

# 记录环境信息
uname -a > env_info.txt
lscpu >> env_info.txt
free -h >> env_info.txt
lsblk >> env_info.txt
ip a >> env_info.txt
```

### 6.2 机器 A — 客户端 + 测试驱动

```bash
# mdtest
sudo apt install -y mpich automake
git clone https://github.com/hpc/ior.git
cd ior && ./bootstrap && ./configure --prefix=/usr/local/ior
make -j$(nproc) && sudo make install

# pjdfstest
git clone https://github.com/pjd/pjdfstest.git
cd pjdfstest && autoreconf -ifs && ./configure && make

# FUSE
sudo apt install -y fuse3 libfuse3-dev
echo "user_allow_other" | sudo tee -a /etc/fuse.conf

# JuiceFS client
curl -sSL https://d.juicefs.com/install | sh -

# NFS client
sudo apt install -y nfs-common

# RucksFS (从源码编译)
# 需要先安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
cd /path/to/rucksfs
cargo build --release -p rucksfs

# ext4 本地测试目录
mkdir -p /tmp/ext4-bench
```

### 6.3 机器 B — 元数据服务端

```bash
# MySQL 8.0
sudo apt install -y mysql-server
sudo systemctl start mysql
# 配置：
# - bind-address = 0.0.0.0
# - 创建 juicefs 用户和数据库
sudo mysql -e "CREATE DATABASE juicefs;"
sudo mysql -e "CREATE USER 'juicefs'@'%' IDENTIFIED BY 'juicefs_bench';"
sudo mysql -e "GRANT ALL ON juicefs.* TO 'juicefs'@'%';"
sudo mysql -e "FLUSH PRIVILEGES;"

# Redis
sudo apt install -y redis-server
# 配置 bind 0.0.0.0
sudo sed -i 's/^bind .*/bind 0.0.0.0/' /etc/redis/redis.conf
sudo systemctl restart redis

# RucksFS MetadataServer
# 将编译好的 binary 传到机器 B
./rucksfs-metaserver --listen 0.0.0.0:8001 --data-dir /data/rucksfs-meta
```

### 6.4 机器 C — 数据服务端

```bash
# MinIO
wget https://dl.min.io/server/minio/release/linux-amd64/minio
chmod +x minio
MINIO_ROOT_USER=minioadmin MINIO_ROOT_PASSWORD=minioadmin \
    ./minio server /data/minio --address :9000 &

# NFS server
sudo apt install -y nfs-kernel-server
sudo mkdir -p /data/nfs-export
echo "/data/nfs-export *(rw,sync,no_subtree_check,no_root_squash)" | sudo tee -a /etc/exports
sudo exportfs -rav
sudo systemctl restart nfs-kernel-server

# RucksFS DataServer
./rucksfs-dataserver --listen 0.0.0.0:8002 --data-dir /data/rucksfs-data
```

### 6.5 机器 A — 挂载各文件系统

```bash
# 1. RucksFS 嵌入式（一体化模式，本地 FUSE）
mkdir -p /mnt/rucksfs-embedded
./rucksfs --mount /mnt/rucksfs-embedded --data-dir /data/rucksfs-local

# 2. RucksFS 分布式（远程 MetadataServer + DataServer）
mkdir -p /mnt/rucksfs-dist
./rucksfs-remote-client \
    --mount /mnt/rucksfs-dist \
    --meta-addr http://<B_IP>:8001 \
    --data-addr http://<C_IP>:8002

# 3. JuiceFS + MySQL（核心对比对象）
mkdir -p /mnt/juicefs-mysql
juicefs format \
    --storage minio \
    --bucket http://<C_IP>:9000/jfs-mysql \
    --access-key minioadmin \
    --secret-key minioadmin \
    "mysql://juicefs:juicefs_bench@(<B_IP>:3306)/juicefs" \
    jfs-mysql
juicefs mount "mysql://juicefs:juicefs_bench@(<B_IP>:3306)/juicefs" /mnt/juicefs-mysql -d

# 4. JuiceFS + Redis（内存优势量化）
mkdir -p /mnt/juicefs-redis
juicefs format \
    --storage minio \
    --bucket http://<C_IP>:9000/jfs-redis \
    --access-key minioadmin \
    --secret-key minioadmin \
    "redis://<B_IP>:6379/1" \
    jfs-redis
juicefs mount "redis://<B_IP>:6379/1" /mnt/juicefs-redis -d

# 5. NFS (传统网络文件系统参照)
mkdir -p /mnt/nfs
sudo mount -t nfs <C_IP>:/data/nfs-export /mnt/nfs

# 6. ext4 本地（天花板参照，不挂 FUSE）
mkdir -p /tmp/ext4-bench
```

---

## 7. 测试执行矩阵

每个 target 跑同样的参数，结果放到统一格式里对比。

### 7.1 单进程基准（核心对比）

| 操作 | RucksFS 嵌入式 | RucksFS 分布式 | JuiceFS+MySQL | JuiceFS+Redis | NFS+ext4 | ext4 本地 |
|------|---------------|--------------|--------------|--------------|----------|----------|
| File creation (ops/s) | | | | | | |
| File stat (ops/s) | | | | | | |
| File removal (ops/s) | | | | | | |
| Dir creation (ops/s) | | | | | | |
| Dir stat (ops/s) | | | | | | |
| Dir removal (ops/s) | | | | | | |

> **注**: ext4 本地数据仅标注为 "ceiling reference"，不参与对比分析。NFS+ext4 标注为 "traditional NFS reference"。

### 7.2 并发 scaling（文件 create ops/s）

| 进程数 | RucksFS 嵌入式 | RucksFS 分布式 | JuiceFS+MySQL | JuiceFS+Redis | NFS+ext4 |
|--------|---------------|--------------|--------------|--------------|----------|
| 1 | | | | | |
| 2 | | | | | |
| 4 | | | | | |
| 8 | | | | | |

### 7.3 正确性

| 测试套件 | RucksFS | JuiceFS+MySQL |
|---------|---------|---------------|
| pjdfstest 通过率 | /8800 | /8800 |
| 已知跳过项 | link, mkfifo, mknod | (查 JuiceFS 文档) |

### 7.4 论文图表规划

| 图编号 | 类型 | 内容 | 对比对象 |
|-------|------|------|---------|
| Fig.1 | 柱状图 | 单进程 6 种操作 ops/s | RucksFS-dist vs JuiceFS+MySQL（核心结论图） |
| Fig.2 | 折线图 | 并发 scaling (1→8 进程) | RucksFS-dist vs JuiceFS+MySQL |
| Fig.3 | 柱状图 | RucksFS 嵌入式 vs 分布式 | 量化 gRPC 网络开销 |
| Fig.4 | 柱状图 | JuiceFS+Redis vs JuiceFS+MySQL | 说明内存引擎优势，证明选 MySQL 对比更公平 |
| Fig.5 | 柱状图 | 所有系统 + NFS + ext4 参照线 | 全景图，带虚线标注 reference |
| Fig.6 | 表格 | pjdfstest 通过率 | RucksFS vs JuiceFS |

---

## 8. 测试流程自动化脚本

```bash
#!/bin/bash
# scripts/benchmark/run_comparison.sh

TARGETS=(
    "ext4:/tmp/ext4-bench"
    "rucksfs-embedded:/mnt/rucksfs-embedded"
    "rucksfs-dist:/mnt/rucksfs-dist"
    "juicefs-mysql:/mnt/juicefs-mysql"
    "juicefs-redis:/mnt/juicefs-redis"
    "nfs:/mnt/nfs"
)

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_DIR="test-results/comparison_${TIMESTAMP}"
mkdir -p "$RESULT_DIR"

# 固定 CPU 频率
sudo cpupower frequency-set -g performance 2>/dev/null

for entry in "${TARGETS[@]}"; do
    name="${entry%%:*}"
    path="${entry##*:}"
    echo "=== Testing $name at $path ==="

    # 预热
    echo "  [warmup]"
    mdtest -d "$path" -n 1000 -F -C -T -r -u > /dev/null 2>&1
    rm -rf "$path"/* 2>/dev/null

    # drop caches
    sudo sh -c "echo 3 > /proc/sys/vm/drop_caches"

    # 1. mdtest 单进程
    echo "  [single-process mdtest]"
    mdtest -d "$path" -n 10000 -F -C -T -r -u -i 3 \
        | tee "$RESULT_DIR/${name}_mdtest_single.txt"
    rm -rf "$path"/* 2>/dev/null

    # 2. mdtest 多进程 scaling
    for np in 1 2 4 8; do
        sudo sh -c "echo 3 > /proc/sys/vm/drop_caches"
        echo "  [mdtest np=$np]"
        mpirun -np $np mdtest -d "$path" -n 10000 -F -u -i 3 \
            | tee "$RESULT_DIR/${name}_mdtest_np${np}.txt"
        rm -rf "$path"/* 2>/dev/null
    done

    # 3. mdtest 目录树测试
    sudo sh -c "echo 3 > /proc/sys/vm/drop_caches"
    echo "  [tree test]"
    mdtest -d "$path" -b 6 -I 8 -z 4 -i 3 \
        | tee "$RESULT_DIR/${name}_mdtest_tree.txt"
    rm -rf "$path"/* 2>/dev/null
done

echo "=== All tests complete. Results in $RESULT_DIR ==="
```

---

## 9. 预期结论（根据架构分析）

| 场景 | 预期结果 |
|------|---------|
| **RucksFS-分布式 vs JuiceFS+MySQL** | **核心结论** — RocksDB LSM-Tree 的写入性能可能优于 MySQL B-Tree，尤其在 create/unlink 等写密集操作 |
| **JuiceFS+Redis vs JuiceFS+MySQL** | Redis 快 3-10x，说明我们选 MySQL 做对比是公平的 |
| **RucksFS-嵌入式 vs RucksFS-分布式** | 嵌入式快 2-10x，量化 gRPC 通信开销 |
| **NFS+ext4 参照线** | 内核态 NFS 在 stat 等读操作上可能很快，但 create/unlink 不一定快（ext4 journal 开销） |
| **ext4 天花板** | 比所有网络方案快 10-50x，仅作为天花板参照 |
| **并发 scaling** | RucksFS 的 PCC 事务在高并发下的冲突率是关键指标 |

**毕设论文的核心论点**：
1. RucksFS 分布式模式在元数据性能上与 JuiceFS+MySQL 处于同一数量级或更优（验证 RocksDB 元数据引擎的效率）
2. gRPC 通信开销可控（嵌入式 vs 分布式对比）
3. PCC 事务 + delta compaction 在并发场景下的 scaling 特性
4. POSIX 合规性通过 pjdfstest 验证

---

## 10. 注意事项

1. **预热**：每轮测试前跑一次 warmup（不计入结果），避免冷启动偏差
2. **清理**：每轮测试后清空测试目录，确保不受残留数据影响
3. **迭代**：mdtest 用 `-i 3`，取中位数，排除偶然波动
4. **drop caches**：每轮测试前执行 `echo 3 > /proc/sys/vm/drop_caches`，避免 page cache 干扰
5. **固定 CPU 频率**：`cpupower frequency-set -g performance`，避免动态调频影响结果
6. **关闭无关服务**：测试期间关闭 cron、apt 自动更新等后台服务
7. **记录环境**：保存 `uname -a`、`lscpu`、`free -h`、`lsblk`、`ip a` 输出
8. **网络验证**：测试前用 `ping` 和 `iperf3` 确认网络延迟 < 0.3ms、带宽 >= 3Gbps
9. **MySQL 调优**：`innodb_flush_log_at_trx_commit=1`（保持默认同步写，公平对比）
10. **Redis 持久化**：`appendonly yes`，`appendfsync everysec`（与 JuiceFS 官方推荐一致）

---

## 参考资料

- [mdtest (IOR project) GitHub](https://github.com/hpc/ior)
- [JuiceFS 官方 mdtest 文档](https://www.juicefs.com/docs/zh/community/mdtest)
- [JuiceFS Benchmark 数据](https://github.com/juicedata/juicefs/blob/main/docs/en/benchmark/benchmark.md)
- [JuiceFS MySQL 引擎](https://juicefs.com/docs/zh/community/databases_for_metadata#mysql)
- [pjdfstest GitHub](https://github.com/pjd/pjdfstest)
- [MDTest - Lustre Wiki](https://www.wiki.lustre.org/index.php?oldid=5315&title=MDTest)
- [腾讯云 CVM 实例规格](https://cloud.tencent.com/document/product/213/11518)
