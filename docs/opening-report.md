# 本科毕业设计（论文）开题报告

---

**课题名称：** 文件元数据对键值存储的文件操作实现

**学生姓名：**

**指导教师：**

**日期：** 2026年3月

---

## 一、课题来源、目的与意义

### 1.1 课题来源

本课题来源于文件系统领域的前沿研究方向——利用键值（Key-Value, KV）存储引擎管理文件系统元数据。随着云计算、大数据和人工智能应用的快速发展，现代存储系统面临着前所未有的元数据管理压力：单个命名空间可能包含数十亿甚至万亿级文件，传统文件系统的元数据管理方式已难以满足高并发、低延迟的性能需求。JuiceFS、CephFS、HDFS/Ozone 等工业级系统纷纷采用 KV 存储（如 RocksDB、LevelDB）作为元数据引擎，证明了这一技术路线的可行性和优越性。本课题旨在深入研究文件系统元数据到 KV 存储的映射方法，设计并实现一套完整的基于 KV 存储的 POSIX 文件操作系统。

### 1.2 研究目的

本课题的核心目的是：

1. **验证 KV 存储管理文件元数据的可行性与性能优势。** 通过设计精巧的键值编码方案，将文件系统的层级命名空间（目录树、inode 属性、目录条目等）高效地映射到 KV 存储的扁平键空间上，并在此基础上实现标准的 POSIX 文件操作语义。

2. **构建可工程化的原型系统。** 基于 Rust 语言实现一个模块化的用户态文件系统 RucksFS，包含完整的元数据服务器、数据服务器、客户端和 FUSE 挂载层，能够在真实 Linux 环境中运行和测试。

3. **探索元数据管理的性能优化策略。** 研究增量更新（Delta）、后台压缩（Compaction）、事务化操作、Chunk/Slice 数据模型等优化手段对系统性能的影响，为后续分布式扩展奠定基础。

### 1.3 研究意义

**理论意义：**

文件系统元数据管理是操作系统和存储系统领域的核心问题之一。传统文件系统（ext4、XFS、Btrfs 等）使用 B-tree 或 B+ tree 管理元数据，在大量小文件创建/删除、高频元数据修改等场景下存在严重的随机 I/O 瓶颈。据 Meta 2020 年公开的数据中心工作负载分析，元数据操作占据了超过 50% 的总 I/O 操作量。Log-Structured Merge Tree（LSM-tree）作为 KV 存储的核心数据结构，将随机写入转换为顺序写入，理论上可将元数据写入性能提升 3~10 倍。本课题通过实际系统实现验证这一理论优势，为文件系统元数据管理提供新的技术路径。

**实践意义：**

当前工业界对基于 KV 存储的文件元数据管理有强烈需求。JuiceFS（开源云原生分布式文件系统）使用 Redis/TiKV 管理元数据，Hadoop Ozone 使用三级 RocksDB 突破 HDFS 10 亿文件限制，百度沧海 TafDB 使用类 Spanner 分布式事务数据库支撑万亿级对象。然而，这些系统的设计复杂度极高，缺乏对 "KV 存储如何承载 POSIX 语义" 这一核心问题的系统性分析。本课题通过从零构建一个完整的原型系统，为理解和优化 KV 元数据管理提供参考实现。

---

## 二、国内外研究现况及发展趋势

### 2.1 基于 KV 存储的文件元数据管理

利用键值存储管理文件系统元数据的研究始于 2013 年卡内基梅隆大学（CMU）的 TableFS 系统。此后十余年间，学术界和工业界在这一技术路线上进行了持续深入的探索，形成了从单机到分布式、从学术原型到超大规模生产系统的完整技术演进链。

**第一阶段：早期学术探索（2013—2017）**

TableFS（2013, CMU）是该领域的开山之作，首次将 LevelDB（LSM-tree KV 存储）嵌入本地文件系统用于管理全量元数据。实验表明，在元数据密集型工作负载下，TableFS 比 ext4 快 1~10 倍，100 万文件创建从 600 秒降至 120 秒。然而 TableFS 是单机方案，无法水平扩展。IndexFS（2014, CMU）将 TableFS 的理念扩展到分布式场景，提出了 GIGA+ 增量哈希分裂算法，支持多达 128 个元数据服务器（MDS）的水平扩展。但其紧耦合的目录树结构导致 KV 存储利用率仅 18%。LocoFS（2017, 清华大学）通过松散耦合架构将目录结构（DMS）和文件属性（FMS）分离，将 KV 利用率从 18% 提升到 93%。

