# 本科毕业设计（论文）开题报告

---

**课题名称：** 采用元数据键值对存储的文件操作设计与实现

**学生姓名：**

**指导教师：**

**日期：** 2026年3月

---

## 一、课题来源、目的与意义

### 1.1 课题来源

本课题来源于国家自然科学基金项目"面向新型混合存储的高效异构融合系统架构及机制"（62172175）以及国家重点研发计划"ZB 级海量冷数据存储架构及高效存储管理系统"（2024YFB4505105）。

随着云计算、大数据和人工智能应用的快速发展，现代存储系统面临前所未有的元数据管理压力。据 Meta 2020 年公开的数据中心工作负载分析，元数据操作占据了超过 50% 的总 I/O 操作量；在海量小文件场景下（如 AI 训练数据集、容器镜像、日志采集等），元数据开销更是高达总延迟的 40%。传统文件系统（ext4、XFS 等）的元数据管理方式已难以满足高并发、低延迟的性能需求。近年来，JuiceFS、CephFS、Hadoop Ozone 等工业级系统纷纷采用键值（Key-Value, KV）存储引擎管理元数据，证明了这一技术路线的可行性。本课题旨在设计并实现文件元数据的键值对存储方案和相应的 POSIX 文件操作函数。

### 1.2 研究目的

本课题的核心目的是：设计并实现文件元数据的键值对存储和相应的主要 POSIX 文件操作函数。具体而言：

1. **设计文件命名空间到 KV 存储的映射方案。** 将文件系统的层级命名空间（目录树、inode 属性、目录条目等）高效地映射到 KV 存储的扁平键空间上。

2. **实现标准 POSIX 文件操作语义。** 在 KV 存储映射基础上，实现 lookup、getattr、setattr、readdir、create、mkdir、unlink、rmdir、rename、open、read、write 等标准文件操作，支持通过 Linux FUSE 进行挂载和透明访问。

3. **构建可运行的原型系统。** 基于 Rust 语言实现一个用户态文件系统 RucksFS，在真实 Linux 环境中运行和测试，并在实际硬件平台上评价整体性能。

### 1.3 研究意义

**理论意义：** 传统文件系统使用 B-tree 或 B+ tree 管理元数据，在大量小文件创建/删除等场景下存在严重的随机 I/O 瓶颈。以 ext4 为例，一次 create 操作需修改 inode 位图、inode 表、目录 HTree、Journal 等 4~6 个分散的磁盘区域，全部为随机写。而 LSM-tree（Log-Structured Merge Tree）作为 KV 存储的核心数据结构，将随机写入转换为顺序追加写入，理论上可大幅提升元数据写入性能。本课题通过实际系统验证这一理论优势，为文件系统元数据管理提供新的技术路径。

**实践意义：** 当前工业界对基于 KV 存储的文件元数据管理有强烈需求，但这些系统设计复杂度极高，缺乏对"KV 存储如何承载 POSIX 语义"这一核心问题的系统性分析。本课题通过从零构建一个完整原型系统，为理解和优化 KV 元数据管理提供参考实现。

---

## 二、国内外研究现况及发展趋势

### 2.1 基于 KV 存储的文件元数据管理

利用 KV 存储管理文件系统元数据的研究始于 2013 年卡内基梅隆大学的 TableFS 系统，此后十余年间形成了从学术原型到超大规模生产系统的完整技术演进链。

**学术验证阶段（2013—2017）：** TableFS（ATC'13, CMU）首次将 LevelDB 嵌入本地文件系统管理全量元数据，100 万文件创建比 ext4 快 5 倍，证明了 KV 存储替代传统文件系统管理元数据的可行性。IndexFS（SC'14, CMU）将这一理念扩展到分布式场景，通过 GIGA+ 哈希分裂算法支持 128 个 MDS 节点的水平扩展。LocoFS（SC'17）通过目录元数据与文件属性的松耦合解聚合，将 KV 引擎利用率从 18% 提升到 93%。

