# RucksFS 单机回归分析与下一步路线图

> **文档版本**: v2.0  
> **更新日期**: 2026-03-02  
> **目标读者**: 项目开发者、毕业设计评审人员  
> **范围**: 单机模式下的架构分析、问题诊断、优势评估、演进路线

---

## 目录

1. [当前项目架构总览](#1-当前项目架构总览)
2. [关键问题回答](#2-关键问题回答)
   - [2.1 事务机制：RocksDB 事务还是进程内锁？](#21-事务机制rocksdb-事务还是进程内锁)
   - [2.2 元数据与文件数据的关联方式](#22-元数据与文件数据的关联方式)
3. [多余组件分析](#3-多余组件分析)
4. [当前项目的优势与可复用部分](#4-当前项目的优势与可复用部分)
5. [单机路线图：FUSE 为主 + eBPF 增强](#5-单机路线图fuse-为主--ebpf-增强)
6. [优先级排序与时间估算](#6-优先级排序与时间估算)
7. [附录：代码行数统计](#附录代码行数统计)

---

## 1. 当前项目架构总览

RucksFS 采用 Rust 编写，以 KV 存储（RocksDB）作为元数据管理引擎。项目最初按分布式架构设计，当前正在向 **单机 FUSE 文件系统 + eBPF 可观测增强** 的方向重构。

### 1.1 Workspace Crate 结构

项目由 7 个 workspace crate 组成：

| Crate | 路径 | 职责 |
|-------|------|------|
| **core** | `core/` | 公共类型定义：`FileAttr`、`DirEntry`、`FsError`、trait 定义（`MetadataOps`、`DataOps`、`VfsOps`） |
| **storage** | `storage/` | 存储引擎抽象 + 多后端实现 |
| **server** | `server/` | 元数据服务器核心逻辑 |
| **dataserver** | `dataserver/` | 数据服务器（DataStore 的薄包装层） |
| **client** | `client/` | FUSE 客户端 + VFS 路由层 |
| **rpc** | `rpc/` | gRPC/protobuf 通信层（分布式遗留组件） |
| **demo** | `demo/` | 演示程序（auto/REPL/FUSE 三种模式） |

### 1.2 各 Crate 内部模块详解

#### `storage/` — 存储引擎

```
storage/src/
├── lib.rs          # trait 定义: MetadataStore, DirectoryIndex, DeltaStore, DataStore
├── encoding.rs     # KV key encoding + InodeValue serialization
├── memory.rs       # In-memory backend (BTreeMap/HashMap)
├── rawdisk.rs      # (在 dataserver 中) Raw disk data store
├── rocks.rs        # (在 dataserver 中) RocksDB backend (optional feature)
└── allocator.rs    # Inode allocator (AtomicU64 + persistence)
```

**KV Key 编码方案**（`encoding.rs`）：

| Key 前缀 | 格式 | 用途 |
|----------|------|------|
| `I\|` | `I\|<inode:8B BE>` | Inode 元数据 |
| `D\|` | `D\|<parent_ino:8B BE>\|<name>` | 目录项索引 |
| `X\|` | `X\|<inode:8B BE>\|<seq:8B BE>` | Delta 增量操作 |

#### `server/` — 元数据服务器

```
server/src/
├── lib.rs          # MetadataServer<M, I, DS>，per-directory Mutex 锁
├── cache.rs        # LRU InodeFoldedCache
├── delta.rs        # DeltaOp 增量操作 (IncrementNlink, SetMtime, SetCtime, SetAtime)
├── compaction.rs   # Background delta compaction worker
├── main.rs         # Standalone server binary entry (distributed remnant)
└── tests/
    └── integration.rs
```

#### `client/` — FUSE 客户端

```
client/src/
├── fuse.rs         # fuser::Filesystem trait 实现
├── vfs_core.rs     # VFS 路由: MetadataOps + DataOps → VfsOps
├── embedded.rs     # Embedded client (in-process direct connection)
├── lib.rs
└── main.rs
```

#### `rpc/` — gRPC 通信层（分布式遗留）

```
rpc/
├── proto/
│   ├── metadata.proto
│   └── data.proto
└── src/
    ├── metadata_client.rs / metadata_server.rs
    ├── data_client.rs / data_server.rs
    ├── auth.rs / tls.rs / framing.rs / message.rs
    └── lib.rs
```

### 1.3 数据流架构图（Embedded 模式）

Embedded 模式下，所有组件运行在同一进程中，无网络开销：

```
┌─────────────────────────────────────────────────────────────────────┐
│                         User Programs                              │
│                            │ syscall                                │
│                            ▼                                        │
│                    FUSE (/dev/fuse)                                 │
│                            │                                        │
│                            ▼                                        │
│              FuseClient<EmbeddedClient>                             │
│                     │              │                                │
│            Metadata ops        Data ops                             │
│                     │              │                                │
│                     ▼              ▼                                │
│                VfsCore ──────► VfsCore                              │
│                     │              │                                │
│                     ▼              ▼                                │
│            MetadataServer     DataServer                            │
│            ┌────────┴────────┐     └──► MemoryDataStore            │
│            │    │    │    │  │          / RawDiskDataStore          │
│            ▼    ▼    ▼    ▼  ▼                                      │
│     MetadataStore  DirIndex  DeltaStore                            │
│     (RocksDB       (RocksDB  (RocksDB                              │
│      CF:inodes)    CF:dir_   CF:delta_                             │
│                    entries)  entries)                               │
│            │                                                        │
│            ▼                                                        │
│    InodeFoldedCache (LRU memory cache)                             │
│            │                                                        │
│            ▼                                                        │
│    DeltaCompactionWorker (background thread)                       │
└─────────────────────────────────────────────────────────────────────┘
```

完整的数据流路径：

```
User Programs → syscall → FUSE (/dev/fuse) → FuseClient<EmbeddedClient>
  ├── Metadata ops → VfsCore → MetadataServer
  │     ├── MetadataStore (RocksDB CF:inodes)
  │     ├── DirectoryIndex (RocksDB CF:dir_entries)
  │     ├── DeltaStore (RocksDB CF:delta_entries)
  │     ├── InodeFoldedCache (LRU memory cache)
  │     └── DeltaCompactionWorker
  └── Data ops → VfsCore → DataServer
        └── MemoryDataStore / RawDiskDataStore
```

### 1.4 RocksDB Column Family 布局

| Column Family | Key 格式 | Value | 用途 |
|--------------|---------|-------|------|
| `inodes` | `I\|<inode:8B BE>` | `InodeValue` (57 bytes fixed) | Inode 元数据存储 |
| `dir_entries` | `D\|<parent_ino:8B BE>\|<name>` | `<child_inode:8B BE>` | 目录项索引 |
| `delta_entries` | `X\|<inode:8B BE>\|<seq:8B BE>` | `DeltaOp` (serialized) | 增量操作日志 |
| `system` | String keys | 配置值 | 系统配置（如 `next_inode`） |

---

## 2. 关键问题回答

### 2.1 事务机制：RocksDB 事务还是进程内锁？

**结论：当前使用的是进程内 per-directory Mutex 锁，而非 RocksDB 事务。**

#### 当前并发控制机制

元数据服务器的并发控制通过 `dir_locks` 实现：

```rust
// server/src/lib.rs
pub struct MetadataServer<M, I, DS> {
    metadata_store: M,
    dir_index: I,
    delta_store: DS,
    // Per-directory mutex for directory-modifying operations
    dir_locks: Mutex<HashMap<Inode, Arc<Mutex<()>>>>,
    // ...
}
```

所有目录修改操作（`create`、`mkdir`、`unlink`、`rmdir`、`rename`）在执行前都会调用 `lock_dir(parent)` 获取对应目录的互斥锁。

#### WriteBatch 的有限使用

RocksDB `WriteBatch` **仅**在 `RocksDeltaStore` 中使用：

```rust
// RocksDeltaStore::append_deltas() — batch write multiple delta entries in same CF
// RocksDeltaStore::clear_deltas()  — batch delete delta entries in same CF
```

这两处都是在 **同一个 Column Family** 内的批量操作，不涉及跨 CF 原子写入。

#### 关键风险：非原子多步操作

以 `create` 操作为例，执行链路包含多个独立的 RocksDB 写入：

```
create(parent, name, mode) {
    1. lock_dir(parent)                           // acquire mutex
    2. check name doesn't exist in dir_entries    // read CF:dir_entries
    3. allocate new inode                          // AtomicU64
    4. save_inode(new_inode, attrs)                // WRITE to CF:inodes ←── 独立写入 ①
    5. insert_child(parent, name, new_inode)       // WRITE to CF:dir_entries ←── 独立写入 ②
    6. append_parent_deltas(parent, nlink++, mtime)// WRITE to CF:delta_entries ←── 独立写入 ③
    7. unlock_dir(parent)
}
```

**步骤 4、5、6 是三次独立的 RocksDB 写入操作**，原子性完全依赖进程级别的锁。

**崩溃风险场景**：如果进程在步骤 4（`save_inode`）完成后、步骤 5（`insert_child`）执行前崩溃：
- RocksDB 中存在一个已持久化的 inode（CF:inodes），但没有对应的目录项（CF:dir_entries）
- 结果：产生一个**孤儿 inode**，既不可达也不会被清理

#### 现状 vs 理想状态对比

| 维度 | 当前状态 | 理想状态 |
|------|---------|---------|
| **并发控制** | Per-directory `Mutex`（进程内锁） | Per-directory 锁 + WriteBatch（双重保障） |
| **原子写入** | 每个 CF 操作独立写入 | 单个 `WriteBatch` 包裹所有跨 CF 写入 |
| **跨 CF 事务** | ❌ 无保障 | ✅ `WriteBatch` 天然支持跨 CF 原子写 |
| **崩溃一致性** | ⚠️ 部分写入可能产生孤儿 inode | ✅ WriteBatch 要么全部写入，要么全部丢弃 |
| **性能影响** | 多次 `put()` 调用，多次 WAL sync | 单次 `write(batch)`，单次 WAL sync，性能更优 |

> **注意**：RocksDB 的 `WriteBatch` 本身就保证跨 CF 的原子性，无需使用更重的 `Transaction`（`TransactionDB`）。对于单机场景，`WriteBatch` 是最优解。

### 2.2 元数据与文件数据的关联方式

**结论：元数据与文件数据完全分离，仅通过 inode 编号关联。**

#### 数据路径分析

| 操作 | 路径 | 说明 |
|------|------|------|
| **Write** | `VfsCore.write()` → `DataServer.write_data(inode, offset, data)` → `MetadataServer.report_write(inode, new_size, mtime)` | 先写数据，再更新元数据 |
| **Read** | `VfsCore.read()` → `DataServer.read_data(inode, offset, size)` | 完全绕过元数据 |
| **Delete** | `MetadataServer.unlink()` → 当 `nlink` 降至 0 → `DataServer.delete_data(inode)` | 元数据驱动删除 |
| **Truncate** | `MetadataServer.setattr(size=X)` → `DataServer.truncate(inode, X)` | 元数据驱动截断 |

```
┌─────────┐    write_data(ino, offset, data)    ┌────────────┐
│         │ ──────────────────────────────────► │            │
│ VfsCore │                                     │ DataServer │
│         │ ◄────────────────────────────────── │            │
│         │           ok / bytes_written         └────────────┘
│         │
│         │    report_write(ino, new_size, mtime) ┌─────────────────┐
│         │ ──────────────────────────────────►   │                 │
│         │                                       │ MetadataServer  │
│         │ ◄──────────────────────────────────   │                 │
└─────────┘           ok                          └─────────────────┘
```

#### 存在的问题

**问题 1：Write 操作非原子**

`write_data` 和 `report_write` 不在同一个事务中。如果 `write_data` 成功但 `report_write` 失败或进程崩溃，数据已写入但元数据（`size`、`mtime`）未更新，导致文件元数据与实际数据不一致。

**问题 2：RawDiskDataStore 固定分区方案**

```rust
// dataserver/src/lib.rs
const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024; // 64 MB per inode

fn offset_for(inode: Inode) -> u64 {
    inode * MAX_FILE_SIZE // fixed partition
}
```

每个 inode 固定分配 64MB 空间。对于大量小文件（如配置文件、日志片段），空间浪费严重。

**问题 3：无数据校验机制**

元数据中记录的 `size` 与 `DataStore` 中实际存储的数据大小之间没有校验逻辑。无法检测或修复不一致。

**问题 4：DataLocation 分布式遗留**

```rust
// core/src/lib.rs
pub struct DataLocation {
    pub server_id: String,
    pub address: String,
}
```

`DataLocation` 用于分布式模式下路由到不同的 DataServer。在单机模式下完全无用。

---

## 3. 多余组件分析

### 3.1 可以删除的组件

| 组件 | 位置 | 删除原因 | 建议 |
|------|------|---------|------|
| `rpc/` | `rpc/` | 纯分布式组件，gRPC 通信层 | ⚠️ 建议暂时保留，不影响编译，便于未来扩展 |
| `DataLocation` | `core/src/lib.rs` | 分布式 DataServer 路由信息 | 可安全移除 |
| `OpenResponse.data_location` | `core/src/lib.rs` | 同上 | 随 `DataLocation` 一起移除 |
| `VfsCore.handle_cache` | `client/src/vfs_core.rs` | 缓存 DataServer 地址，单机模式无用 | 可安全移除 |
| `server/src/main.rs` | `server/src/main.rs` | 独立服务器二进制入口（分布式遗留） | 可安全移除 |

### 3.2 需要改进的组件

| 组件 | 当前问题 | 改进方向 | 优先级 |
|------|---------|---------|--------|
| **RawDiskDataStore** | 固定分区，每个 inode 64MB 上限，小文件浪费 | 改为 per-inode 文件存储，或混合方案（<4KB inline 到 RocksDB，大文件独立存储） | 🔴 高 |
| **MetadataServer 写操作** | 非原子多步写入，崩溃可能产生孤儿 inode | 引入 `WriteBatch` 事务包裹 | 🔴 高 |
| **FUSE 挂载选项** | 当前使用 `MountOption::RO`（**只读！**） | 改为读写挂载 | 🔴 高（阻塞所有写入测试） |
| **InodeAllocator.persist** | 持久化操作未绑定到 create/mkdir 的 WriteBatch | 纳入统一事务管理 | 🟡 中 |

> ⚠️ **重要发现**：FUSE 当前以只读模式挂载（`MountOption::RO`），这意味着所有 write、create、mkdir 等写操作在内核层面就会被拒绝。这是阻塞单机文件系统可用性的首要问题。

### 3.3 保留但低优先级的组件

| 组件 | 理由 |
|------|------|
| `rpc/` | 代码不影响单机编译路径，保留便于未来分布式扩展 |
| `demo/` | auto demo + REPL + FUSE 三种模式设计优秀，是良好的测试和演示工具 |
| `server/tests/` | 集成测试覆盖全面，是保证重构安全性的重要资产 |

---

## 4. 当前项目的优势与可复用部分

尽管存在上述问题，RucksFS 在架构设计上有多处亮点，在单机重构过程中可以直接复用。

### 4.1 KV 编码设计（encoding.rs）

编码方案简洁高效，充分利用了 RocksDB 的有序 key 特性：

```rust
// Key encoding examples:
// Inode key:     I|<inode:8B big-endian>
// Dir entry key: D|<parent_ino:8B big-endian>|<name:variable>
// Delta key:     X|<inode:8B big-endian>|<seq:8B big-endian>

// InodeValue: 57 bytes fixed-length binary encoding
// ┌──────┬──────┬───────┬───────┬──────┬──────┬──────┬───────┬──────┐
// │ ino  │ size │ nlink │ mode  │ uid  │ gid  │atime │ mtime │ctime │
// │ 8B   │ 8B   │ 4B    │ 4B    │ 4B   │ 4B   │ 8+4B │ 8+4B │ 1B   │
// └──────┴──────┴───────┴───────┴──────┴──────┴──────┴───────┴──────┘
```

**优势**：
- Big-endian 编码保证 inode 的字典序与数值序一致，支持 RocksDB range scan
- 目录项以 `parent_ino` 为前缀，`list_children(parent)` 只需一次 prefix scan
- 固定长度的 `InodeValue` 避免了序列化框架的开销

### 4.2 Delta 增量更新 + 后台压缩

写优化设计，将频繁的小更新（如 mtime、atime）转化为追加操作：

```rust
// server/src/delta.rs
pub enum DeltaOp {
    IncrementNlink,       // nlink += 1
    DecrementNlink,       // nlink -= 1
    SetMtime(SystemTime), // update modification time
    SetCtime(SystemTime), // update change time
    SetAtime(SystemTime), // update access time
    SetSize(u64),         // update file size
}
```

```
Write path (fast):
  create() → append delta [IncrementNlink, SetMtime] to CF:delta_entries
           (avoids read-modify-write on CF:inodes)

Read path (bounded amplification):
  getattr() → read base inode from CF:inodes (or cache)
            → scan deltas from CF:delta_entries
            → fold deltas onto base inode
            → return merged result

Background compaction:
  DeltaCompactionWorker periodically:
    1. read base inode
    2. fold all pending deltas
    3. write merged inode back to CF:inodes (single put)
    4. clear applied deltas from CF:delta_entries
```

**优势**：写入路径避免了 read-modify-write 的开销；读取放大由后台 compaction 控制在有界范围内。

### 4.3 LRU Cache + Cache-Aware Delta Apply

```rust
// server/src/cache.rs
pub struct InodeFoldedCache {
    cache: LruCache<Inode, FileAttr>,
    // ...
}
```

缓存感知的设计确保在写入操作后不会产生 cache miss：

1. `getattr()` 先查 cache → hit 则直接返回（仅需 fold 新 delta）
2. `create()/write()` 写入后主动更新 cache
3. 后台 compaction 完成后更新 cache

### 4.4 清晰的 Trait 分层

```rust
// Trait hierarchy (storage/src/lib.rs + core/src/lib.rs):

// Storage layer traits:
trait MetadataStore   { fn get_inode(), fn save_inode(), ... }
trait DirectoryIndex  { fn insert_child(), fn remove_child(), fn list_children(), ... }
trait DeltaStore      { fn append_deltas(), fn get_deltas(), fn clear_deltas(), ... }
trait DataStore       { fn write_data(), fn read_data(), fn delete_data(), ... }

// Service layer traits:
trait MetadataOps     { fn create(), fn mkdir(), fn unlink(), fn getattr(), ... }
trait DataOps         { fn write(), fn read(), fn truncate(), ... }
trait VfsOps          { fn lookup(), fn getattr(), fn read(), fn write(), ... }
```

每层 trait 职责明确，backend 可插拔。`MemoryMetadataStore` 用于测试，`RocksMetadataStore` 用于生产，切换无需修改上层代码。

### 4.5 全面的测试覆盖

- `server/tests/integration.rs`: 元数据操作集成测试，包含并发测试
- `demo/tests/integration_test.rs`: 端到端集成测试
- `demo/tests/stress_test.rs`: 压力测试，包含并发和边界条件
- 多数关键路径都有对应的测试用例

---

## 5. 单机路线图：FUSE 为主 + eBPF 增强

### Phase 1（第 1-2 周）：单机 FUSE 文件系统精炼

**目标**：使 RucksFS 成为一个可正常读写的单机 FUSE 文件系统。

#### 5.1.1 修复 FUSE 只读问题

当前 FUSE 以只读模式挂载，这是最紧急的修复项：

```rust
// BEFORE (client/src/fuse.rs or demo/src/main.rs):
let options = vec![MountOption::RO, MountOption::FSName("rucksfs".to_string())];

// AFTER:
let options = vec![
    MountOption::FSName("rucksfs".to_string()),
    MountOption::AutoUnmount,
    MountOption::AllowOther,  // optional: allow non-root access
    // Remove MountOption::RO to enable read-write
];
```

#### 5.1.2 改进 DataStore 后端

替换固定分区的 `RawDiskDataStore`，采用混合存储方案：

```rust
// Hybrid storage strategy:
enum DataStorage {
    /// Files < 4KB: inline in RocksDB (CF:inlines)
    Inline(Vec<u8>),
    /// Files >= 4KB: per-inode file on disk
    FileBackend { path: PathBuf },
}

// Per-inode file storage layout:
// data_root/
//   00/00/00/01.dat  → inode 1
//   00/00/00/02.dat  → inode 2
//   ...
// (2-level directory hashing to avoid too many files in one dir)
```

#### 5.1.3 补充 readdir 中的 `.` 和 `..`

POSIX 要求 `readdir` 返回 `.`（当前目录）和 `..`（父目录）两个特殊目录项。

#### 5.1.4 精简 core 接口

移除 `DataLocation`、`OpenResponse.data_location`、`VfsCore.handle_cache` 等分布式遗留。

#### 5.1.5 POSIX 操作覆盖度

| 操作 | 状态 | 说明 |
|------|------|------|
| `lookup` | ✅ 已实现 | |
| `getattr` | ✅ 已实现 | |
| `setattr` | ✅ 已实现 | |
| `create` | ✅ 已实现 | |
| `mkdir` | ✅ 已实现 | |
| `unlink` | ✅ 已实现 | |
| `rmdir` | ✅ 已实现 | |
| `rename` | ✅ 已实现 | |
| `read` | ✅ 已实现 | |
| `write` | ✅ 已实现 | |
| `readdir` | ✅ 已实现 | 需补充 `.` 和 `..` |
| `open` / `release` | ✅ 已实现 | |
| `link` (hard link) | ❌ 未实现 | Phase 2 实现 |
| `symlink` / `readlink` | ❌ 未实现 | 低优先级 |
| `statfs` | ❌ 未实现 | 低优先级 |
| `chmod` / `chown` | ⚠️ 部分（via setattr） | |
| `utimens` | ⚠️ 部分（via setattr） | |

#### 5.1.6 验收标准

- 通过基础端到端测试：挂载 → 创建文件 → 写入 → 读取 → 删除 → 卸载
- `ls`、`cat`、`echo ... > file`、`rm`、`mkdir`、`cp` 等基础 shell 命令正常工作

---

### Phase 2（第 3-4 周）：事务化与原子性增强

**目标**：消除崩溃一致性风险，引入 WriteBatch 事务包裹。

#### 5.2.1 设计 WriteBatch/Transaction 抽象

```rust
/// Transaction abstraction for atomic multi-CF writes
pub trait AtomicWriteContext {
    fn put_inode(&mut self, inode: Inode, value: &InodeValue);
    fn put_dir_entry(&mut self, parent: Inode, name: &str, child: Inode);
    fn delete_dir_entry(&mut self, parent: Inode, name: &str);
    fn put_delta(&mut self, inode: Inode, seq: u64, op: &DeltaOp);
    fn put_system(&mut self, key: &str, value: &[u8]);
    fn commit(self) -> Result<(), FsError>;
}

/// RocksDB implementation
pub struct RocksWriteContext<'a> {
    batch: WriteBatch,
    db: &'a DB,
    cf_inodes: &'a ColumnFamily,
    cf_dir_entries: &'a ColumnFamily,
    cf_delta_entries: &'a ColumnFamily,
    cf_system: &'a ColumnFamily,
}
```

#### 5.2.2 重构 create 操作示例

```rust
// BEFORE: 3 independent writes, non-atomic
fn create(&self, parent: Inode, name: &str, mode: u32) -> Result<FileAttr> {
    let _lock = self.lock_dir(parent);
    self.check_not_exists(parent, name)?;
    let ino = self.allocator.next();
    self.metadata_store.save_inode(ino, &attrs)?;       // Write ①
    self.dir_index.insert_child(parent, name, ino)?;     // Write ②
    self.delta_store.append_deltas(parent, &deltas)?;    // Write ③
    Ok(attrs)
}

// AFTER: single WriteBatch, all-or-nothing
fn create(&self, parent: Inode, name: &str, mode: u32) -> Result<FileAttr> {
    let _lock = self.lock_dir(parent);
    self.check_not_exists(parent, name)?;
    let ino = self.allocator.next();

    let mut txn = self.begin_write_context();
    txn.put_inode(ino, &inode_value);                    // ─┐
    txn.put_dir_entry(parent, name, ino);                // ─┤ Single atomic
    txn.put_delta(parent, seq, &DeltaOp::IncrementNlink);// ─┤ WriteBatch
    txn.put_system("next_inode", &(ino + 1).to_be_bytes()); // ─┘
    txn.commit()?;                                        // One WAL sync

    self.cache.insert(ino, attrs);
    Ok(attrs)
}
```

#### 5.2.3 集成 InodeAllocator 持久化

将 `next_inode` 的持久化写入纳入 WriteBatch，确保 inode 分配与使用的原子性。

#### 5.2.4 添加 Hard Link 支持

```rust
fn link(&self, inode: Inode, new_parent: Inode, new_name: &str) -> Result<FileAttr> {
    let _lock = self.lock_dir(new_parent);
    let mut txn = self.begin_write_context();
    txn.put_dir_entry(new_parent, new_name, inode);
    txn.put_delta(inode, seq, &DeltaOp::IncrementNlink);
    txn.put_delta(new_parent, seq, &DeltaOp::SetMtime(now));
    txn.commit()?;
    Ok(attrs)
}
```

---

### Phase 3（第 5-7 周）：eBPF 可观测与自适应优化

**目标**：利用 eBPF 实现零侵入的文件系统可观测性和自适应预取优化。

#### 5.3.1 技术栈

| 组件 | 技术选型 | 说明 |
|------|---------|------|
| eBPF 框架 | **Aya** (Rust native) | 纯 Rust eBPF 开发，与项目技术栈一致 |
| Hook 类型 | tracepoint + kprobe | 内核层面的无侵入探测 |
| 数据交换 | BPF maps | eBPF 内核态 ↔ 用户态数据通道 |

#### 5.3.2 eBPF 程序设计

三个 Hook 点对应三种观测维度：

```
┌──────────────────────────────────────────────────────────────┐
│                     Kernel Space (eBPF)                      │
│                                                              │
│  ┌─────────────────────────────────────┐                     │
│  │ Hook 1: tracepoint/fuse/fuse_request_send                │
│  │   → Increment access_count_map[opcode]                   │
│  │   → Record request start timestamp                       │
│  └─────────────────────────────────────┘                     │
│                                                              │
│  ┌─────────────────────────────────────┐                     │
│  │ Hook 2: tracepoint/fuse/fuse_request_end                 │
│  │   → Calculate latency = end - start                      │
│  │   → Update latency_histogram_map[opcode][bucket]         │
│  └─────────────────────────────────────┘                     │
│                                                              │
│  ┌─────────────────────────────────────┐                     │
│  │ Hook 3: kprobe on readdir path                           │
│  │   → Detect sequential readdir patterns                   │
│  │   → Push prefetch hints to prefetch_hint_rb (ring buf)   │
│  └─────────────────────────────────────┘                     │
│                                                              │
│  BPF Maps:                                                   │
│  ┌───────────────────┐  ┌────────────────────────┐          │
│  │ access_count_map  │  │ latency_histogram_map  │          │
│  │ HashMap<u32, u64> │  │ HashMap<(u32,u32),u64> │          │
│  └───────────────────┘  └────────────────────────┘          │
│  ┌───────────────────┐                                       │
│  │ prefetch_hint_rb  │                                       │
│  │ RingBuf           │                                       │
│  └───────────────────┘                                       │
└──────────────────────────────────────────────────────────────┘
                           │
                    perf event / polling
                           │
                           ▼
┌──────────────────────────────────────────────────────────────┐
│                    User Space (Rust + Aya)                    │
│                                                              │
│  ┌─────────────────┐  ┌──────────────────┐                  │
│  │ PrefetchEngine  │  │ LatencyMonitor   │                  │
│  │ - batch multiget│  │ - p50/p99/p999   │                  │
│  │ - warm cache    │  │ - alert on spike │                  │
│  └─────────────────┘  └──────────────────┘                  │
│  ┌─────────────────┐                                         │
│  │ PatternDetector │                                         │
│  │ - seq readdir   │                                         │
│  │ - hot dir detect│                                         │
│  └─────────────────┘                                         │
└──────────────────────────────────────────────────────────────┘
```

#### 5.3.3 BFO 启发的批量预取优化

借鉴 FetchBPF 的思路，在 readdir 场景下实现批量元数据预取：

```
Traditional readdir:
  readdir(dir) → [entry1, entry2, ..., entryN]
  getattr(entry1) → miss → RocksDB get
  getattr(entry2) → miss → RocksDB get
  ...
  getattr(entryN) → miss → RocksDB get
  (N sequential RocksDB reads)

BFO-optimized readdir:
  readdir(dir) → [entry1, entry2, ..., entryN]
  eBPF detects readdir pattern → prefetch_hint_rb
  PrefetchEngine receives hint →
    batch_keys = [I|<ino1>, I|<ino2>, ..., I|<inoN>]
    RocksDB multiget(batch_keys)  ← single batch read
    warm InodeFoldedCache with results
  getattr(entry1) → cache hit ✓
  getattr(entry2) → cache hit ✓
  ...
  (all cache hits, near-zero latency)
```

#### 5.3.4 研究创新点

| 创新点 | 说明 |
|--------|------|
| **运行时自适应** | 无需静态配置，eBPF 程序根据实际访问模式动态调整预取策略 |
| **零侵入性** | eBPF 程序可随时加载/卸载，不修改文件系统核心代码，不影响基线性能 |
| **可量化改进** | eBPF latency histogram 提供精确的 A/B 测试数据，论文数据有说服力 |
| **元数据预取扩展** | 将 FetchBPF 的数据预取思想扩展到 KV 元数据场景，属于新的探索方向 |

---

### Phase 4（第 8-9 周）：性能测试与论文撰写

#### 5.4.1 测试工具矩阵

| 工具 | 测试维度 | 典型测试场景 |
|------|---------|-------------|
| **fio** | 顺序/随机 I/O 吞吐量和延迟 | 4K 随机读写、128K 顺序读写 |
| **mdtest** (IO500) | 元数据操作性能 | 大量文件创建/stat/删除 |
| **filebench** | 复合工作负载 | fileserver, webserver, mailserver profiles |
| **自定义 benchmark** | 特定场景 | readdir + stat 批量操作、delta compaction 效果 |

#### 5.4.2 对比基准

| 基准 | 意义 |
|------|------|
| **ext4** | POSIX 文件系统标准参考 |
| **tmpfs** | 纯内存性能上界 |
| **RucksFS (without eBPF)** | 自身基线 |
| **RucksFS (with eBPF)** | eBPF 增强效果 |
| **RucksFS (without delta)** | Delta 机制效果 |

#### 5.4.3 关键指标

- **Metadata IOPS**: create/stat/unlink operations per second
- **Throughput**: sequential and random read/write (MB/s)
- **Latency distribution**: p50, p99, p999
- **readdir + stat batch latency**: with/without eBPF prefetch
- **Space efficiency**: metadata overhead per file

---

## 6. 优先级排序与时间估算

### 10 周时间线

```
Week 1-2  ░░░░░░░░░░░░░░░░░░░░  Phase 1: 单机 FUSE 精炼
Week 3-4  ░░░░░░░░░░░░░░░░░░░░  Phase 2: 事务化 + 原子性
Week 5-7  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  Phase 3: eBPF 可观测
Week 8-9  ░░░░░░░░░░░░░░░░░░░░  Phase 4: 性能测试
Week 10   ░░░░░░░░░░░░░░░░░░░░  论文撰写 + 答辩准备
```

### 交付物分级

#### 🔴 最低交付物（Must Have）

| 编号 | 交付物 | 对应阶段 | 完成标准 |
|------|--------|---------|---------|
| D1 | 单机可读写 FUSE 文件系统 | Phase 1 | shell 命令（ls, cat, echo, rm, mkdir, cp）正常工作 |
| D2 | WriteBatch 事务化写入 | Phase 2 | 至少覆盖 `create`、`unlink`、`rename` 三个核心操作 |
| D3 | 性能测试报告 | Phase 4 | 至少完成 mdtest vs ext4 的对比数据 |

#### 🟡 加分交付物（Nice to Have）

| 编号 | 交付物 | 对应阶段 | 完成标准 |
|------|--------|---------|---------|
| D4 | eBPF 可观测性 + 预取优化 | Phase 3 | readdir + stat 场景可量化的延迟降低 |
| D5 | Hard link 支持 | Phase 2 | `ln` 命令正常工作 |
| D6 | Hybrid DataStore | Phase 1 | 小文件 inline + 大文件独立存储 |

### 风险与缓解

| 风险 | 影响 | 缓解策略 |
|------|------|---------|
| eBPF tracepoint 不可用（内核版本限制） | Phase 3 无法完成 | 提前在目标内核验证；备选方案：用 `perf_event` 替代 |
| RocksDB WriteBatch 跨 CF 性能不达预期 | Phase 2 效果有限 | 基准测试证明 WriteBatch 通常比多次 put 更快 |
| FUSE overhead 过大导致性能数据不好看 | Phase 4 对比不利 | 强调元数据性能（FUSE overhead 主要影响数据 I/O），使用 mdtest |

---

## 附录：代码行数统计

### 各 Crate 代码行数

| Crate | 源码（行） | 测试（行） | 合计（行） | 说明 |
|-------|-----------|-----------|-----------|------|
| `core/` | ~160 | — | ~160 | 公共类型与 trait 定义 |
| `storage/` | ~700 | ~350 | ~1,050 | 编码、内存后端、分配器 |
| `server/` | ~600 | ~650 | ~1,250 | 元数据服务器 + 集成测试 |
| `dataserver/` | ~130 | — | ~130 | 数据服务器薄包装 |
| `client/` | ~500 | — | ~500 | FUSE + VFS + Embedded |
| `rpc/` | ~500 | — | ~500 | gRPC 通信层 |
| `demo/` | ~600 | ~1,300 | ~1,900 | demo + 集成测试 + 压力测试 |
| **合计** | **~3,190** | **~2,300** | **~5,490** | |

> 注：以上为估算值，实际行数可能因空行、注释、宏展开等因素略有差异。核心代码约 3,500 行，测试代码约 2,300 行，总计约 5,994 行（含注释和空行）。

### 代码质量评估

| 指标 | 评估 |
|------|------|
| **测试覆盖率** | ~40% 代码为测试代码，覆盖率较高 |
| **模块化程度** | Trait-based 分层设计，backend 可插拔 |
| **文档注释** | 核心 trait 有文档注释，内部实现注释较少 |
| **错误处理** | 统一使用 `FsError` 枚举，`Result` 传播 |
| **依赖管理** | 合理使用 workspace dependencies，RocksDB 为 optional feature |