**第二阶段：工业实践（2006—2020）**

AWS S3（2006 年发布）采用扁平命名空间的极端简化策略，对象名作为 Key、元数据作为 Value，支撑了 400+ 万亿对象的存储，但无法提供 POSIX 语义。Hadoop Ozone（2018, Apache）将 HDFS 的全内存元数据方案替换为 RocksDB 持久化，通过 OzoneManager + StorageContainerManager + DataNode 三层架构突破了 10 亿文件限制。百度沧海 TafDB（2020）走了自研分布式事务数据库的路线，通过类 Spanner 的 Multi-Raft 架构同时支撑平坦命名空间和层级命名空间，实现万亿级对象管理。

**第三阶段：极致优化（2023—2025）**

FUSEE（2023, USENIX FAST）从硬件层面入手，利用 RDMA + 内存解聚合彻底消除元数据服务器瓶颈，吞吐提升 4.5 倍。MetaHive（2024）通过物理邻近布局优化缓存局部性，减少元数据检索的额外 I/O。Mantle（2025, SOSP）综合了前面所有阶段的经验，提出两层架构（IndexNode 缓存 + TafDB 持久化），在保证强一致性的同时实现了自适应扩展，吞吐量提升最高 115 倍。

### 2.2 指导教师选定参考文献的研究分析

**[1] Yang Yang, Qiang Cao 等, Batch-File Operations to Optimize Massive Files Accessing, ACM TOS 2020**

该论文针对传统文件系统在批量小文件访问中的性能瓶颈，提出了批量文件操作（BFO）优化方案。核心设计采用两阶段访问方法——先聚合处理元数据，再批量处理数据——将序列化的单文件访问模式转换为高效的批量操作。在 ext4 上的原型实现表明，BFO 在 HDD 上可将读取性能提升 22.4 倍、写入性能提升 111.4 倍，在 SSD 上也有 1.8~2.9 倍的提升。该研究深刻揭示了元数据与数据访问模式的耦合关系：小文件场景下 80% 的桌面文件系统访问目标小于 32 字节，元数据开销占总延迟的 40%。这一发现直接启发了本课题的设计——通过 KV 存储的 WriteBatch 机制实现元数据操作的原子批量提交。

**[2] SingularFS: A Billion-Scale Distributed File System Using a Single Metadata Server, USENIX ATC 2023**

SingularFS 颠覆了"扩展元数据必须增加 MDS 数量"的传统认知，证明单个元数据服务器在极致优化下可管理十亿级文件。其关键创新包括：无日志元数据操作（消除崩溃一致性开销）、层级并发控制（最大化共享目录并行度）、以及混合 inode 分区（将时间戳与子 inode 分组，减少跨 NUMA 访问和锁争用）。SingularFS 实现了 8.36M/18.80M IOPS 的文件创建/查询性能，超过了使用 32 个 MDS 的 InfiniFS。这一结果对本课题有重要启示：单机元数据性能的优化空间远比想象中大，在走向分布式之前应充分挖掘单机性能潜力。本课题采用的 Delta 增量更新、LRU 缓存、WriteBatch 事务等设计均受到 SingularFS 的启发。

**[3] FetchBPF: Customizable Prefetching Policies in Linux with eBPF, USENIX ATC 2024**

FetchBPF 利用 eBPF 框架实现了 Linux 内核中可定制的预取策略，无需修改内核代码即可部署自定义预取算法。其引入的三个 eBPF 辅助函数（`bpf_prefetch_physical_page`、`bpf_prefetch_virtual_page`、`bpf_block_plug`）为应用感知的 I/O 优化提供了灵活手段。该工作的开销可忽略不计，性能与内核原生实现持平。对本课题的启示在于：文件系统的性能优化不仅限于数据结构层面，还可以通过内核级策略定制来实现。未来 RucksFS 可考虑利用 eBPF 实现自适应的元数据预取策略，例如根据目录访问模式预加载子目录的 inode 缓存。