**工业落地阶段（2018—2020）：** JuiceFS 使用 Redis/TiKV 管理元数据，在生产环境中广泛验证了 KV 元数据管理的可行性。Hadoop Ozone 用三级 RocksDB 突破了 HDFS 10 亿文件限制。Yang 等人的 BFO（Batch-File Operations）研究深入分析了批量文件操作中的元数据开销，发现 80% 的小文件操作目标小于 32 字节，元数据开销占总延迟的 40%，这一发现直接启发了本课题中 WriteBatch 批量提交机制的设计。

**极致优化阶段（2023—2025）：** SingularFS（ATC'23, 清华大学）证明精心优化的单机元数据服务器性能可超过多节点分布式方案，单 MDS 达 8.36M IOPS，打破了"必须分布式"的思维定势——这对本课题的启示是，在走向分布式前应充分挖掘单机性能潜力。Mantle（SOSP'25）提出 Delta Record 追加更新 + IndexNode 内存缓存层，避免高频属性的读-改-写模式，吞吐提升最高 115 倍。本课题的 Delta 增量更新机制直接受 Mantle 启发。

### 2.2 发展趋势

综合以上分析，文件元数据 KV 存储技术呈现以下趋势：（1）事务化与原子性保证成为标配，从简单 KV 读写演进到 WriteBatch 局部事务保证；（2）元数据与数据分离架构成为主流，实现独立优化和扩展；（3）增量更新替代读-改-写，通过追加式 Delta 记录减少写放大；（4）单机性能挖掘仍有巨大空间，SingularFS 等系统证明单机可达十亿级文件管理。

---

## 三、预计达到的目标、关键理论和技术、主要研究内容、完成课题的方案及主要措施

### 3.1 预计达到的目标

能够通过标准 POSIX 文件接口进行文件/目录访问，并进一步优化整体性能。具体包括：

1. 完成文件命名空间到 KV 存储的完整映射设计与实现，覆盖 inode 元数据、目录条目、增量日志等核心概念。

2. 实现 22 种 FUSE 文件操作，覆盖 create、mkdir、lookup、getattr、setattr、readdir、unlink、rmdir、rename、open、read、write、flush、fsync、link、symlink、readlink、statfs、access、mknod、fallocate、release 等标准操作。

3. 构建事务化元数据管理引擎，所有元数据写操作具备原子性保证。

4. 在实际硬件平台上进行性能测试和评价，对比 KV 存储方案与 ext4 等传统文件系统的性能差异。

### 3.2 关键理论和技术

**（1）LSM-tree 理论。** LSM-tree 是 RocksDB 的核心数据结构，写入先进入内存 MemTable，满后刷盘为有序 SSTable 文件，后台 Compaction 合并多层 SSTable。其核心优势在于将随机写转换为顺序写，而元数据操作以写入为主（创建、修改占 60% 以上），单个 inode 体积小（约 100 字节）适合 MemTable 聚合，目录遍历可利用 SSTable 有序性进行前缀扫描。

**（2）层级 KV 映射方案。** 将文件系统层级命名空间映射到扁平 KV 空间有两种主流方案：扁平 KV（完整路径做 Key）和层级 KV（parent_inode + name 做 Key）。本课题采用层级方案，使用两个 Column Family 分别存储 inode 属性和目录条目。其核心优势在于：rename 目录为 O(1) 操作（扁平方案为 O(N)）、readdir 可精确前缀扫描、Key 存储紧凑（固定 8 字节 + 文件名）。

**（3）RocksDB PCC 事务。** 元数据操作（如 create、rename）通常涉及多个 KV 键的联合修改，需要事务保证原子性。本课题采用 RocksDB 的 Pessimistic Concurrency Control 事务模式，所有写操作先缓存在 WriteBatch 中，commit 时一次性原子写入。跨 Column Family 同样保证原子性。通过 GetForUpdate 获取行级排他锁，加锁失败立即返回 Busy，封装 execute_with_retry 自动重试。