**[4] FBMM: Making Memory Management Extensible With Filesystems, USENIX ATC 2024**

FBMM 提出了一种基于虚拟文件系统（VFS）层的可扩展内存管理框架，将内存管理器实现为"内存管理文件系统"（Memory Management Filesystem, MFS）。内存分配通过在 MFS 挂载目录中创建/映射文件来完成，释放通过删除文件完成——完全透明于应用程序。MFS 模块仅需 500~1500 行代码，开销 <8%（单页分配）至 <0.1%（128 页分配）。该工作的核心理念——利用文件系统抽象作为系统资源管理的统一接口——与本课题形成了有趣的对比：FBMM 用文件系统接口管理内存，而 RucksFS 用 KV 存储接口管理文件系统元数据。两者都体现了"接口适配"在系统设计中的重要价值。

### 2.3 发展趋势

综合以上分析，文件元数据 KV 存储技术呈现以下发展趋势：

1. **事务化与原子性保证成为标配。** 从 TableFS 的简单 KV 读写，到 TafDB 的分布式事务，再到 RocksDB WriteBatch 的局部事务，元数据操作的原子性保证不断增强。
2. **元数据与数据分离架构成为主流。** JuiceFS、Ozone、Mantle 等系统均采用独立的元数据引擎和数据引擎，实现独立优化和独立扩展。
3. **增量更新（Delta）替代读-改-写。** Mantle 的 Delta Record、LocoFS 的延迟更新、SingularFS 的无日志操作都在减少元数据写放大。
4. **可编程化与可扩展性。** FetchBPF 和 FBMM 代表的 eBPF/VFS 扩展方向，使得存储系统的策略定制更加灵活。
5. **单机性能挖掘仍有巨大空间。** SingularFS 证明单 MDS 可达十亿级文件管理，打破了"必须分布式"的思维定势。

---

## 三、预计达到的目标、关键理论和技术、主要研究内容、完成课题的方案及主要措施

### 3.1 预计达到的目标

1. **完成文件命名空间到 KV 存储的完整映射设计与实现。** 设计结构化的键值编码方案，将 inode 元数据、目录索引、增量日志、Chunk/Slice 数据模型等文件系统概念映射到 RocksDB 的 Column Family 体系中，保证键空间的有序性和查询效率。

2. **实现完整的 POSIX 文件操作语义。** 覆盖 `lookup`、`getattr`、`setattr`、`readdir`、`create`、`mkdir`、`unlink`、`rmdir`、`rename`、`open`、`read`、`write`、`flush`、`fsync`、`statfs` 等 13 种以上标准操作，支持通过 Linux FUSE 进行挂载和透明访问。

3. **构建事务化元数据管理引擎。** 实现统一的 StorageEngine + Transaction 抽象，所有元数据写操作具备原子性保证（基于 RocksDB WriteBatch），支持失败回滚。

4. **实现 Chunk/Slice 数据模型和延迟 GC 机制。** 文件按 64MB Chunk 分片管理元数据，支持 `open` 时返回完整的数据映射信息；文件删除采用延迟垃圾回收策略，通过后台 GcWorker 异步清理。

5. **在实际硬件平台上进行性能测试和评价。** 设计元数据密集型基准测试，对比 KV 存储方案与传统文件系统的性能差异，验证优化策略的有效性。

### 3.2 关键理论和技术

**（1）LSM-tree（Log-Structured Merge Tree）理论**

LSM-tree 是 RocksDB/LevelDB 的核心数据结构，通过将随机写入转换为顺序写入来优化写密集型工作负载。其工作原理为：写入先进入内存中的 MemTable，MemTable 满后刷盘为不可变的 SSTable 文件，后台 Compaction 定期合并多层 SSTable。LSM-tree 在元数据管理中的核心优势是：元数据操作以写入为主（创建、修改占 60%+），单个 inode 体积小（~100-200 字节）适合 MemTable 聚合，目录遍历可利用 SSTable 的有序性进行前缀扫描。

**（2）文件系统元数据编码理论**

将层级文件命名空间映射到扁平 KV 空间的核心挑战是：如何设计键编码使得 KV 存储的字典序排列自然支持文件系统的语义查询（如"列出某目录下所有子项"、"按 inode 号定位文件属性"等）。本课题采用类型前缀 + 大端序编码策略，保证数值序等于字典序，支持高效的前缀扫描和范围查询。

**（3）事务与并发控制技术**

元数据操作（如 create、rename）通常涉及多个 KV 键的联合修改（创建 inode + 插入目录项 + 更新父目录属性），需要事务保证原子性。本课题采用 RocksDB WriteBatch 实现局部事务，结合 Per-directory Mutex 实现目录级并发控制，跨目录操作通过 Lock Ordering 防止死锁。

**（4）增量更新（Delta）与后台压缩（Compaction）**

受 SingularFS 和 Mantle 的启发，本课题采用追加式增量更新策略：对于高频修改的元数据字段（如 mtime、nlink），不执行 read-modify-write，而是追加 Delta 记录。读取时将基础值与 Delta 折叠合并，后台 Compaction Worker 定期将 Delta 原子地合并到基础值中。这一设计将目录变更的写路径缩短为单次追加操作。

**（5）FUSE（Filesystem in Userspace）技术**

FUSE 是 Linux 内核提供的用户态文件系统框架，允许在用户空间实现文件系统逻辑，通过内核模块将 VFS 调用转发到用户进程。本课题使用 Rust 的 `fuser` crate 实现 FUSE 适配层，将系统的 VfsOps 接口暴露为标准的 Linux 文件系统挂载点。

### 3.3 主要研究内容

**（1）文件系统元数据的 KV 存储映射方案研究**

调研 TableFS、IndexFS、LocoFS、SingularFS、JuiceFS、Mantle 等系统的元数据 KV 编码方案，分析各方案的键空间设计、序列化格式、查询效率和空间利用率。在此基础上，设计适合 POSIX 语义的 5 类键编码方案（Inode 键、目录条目键、Delta 条目键、Chunk 条目键、PendingDelete 键）和版本化的二进制 InodeValue 序列化格式。

**（2）事务化 POSIX 文件操作的设计与实现**

设计统一的 StorageEngine + Transaction trait 抽象，在此基础上实现 13 种 POSIX 文件操作。每个写操作使用 `begin_txn() → mutate → commit()` 的事务模式，保证原子性。重点解决以下技术难点：
- `rename` 的跨目录原子移动和覆盖逻辑
- `unlink` 的 nlink 引用计数管理和延迟 GC 触发
- `setattr` 中 truncate 与文件大小变更的联动
- 高并发场景下的目录级锁和全局 inode 分配

**（3）Chunk/Slice 数据模型与数据生命周期管理**

设计文件数据的 Chunk/Slice 分片元数据模型：文件按 64MB Chunk 分片，每次写入记录 SliceInfo 到对应 Chunk 的元数据中。实现 `report_write` 的 Chunk 范围计算和 Slice 分配，以及 `open` 时的完整 Chunk 映射返回。实现延迟 GC 机制：unlink 记录 PendingDelete，后台 GcWorker 异步清理 Chunk 元数据和数据。

**（4）性能优化策略研究**

- **Delta 增量更新：** 减少高频元数据修改的写放大
- **LRU 缓存：** 服务端 InodeFoldedCache 缓存热点 inode
- **WriteBatch 批量提交：** 将多个 KV 操作合并为单次原子写入
- **后台 Compaction：** DeltaCompactionWorker 异步折叠 Delta

**（5）系统测试与性能评价**

设计包含单元测试、集成测试、并发压力测试和 FUSE E2E 测试的完整测试体系。使用自定义元数据密集型基准测试（大量文件创建/删除、目录遍历、属性修改等）评价系统性能，对比 KV 存储方案与 ext4 等传统文件系统在不同工作负载下的表现。

### 3.4 完成课题的方案及主要措施

**技术方案：**

系统使用 Rust 语言开发，采用 Cargo Workspace 组织为 7 个独立的 crate，实现关注点分离：