**（4）Delta 增量更新。** 受 Mantle（SOSP'25）启发，对于高频修改的父目录属性（nlink、mtime、ctime），不执行传统的加锁-读-改-写路径，而是向独立的 delta_entries 列族追加 5~9 字节的增量记录。读取时将基础值与 Delta 折叠合并（Fold-on-Read），后台 Compaction Worker 定期将 Delta 合并回基础值。这一设计将父目录更新从事务关键路径中移除，消除同目录操作的串行化瓶颈。

**（5）FUSE 技术。** FUSE（Filesystem in Userspace）是 Linux 内核提供的用户态文件系统框架，通过 /dev/fuse 设备将 VFS 调用转发到用户进程。本课题使用 Rust 的 fuser crate 实现 FUSE 适配层，将系统的 VfsOps 接口暴露为标准 Linux 文件系统挂载点。

### 3.3 主要研究内容

**（1）调研现有采用键值对存储管理元数据的文件系统现状。** 系统性研读 TableFS、IndexFS、LocoFS、SingularFS、BFO、Mantle 等代表性工作，分析各系统的键空间设计、编码格式、查询效率和性能表现，建立技术全景图。

**（2）设计并实现采用键值对管理文件元数据的方法，文件命名空间到键值对存储的映射。** 设计 5 类键编码方案（Inode 键、目录条目键、Delta 条目键、Chunk 条目键、系统键），采用类型前缀 + 大端序编码策略，保证数值序等于字典序。通过 RocksDB 的 Column Family 机制将不同类型的元数据隔离存储，实现高效的前缀扫描和范围查询。

**（3）设计并实现常用 POSIX 文件存取接口及函数，内部调度键值对存储接口存取其元数据。** 实现完整的 POSIX 文件操作语义，每个写操作遵循 begin_txn → mutate → commit 的事务模式。重点解决以下技术难点：create 的目录条目存在性检查与原子插入；rename 的跨目录原子移动（按 ID 排序加锁防死锁）；rmdir 的非空检查与 TOCTOU 竞态防护；unlink 的 nlink 引用计数管理。所有父目录属性更新通过 Delta 机制在事务外追加完成。

**（4）在实际硬件平台上测试和评价整体性能。** 设计包含三个层次的测试方案：使用 pjdfstest 标准套件验证 POSIX 正确性；使用 Rust 单元/集成测试覆盖正常和异常路径；使用自研元数据压测工具和 mdtest 标准工具进行性能评测，在 10K/100K/1M 文件梯度和 1/2/4/8 线程并发下与 ext4 对比。

### 3.4 完成课题的方案及主要措施

**技术方案：** 系统使用 Rust 语言开发，采用 Cargo Workspace 组织为多个独立 crate，实现关注点分离。整体架构为单进程 FUSE 文件系统：用户程序通过 POSIX syscall 进入内核 VFS → FUSE 内核模块 → /dev/fuse → 用户态 RucksFS 进程。RucksFS 内部分为 FuseClient（解析 FUSE 请求）、VfsCore（元数据/数据路由）、MetadataServer（事务化元数据操作、Delta 增量、LRU 缓存、后台压缩）、DataServer（文件数据 I/O）四个层次，底层分别对接 RocksDB（元数据存储）和 RawDisk（数据存储）。通过 MetadataOps / DataOps / VfsOps 三层 Trait 抽象，存储后端可独立替换。

**主要措施：**

1. **迭代式开发。** 采用增量开发模式：先实现内存后端验证 POSIX 语义正确性，再切换到 RocksDB 后端验证持久化和性能，最后实现 FUSE 集成进行端到端验证。

2. **测试驱动。** 为每个核心模块编写充分的单元测试和集成测试。当前已完成 10,000+ 行 Rust 代码和 228 个自动化测试。

3. **性能基准。** 设计标准化的元数据基准测试套件，在统一硬件环境下与 ext4 进行对比测试。

---

## 四、课题研究进度安排

| 阶段 | 时间 | 工作内容 |
|------|------|----------|
| 第一阶段 | 第7学期 11—12月 | 文献调研：研读 TableFS、IndexFS、LocoFS、SingularFS、BFO、Mantle 等论文；撰写技术调研报告；明确系统架构设计 |
| 第二阶段 | 第7学期 1—2月 | 核心实现（一）：完成 KV 编码方案设计与实现、RocksDB 存储后端、MetadataServer 基础框架、基本 POSIX 操作（create/mkdir/lookup/getattr/readdir/unlink/rmdir） |
| 第三阶段 | 第8学期 2—3月 | 核心实现（二）：完成 Delta 增量更新与后台 Compaction、rename/setattr 等复杂操作、PCC 事务化元数据管理、FUSE 客户端集成 |
| 第四阶段 | 第8学期 3—4月 | 测试与优化：部署 pjdfstest 正确性验证、自研压测工具和 mdtest 性能评测、并发安全修复、WAL 写入优化、锁粒度细化 |
| 第五阶段 | 第8学期 4—5月 | 论文撰写：整理实验数据、完成与 ext4 的性能对比分析、撰写毕业论文、准备答辩材料 |

---

## 五、主要参考文献

### 指导教师选定文献

[1] Yang Yang, Qiang Cao, Li Yang, Hong Jiang, Jie Yao. Batch-File Operations to Optimize Massive Files Accessing: Analysis, Design, and Application[J]. ACM Transactions on Storage, Vol. 16, No. 3, 2020.

[2] Jing Liu, Andrea Arpaci-Dusseau, Remzi Arpaci-Dusseau, et al. SingularFS: A Billion-Scale Distributed File System Using a Single Metadata Server[C]. Proceedings of the 2023 USENIX Annual Technical Conference (ATC '23), 2023.

[3] Xuechun Cao, Shaurya Patel, Soo-Yee Lim, Xueyuan Han, Thomas Pasquier. FetchBPF: Customizable Prefetching Policies in Linux with eBPF[C]. Proceedings of the 2024 USENIX Annual Technical Conference (ATC '24), 2024.

[4] Sandeep Kumar, Aravinda Prasad, et al. FBMM: Making Memory Management Extensible With Filesystems[C]. Proceedings of the 2024 USENIX Annual Technical Conference (ATC '24), 2024.

### 补充参考文献

[5] Kai Ren, Garth Gibson. TableFS: Enhancing Metadata Efficiency in the Local File System[C]. Proceedings of the 2013 USENIX Annual Technical Conference (ATC '13), 2013.

[6] Kai Ren, Qing Zheng, Swapnil Patil, Garth Gibson. IndexFS: Scaling File System Metadata Performance with Stateless Caching and Bulk Insertion[C]. Proceedings of the IEEE/ACM International Conference on High Performance Computing, Networking, Storage and Analysis (SC '14), 2014.

[7] Siyang Li, Youyou Lu, Jiwu Shu, Yang Hu, Tao Li. LocoFS: A Loosely-Coupled Metadata Service for Distributed File Systems[C]. Proceedings of the IEEE/ACM International Conference on High Performance Computing, Networking, Storage and Analysis (SC '17), 2017.

[8] Wenhao Lv, Qiang Cao, et al. Mantle: A Scalable Hierarchical Namespace Service for Object Storage[C]. Proceedings of the ACM SIGOPS Symposium on Operating Systems Principles (SOSP '25), 2025.

[9] Facebook Inc. RocksDB: A Persistent Key-Value Store for Fast Storage Environments[EB/OL]. https://rocksdb.org, 2013.

[10] Patrick O'Neil, Edward Cheng, Dieter Gawlick, Elizabeth O'Neil. The Log-Structured Merge-Tree (LSM-Tree)[J]. Acta Informatica, Vol. 33, No. 4, pp. 351-385, 1996.

[11] libfuse Contributors. fuser: Filesystem in Userspace (FUSE) for Rust[EB/OL]. https://github.com/cberner/fuser, 2020.