| 模块 | 职责 | 关键技术 |
|------|------|----------|
| `core` | 共享类型与 trait 定义 | Rust trait 抽象、泛型 |
| `storage` | StorageEngine/Transaction 抽象与实现 | RocksDB、WriteBatch、Column Family |
| `server` | MetadataServer + Compaction + GC | Delta 增量更新、LRU 缓存、事务化操作 |
| `dataserver` | DataServer 数据 I/O | RawDisk 块存储 |
| `client` | VfsCore 路由 + FUSE 适配 | fuser crate、VFS 路由 |
| `rpc` | gRPC 通信层 | tonic、protobuf、TLS |
| `demo` | 单二进制演示程序 | 嵌入式集成 |

**主要措施：**

1. **文献调研先行。** 系统性研读 TableFS、IndexFS、LocoFS、SingularFS、BFO、FetchBPF、FBMM 等代表性论文，建立技术全景图。
2. **迭代式开发。** 采用增量开发模式：先实现内存后端验证 POSIX 语义正确性，再切换到 RocksDB 后端验证持久化和性能，最后实现 FUSE 集成进行端到端验证。
3. **测试驱动。** 为每个核心模块编写充分的单元测试和集成测试，目标覆盖率 >80%。使用并发压力测试验证线程安全性。
4. **性能基准。** 设计标准化的元数据基准测试套件，在统一硬件环境下进行对比测试，确保结论的可靠性。

---

## 四、课题研究进度安排

| 阶段 | 时间 | 工作内容 |
|------|------|----------|
| 第一阶段 | 第7学期 11—12月 | 文献调研：研读 TableFS、IndexFS、LocoFS、SingularFS、BFO、FetchBPF、FBMM 等论文；撰写技术调研报告；明确系统架构设计 |
| 第二阶段 | 第7学期 1—2月 | 核心实现（一）：完成 KV 编码方案设计与实现、内存存储后端、MetadataServer 基础框架、基本 POSIX 操作（create/mkdir/lookup/getattr/readdir/unlink/rmdir） |
| 第三阶段 | 第8学期 2—3月 | 核心实现（二）：完成 RocksDB StorageEngine 实现、Delta 增量更新与 Compaction、rename/setattr 等复杂操作、Chunk/Slice 数据模型、延迟 GC 机制 |
| 第四阶段 | 第8学期 3—4月 | 系统集成：完成 FUSE 客户端集成、gRPC 通信层、单二进制 demo 模式、编写完整测试套件（单元测试 + 集成测试 + 并发压力测试） |
| 第五阶段 | 第8学期 4—5月 | 测试与优化：在实际硬件平台上进行性能测试和评价、对比实验、性能调优、完善 FUSE E2E 测试 |
| 第六阶段 | 第8学期 5—6月 | 论文撰写：整理实验数据、撰写毕业论文、准备答辩材料 |

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

[8] Juicedata Inc. JuiceFS — A POSIX-compatible Distributed File System for Cloud[EB/OL]. https://github.com/juicedata/juicefs, 2021.

[9] Apache Software Foundation. Apache Ozone — Scalable, Distributed Object Store for Hadoop[EB/OL]. https://ozone.apache.org, 2020.

[10] Wenhao Lv, Qiang Cao, et al. Mantle: A Scalable Hierarchical Namespace Service for Object Storage[C]. Proceedings of the ACM SIGOPS Symposium on Operating Systems Principles (SOSP '25), 2025.

[11] Jiacheng Shen, et al. FUSEE: A Fully Memory-Disaggregated Key-Value Store[C]. Proceedings of the 21st USENIX Conference on File and Storage Technologies (FAST '23), 2023.

[12] Google Inc. LevelDB: A Fast and Lightweight Key/Value Storage Library[EB/OL]. https://github.com/google/leveldb, 2011.

[13] Facebook Inc. RocksDB: A Persistent Key-Value Store for Fast Storage Environments[EB/OL]. https://rocksdb.org, 2013.

[14] Patrick O'Neil, Edward Cheng, Dieter Gawlick, Elizabeth O'Neil. The Log-Structured Merge-Tree (LSM-Tree)[J]. Acta Informatica, Vol. 33, No. 4, pp. 351-385, 1996.

[15] libfuse Contributors. fuser: Filesystem in Userspace (FUSE) for Rust[EB/OL]. https://github.com/cberner/fuser, 2020.
