
# RucksFS 架构深度解析

本文档详细解答以下四个核心技术问题：

1. 元数据和文件数据之间如何关联？
2. 元数据操作的锁 / 事务是如何处理的？
3. Delta Record 机制是如何工作的？
4. 当前项目有哪些妥协？

---

## 目录

- [1. 元数据与文件数据的关联](#1-元数据与文件数据的关联)
  - [1.1 InodeValue 的存储内容](#11-inodevalue-的存储内容)
  - [1.2 数据定位：隐式映射而非显式记录](#12-数据定位隐式映射而非显式记录)
  - [1.3 元数据 → 数据的关联流程](#13-元数据--数据的关联流程)
  - [1.4 KV Key Schema 总览](#14-kv-key-schema-总览)
- [2. 并发控制：PCC 事务模型](#2-并发控制pcc-事务模型)
  - [2.1 从 dir_locks 到 PCC 的演进](#21-从-dir_locks-到-pcc-的演进)
  - [2.2 事务化写路径详解](#22-事务化写路径详解)
  - [2.3 死锁预防策略](#23-死锁预防策略)
  - [2.4 并发冲突分析](#24-并发冲突分析)
  - [2.5 树形结构一致性保证](#25-树形结构一致性保证)
- [3. Delta Record 机制](#3-delta-record-机制)
  - [3.1 解决的核心问题](#31-解决的核心问题)
  - [3.2 工作原理](#32-工作原理)
  - [3.3 后台压缩（Compaction）](#33-后台压缩compaction)
  - [3.4 LRU 缓存集成](#34-lru-缓存集成)
  - [3.5 Delta 与 PCC 的协同](#35-delta-与-pcc-的协同)
  - [3.6 性能分析](#36-性能分析)
- [4. 项目现状与妥协](#4-项目现状与妥协)
  - [4.1 存储选择](#41-存储选择)
  - [4.2 数据存储的简化](#42-数据存储的简化)
  - [4.3 内存后端的局限](#43-内存后端的局限)
  - [4.4 其他妥协](#44-其他妥协)

---

## 1. 元数据与文件数据的关联

### 1.1 InodeValue 的存储内容

每个文件/目录的元数据以 `InodeValue` 的形式存储在 RocksDB 的 `inodes` Column Family 中。其二进制布局为：

```
[version: u8][inode: u64 BE][size: u64 BE][mode: u32 BE][nlink: u32 BE]
[uid: u32 BE][gid: u32 BE][atime: u64 BE][mtime: u64 BE][ctime: u64 BE]
```

**总大小：57 字节**。

关键点：**`InodeValue` 不存储文件数据的物理位置（没有 block pointer、extent、offset 等字段）**。它只记录：

| 字段     | 含义                                     |
|----------|------------------------------------------|
| `inode`  | inode 编号（唯一标识符）                  |
| `size`   | 文件逻辑大小（字节数）                    |
| `mode`   | 文件类型 + 权限位（如 `0o100644`）        |
| `nlink`  | 硬链接数                                 |
| `uid/gid`| 所有者 / 组                              |
| `atime/mtime/ctime` | 访问 / 修改 / 状态变更时间     |

### 1.2 数据定位：隐式映射而非显式记录

RucksFS 的数据存储（`RawDiskDataStore`）使用一种**隐式映射**方案——文件数据的物理位置完全由 inode 编号和 offset 计算得出，**不需要在元数据中记录任何数据位置信息**。

计算公式：

```
absolute_offset = inode × max_file_size + file_offset
```

```
┌─────────────────────────────────────────────────────────────┐
│                    data.raw (flat file)                      │
├────────────────┬────────────────┬────────────────┬──────────┤
│  inode 0       │  inode 1       │  inode 2       │   ...    │
│  [0, 64MiB)    │  [64MiB,128MiB)│  [128MiB,192MiB)│         │
└────────────────┴────────────────┴────────────────┴──────────┘
                   ↑
                   inode=1, offset=100 → absolute = 1×64MiB + 100
```

这意味着：
- **不需要块分配表**（block bitmap / extent tree）
- **不需要间接块**（indirect block）
- 给定 `(inode, offset)` 即可直接定位到文件中的字节位置
- `max_file_size` 默认为 64 MiB（可配置）

### 1.3 元数据 → 数据的关联流程

以写入为例，完整的数据流：

```
Client.write(inode=42, offset=0, data="hello")
    │
    ├─── ① DataServer.write_at(inode=42, offset=0, data)
    │       └── absolute_offset = 42 × 64MiB + 0
    │           写入 data.raw 文件的对应位置
    │
    └─── ② MetadataServer.report_write(inode=42, new_size=5, mtime=now)
            └── 事务内：
                get_for_update_inode([I][42]) → 读取并锁定 InodeValue
                iv.size = max(iv.size, 5)     → 更新 size
                iv.mtime = now                → 更新修改时间
                batch.commit()                → 原子提交
```

以读取为例：

```
Client.read(inode=42, offset=0, size=5)
    │
    ├─── ① MetadataServer.getattr(42) → 确认 inode 存在，获取 size
    │
    └─── ② DataServer.read_at(42, 0, 5) → 直接计算偏移读取
```

**总结**：元数据（RocksDB）和文件数据（RawDisk）通过 **inode 编号**这个共同的逻辑标识符关联。元数据中的 `size` 字段记录了文件的逻辑大小，但数据的物理位置由 inode 编号隐式决定。

### 1.4 KV Key Schema 总览

RocksDB 中使用 4 个 Column Family：

| Column Family   | Key 格式                                     | Value 格式             | 用途                 |
|-----------------|----------------------------------------------|------------------------|----------------------|
| `inodes`        | `[b'I'][inode: u64 BE]`                      | `InodeValue` (57 bytes)| inode 元数据          |
| `dir_entries`   | `[b'D'][parent_inode: u64 BE][name: UTF-8]`  | `[child_inode: u64 BE][child_mode: u32 BE]` | 目录条目 |
| `delta_entries` | `[b'X'][inode: u64 BE][seq: u64 BE]`         | `DeltaOp` (5-9 bytes)  | 增量更新             |
| `system`        | 如 `b"next_inode"`                            | `u64 BE`               | 全局状态             |

Key 设计的核心思想：
- **Big-endian 编码**：保证字节序 = 数值序，RocksDB 的 prefix scan 天然有序
- **dir_entries 包含文件名**：`[parent][name]` 使得同目录下不同文件的 key 不同，PCC 行锁粒度达到**单文件级**
- **delta key 包含 seq**：每个 delta 有独立 key，并发 append 不冲突

---

## 2. 并发控制：PCC 事务模型

### 2.1 从 dir_locks 到 PCC 的演进

项目经历了一次重要的并发控制模型升级：

| 维度     | 旧方案：dir_locks                           | 新方案：PCC 事务                              |
|----------|---------------------------------------------|-----------------------------------------------|
| 锁粒度   | **目录级**：`Mutex<HashMap<Inode, Arc<Mutex<()>>>>` | **Key 级**：RocksDB `get_for_update` 行锁      |
| 并发能力 | 同目录所有操作串行                           | 同目录不同文件并发                             |
| 安全保证 | 依赖开发者正确加锁                           | 引擎级隔离 + 自动死锁检测                      |
| 原子性   | `WriteBatch` 保证原子写入                     | `Transaction::commit()` 保证读写全链路原子性   |
| 代码复杂度| 锁获取分散在各写路径中                       | 统一的 `execute_with_retry` 包装               |

**升级的核心动机**：旧方案中，同一目录下的 `create("a")` 和 `create("b")` 被完全串行化（因为它们都要获取 parent 目录的锁），而这两个操作操作的是完全不同的 key，不应该冲突。

### 2.2 事务化写路径详解

当前所有 FUSE 写操作都在 RocksDB PCC 事务中执行。以 `create` 为例：

```rust
async fn create(&self, parent: Inode, name: &str, mode: u32) -> FsResult<FileAttr> {
    let (iv, new_inode) = self.execute_with_retry(|| {
        // ① 开始事务
        let mut batch = self.begin_write();  // → TransactionDB::transaction()

        // ② 检查文件名是否已存在 (get_for_update 获取行锁)
        let dir_key = encode_dir_entry_key(parent, name);
        if batch.get_for_update_dir_entry(&dir_key)?.is_some() {
            return Err(FsError::AlreadyExists);
        }

        // ③ 分配新 inode (AtomicU64, 事务外)
        let new_inode = self.allocator.alloc();

        // ④ 写入新 inode + 目录条目
        Self::batch_put_inode(batch.as_mut(), new_inode, &iv);
        Self::batch_put_dir_entry(batch.as_mut(), parent, name, new_inode, iv.mode);

        // ⑤ 原子提交
        batch.commit()?;
        Ok((iv, new_inode))
    })?;

    // ⑥ 事务外：持久化 inode 分配器 (热点规避)
    self.allocator.persist(self.metadata.as_ref())?;

    // ⑦ 事务外：delta append 更新父目录 mtime/ctime
    self.append_parent_deltas(parent, &[DeltaOp::SetMtime(ts), DeltaOp::SetCtime(ts)]);

    Ok(iv.to_attr())
}
```

**关键设计点**：

1. **`get_for_update_dir_entry`**：在事务内读取目录条目的同时获取行锁，保证从读取到写入期间没有其他事务修改同一个 key
2. **`allocator.alloc()` 在事务外**：inode 分配使用 `AtomicU64::fetch_add`（无锁原子操作），不经过 KV 事务，避免 `next_inode` 成为全局热点
3. **`allocator.persist()` 在事务后**：`next_inode` 的持久化通过普通写入（非事务写入），仅用于崩溃恢复
4. **`append_parent_deltas` 在事务外**：父目录的 mtime/ctime 更新走 Delta 机制，不在主事务内

### 2.3 死锁预防策略

**两层保险**：

**第一层：固定加锁顺序（物理消除死锁环）**

所有涉及多个 inode 的操作（如 `rename`），按 **inode ID 从小到大** 的顺序调用 `get_for_update_inode`：

```rust
// rename 中的加锁顺序
let mut inode_ids = vec![src_inode];
if let Some(dst_ino) = dst_inode_opt { inode_ids.push(dst_ino); }
if !inode_ids.contains(&parent) { inode_ids.push(parent); }
if parent != new_parent { inode_ids.push(new_parent); }
inode_ids.sort_unstable();  // ← 固定顺序
inode_ids.dedup();

// 按排序后的顺序逐个加锁
for &ino in &inode_ids {
    batch.get_for_update_inode(&encode_inode_key(ino))?;
}
```

任何两个并发事务的加锁顺序都一致，**物理上不可能形成死锁环**。

**第二层：`execute_with_retry` 兜底重试**

```rust
fn execute_with_retry<F, T>(&self, mut f: F) -> FsResult<T> {
    for attempt in 0..TXN_MAX_RETRIES {  // TXN_MAX_RETRIES = 3
        match f() {
            Ok(v) => return Ok(v),
            Err(FsError::TransactionConflict) if attempt + 1 < TXN_MAX_RETRIES => {
                continue;  // 自动重试
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}
```

RocksDB 的 `TransactionDB` 内置死锁检测（`DeadlockDetect`），检测到死锁后返回 `Status::Busy`，被映射为 `FsError::TransactionConflict`。`execute_with_retry` 会自动重试，最多 3 次。

**实际上第二层几乎不会触发**——有了固定加锁顺序，死锁概率极低。

### 2.4 并发冲突分析

以下表格展示了各种并发场景在 PCC 模型下的冲突情况：

| 并发场景                                       | 涉及 Key                                          | 冲突？                    |
|------------------------------------------------|---------------------------------------------------|---------------------------|
| 同目录 `create("a")` + `create("b")`           | `[D][p]["a"]` vs `[D][p]["b"]`                    | ❌ **不冲突** — 不同 key  |
| 同目录 `create("foo")` + `unlink("foo")`       | 都操作 `[D][p]["foo"]`                             | ✅ 冲突 — **期望行为**    |
| 跨目录 `rename(dir1,"a",dir2,"b")` + `create(dir2,"c")` | 操作不同 dir_entry key                  | ❌ **不冲突**            |
| 同 inode `unlink` + `setattr`                  | 都操作 `[I][42]`                                   | ✅ 冲突 — **期望行为**    |
| 任意写操作 + `getattr`（读操作）                | 读不加锁                                           | ❌ **不冲突**            |

**对比旧方案（dir_locks）**：

| 场景                          | dir_locks | PCC    |
|-------------------------------|-----------|--------|
| 同目录创建不同文件             | ❌ 串行   | ✅ 并发 |
| 跨目录 rename + 另一目录 create| ❌ 串行   | ✅ 并发 |
| 同文件 create + unlink        | ✅ 串行   | ✅ 串行 |

### 2.5 树形结构一致性保证

文件系统的树形结构一致性由以下三个层次保证：

**① 事务原子性**：每个写操作的所有 KV 变更（inode + dir_entry）在一个事务内提交，要么全部可见、要么全部不可见。不可能出现"dir_entry 指向一个不存在的 inode"或"inode 存在但没有 dir_entry 指向它"的中间状态。

**② 行级锁隔离**：通过 `get_for_update` 在事务内锁定相关 key，防止 TOCTOU（time-of-check-time-of-use）竞态。例如 `create` 中先检查文件名不存在，再创建——这两步在同一事务内，外部无法在中间插入同名文件。

**③ Delta 机制解耦父子操作**：父目录的 mtime/ctime/nlink 更新不在子文件的主事务中，避免父目录 inode 成为热点。Delta 的 append-only 特性保证并发 append 不冲突。

---

## 3. Delta Record 机制

### 3.1 解决的核心问题

**热点目录的 read-modify-write 竞争**。

传统文件系统中，每次 `create`/`mkdir`/`unlink`/`rmdir` 都需要更新**父目录** inode 的 `mtime`、`ctime`（以及 `mkdir`/`rmdir` 时的 `nlink`）。这是一个 read-modify-write 操作：

```
// 无 Delta 时的问题
Thread A: create("a") → 读 parent_inode → 改 mtime → 写 parent_inode
Thread B: create("b") → 读 parent_inode → 改 mtime → 写 parent_inode
                                                       ↑ 冲突！
```

即使用了 PCC 事务，两个事务都要 `get_for_update([I][parent])`，只能串行执行。**这会让同一目录下的所有创建/删除操作退化为串行**。

Delta 机制的核心思想：**把 N 次 read-modify-write 变成 N 次 append + 1 次合并**。

### 3.2 工作原理

#### 3.2.1 DeltaOp 类型

```rust
pub enum DeltaOp {
    IncrementNlink(i32),  // 增减硬链接计数 (mkdir/rmdir 时)
    SetMtime(u64),        // 设置修改时间 (折叠取 max)
    SetCtime(u64),        // 设置状态变更时间 (折叠取 max)
    SetAtime(u64),        // 设置访问时间 (折叠取 max)
}
```

每个 `DeltaOp` 序列化为 5-9 字节的紧凑二进制格式：`[op_type: u8][payload: BE]`。

#### 3.2.2 写入路径（Append）

当 `create` / `mkdir` 等操作完成主事务后，在**事务外**追加 delta：

```rust
fn append_parent_deltas(&self, parent: Inode, deltas: &[DeltaOp]) -> FsResult<()> {
    // ① 序列化 DeltaOp
    let serialized: Vec<Vec<u8>> = deltas.iter().map(|d| d.serialize()).collect();

    // ② 追加到 RocksDB delta_entries CF
    //    key = [b'X'][parent_inode: u64 BE][seq: u64 BE]
    //    每个 delta 有独立的 seq → 独立的 key → 并发不冲突
    self.delta_store.append_deltas(parent, &serialized)?;

    // ③ 更新内存缓存（在缓存中立即折叠）
    self.cache.apply_deltas(parent, deltas);

    // ④ 标记 inode 为脏（通知后台压缩）
    self.compaction.mark_dirty(parent);

    Ok(())
}
```

**为什么并发不冲突？** 因为每个 delta 写入的 key 是 `[X][parent_inode][seq]`，`seq` 由 `AtomicU64::fetch_add` 分配，**每次 append 写入不同的 key**：

```
Thread A: create("a") → append delta → key = [X][parent][seq=1]
Thread B: create("b") → append delta → key = [X][parent][seq=2]
                                       ↑ 不同 key，完全并发！
```

#### 3.2.3 读取路径（Fold）

当读取 inode（如 `getattr`）时，先检查缓存，缓存未命中则执行 fold：

```rust
fn load_inode(&self, inode: Inode) -> FsResult<InodeValue> {
    // ① 缓存命中？
    if let Some(cached) = self.cache.get(inode) {
        return Ok(cached);  // 缓存中已是折叠后的值
    }

    // ② 读取 base inode（RocksDB inodes CF）
    let mut iv = InodeValue::deserialize(&self.metadata.get(&key)?)?;

    // ③ 扫描所有 pending delta（RocksDB delta_entries CF，prefix scan）
    let raw_deltas = self.delta_store.scan_deltas(inode)?;

    // ④ 折叠：将 delta 逐个应用到 base 上
    if !raw_deltas.is_empty() {
        let ops: Vec<DeltaOp> = raw_deltas.iter()
            .filter_map(|bytes| DeltaOp::deserialize(bytes).ok())
            .collect();
        fold_deltas(&mut iv, &ops);
    }

    // ⑤ 写入缓存
    self.cache.put(inode, iv.clone());
    Ok(iv)
}
```

#### 3.2.4 折叠语义

```rust
pub fn fold_deltas(base: &mut InodeValue, deltas: &[DeltaOp]) {
    for delta in deltas {
        match delta {
            IncrementNlink(n) => base.nlink = (base.nlink as i64 + n as i64).max(0) as u32,
            SetMtime(t)       => base.mtime = base.mtime.max(*t),  // 取最大值
            SetCtime(t)       => base.ctime = base.ctime.max(*t),  // 取最大值
            SetAtime(t)       => base.atime = base.atime.max(*t),  // 取最大值
        }
    }
}
```

关键点：
- `nlink` 用**累加**语义（支持负数）
- 时间戳用 **`max`** 语义——无论 delta 的应用顺序如何，结果都一致（**交换律 + 结合律**），这使得并发 append 的 delta 不需要排序就能正确折叠

### 3.3 后台压缩（Compaction）

Delta 如果无限累积会导致读放大（每次 `load_inode` 都要 scan 所有 delta）。`DeltaCompactionWorker` 负责定期将 delta 合并回 base inode。

#### 3.3.1 触发条件

```rust
pub struct CompactionConfig {
    pub interval_ms: u64,        // 扫描间隔，默认 5000ms
    pub delta_threshold: usize,  // 触发阈值，默认 32 个 delta
}
```

- **被动触发**：后台线程每 5 秒扫描一次 dirty set
- **阈值过滤**：只有累积 ≥ 32 个 delta 的 inode 才会被压缩
- **主动触发**：`force_compact_inode()` 和 `flush_all()` 可强制压缩

#### 3.3.2 压缩流程（PCC 事务化）

```rust
pub fn force_compact_inode(&self, inode: Inode) -> FsResult<bool> {
    // ① 事务外：扫描 delta（delta key 是 append-only，不会与其他事务冲突）
    let raw_deltas = self.delta_store.scan_deltas(inode)?;

    // ② 开始事务 + 锁定 base inode
    let mut batch = self.storage_bundle.begin_write();
    let mut base = batch.get_for_update_inode(&key)?;  // PCC 行锁

    // ③ Fold 合并
    fold_deltas(&mut base, &ops);

    // ④ 事务内：写回合并后的 inode + 删除所有 delta key
    batch.push(BatchOp::PutInode { key, value: base.serialize() });
    for dk in delta_keys { batch.push(BatchOp::DeleteDelta { key: dk }); }

    // ⑤ 提交
    match batch.commit() {
        Ok(())                              => { /* 成功 */ }
        Err(FsError::TransactionConflict)   => return Ok(false),  // 安全跳过
        Err(e)                              => return Err(e),
    }

    // ⑥ 失效缓存
    self.cache.invalidate(inode);
    Ok(true)
}
```

**关键设计**：压缩操作在 PCC 事务内通过 `get_for_update_inode` 锁定 base inode。如果另一个事务同时删除了该 inode（如 `unlink`），事务提交时会检测到冲突，压缩安全跳过——**不会出现 "inode 复活" 的问题**。

### 3.4 LRU 缓存集成

`InodeFoldedCache` 是一个线程安全的 LRU 缓存，存储**已折叠**的 `InodeValue`：

```
读路径: getattr(42)
    │
    ├─ cache.get(42) → 命中？返回缓存值（已包含所有 delta 效果）
    │
    └─ cache miss → 读 base + scan deltas + fold → cache.put(42, folded_value)

写路径: create("a", parent=1)
    │
    ├─ 事务提交后：cache.put(new_inode, new_iv)    → 新文件加入缓存
    │
    └─ delta append 后：cache.apply_deltas(parent, [SetMtime, SetCtime])
       → 如果 parent 在缓存中，直接在缓存中折叠（不读 RocksDB）

压缩路径: compact(42)
    │
    └─ 压缩完成后：cache.invalidate(42)
       → 下次读取重新从 RocksDB 加载最新的 base（此时 delta 已清空）
```

### 3.5 Delta 与 PCC 的协同

Delta 和 PCC 是**互补**的两层并发控制机制：

| 层次           | 机制   | 保护的操作                           | 冲突粒度     |
|----------------|--------|--------------------------------------|-------------|
| **主事务**     | PCC    | inode + dir_entry 的 read-modify-write| Key 级行锁 |
| **父目录更新** | Delta  | 父目录 mtime/ctime/nlink 的更新      | 无冲突     |

```
                          ┌──────────────────────────────┐
                          │        PCC 事务范围           │
 create("a") ─────────── │  get_for_update [D][p]["a"]  │ ── ④ commit
                          │  put [I][42]                 │
                          │  put [D][p]["a"]             │
                          └──────────────────────────────┘
                                       │
                                       ▼
                          ┌──────────────────────────────┐
                          │       Delta 范围（事务外）     │
                          │  append [X][p][seq=N]        │ ← 与其他 create 不冲突
                          │  cache.apply_deltas(p, ...)  │
                          │  compaction.mark_dirty(p)    │
                          └──────────────────────────────┘
```

**如果没有 Delta（纯 PCC）**：`create("a")` 和 `create("b")` 都需要在事务内 `get_for_update([I][parent])` → 修改 mtime → 写回。两个事务锁同一个 key，**被迫串行**。

**有了 Delta（PCC + Delta）**：父目录更新走 delta append，写的是不同 key（不同 seq），**完全并发**。

### 3.6 性能分析

#### 带来的收益

| 维度         | 无 Delta                           | 有 Delta                              |
|-------------|------------------------------------|-----------------------------------------|
| 写入吞吐量   | 同目录 create 全串行（父目录热点） | 同目录 create 完全并发                   |
| 写入延迟     | 一次 read-modify-write             | 一次 append（更快）                      |
| 写放大       | 每次操作写 57 字节 InodeValue      | 每次操作写 5-9 字节 DeltaOp             |

#### 代价

| 维度         | 影响                                                           |
|-------------|----------------------------------------------------------------|
| 读放大       | 首次读取未缓存的 inode 需要 scan delta（通过缓存缓解）           |
| 空间放大     | delta 累积占用额外空间（通过后台压缩缓解）                       |
| 复杂度       | 引入 DeltaStore + Compaction Worker + Cache 三个新组件           |

#### 是否引入并发问题？

**不会**。原因：

1. Delta append 写不同 key → 不冲突
2. Delta fold 是纯计算（读 base + 读 deltas → 合并），不修改存储
3. Compaction 在 PCC 事务内执行，与 unlink 等操作的冲突由 RocksDB 自动检测
4. 时间戳 fold 使用 `max` 语义，满足交换律和结合律，**顺序无关**
5. nlink fold 使用累加语义，累加满足交换律和结合律，**顺序无关**

---

## 4. 项目现状与妥协

### 4.1 存储选择

**是的，当前全面使用 RocksDB 作为元数据存储。**

| 组件                 | 存储后端       | 说明                                              |
|---------------------|----------------|---------------------------------------------------|
| `RocksMetadataStore`| RocksDB inodes CF | inode 元数据的 base 值                         |
| `RocksDirectoryIndex`| RocksDB dir_entries CF | 目录条目                                |
| `RocksDeltaStore`   | RocksDB delta_entries CF | 增量更新记录                            |
| `InodeAllocator`    | RocksDB system CF | `next_inode` 计数器持久化                      |
| `RawDiskDataStore`  | 单独的 flat file | 文件数据（不在 RocksDB 中）                     |

项目早期曾有 in-memory 后端可选（`MemoryMetadataStore` / `MemoryDirectoryIndex`），现在仅用于**单元测试**。生产路径始终使用 RocksDB。

**选择 RocksDB 的理由**（参考 TableFS、LocoFS 等论文研究）：
- LSM-tree 的写放大在元数据场景下是可接受的
- `WriteBatch` / `Transaction` 原生支持跨 CF 原子操作
- 前缀扫描（prefix iterator）天然适合 `readdir` 操作
- 内建死锁检测（`TransactionDB`）

### 4.2 数据存储的简化

`RawDiskDataStore` 是一个**有意的简化**：

| 特性                 | 当前实现                     | 生产级实现                              |
|---------------------|------------------------------|----------------------------------------|
| 空间分配             | 固定分区（inode × max_size） | 块分配器 + 空闲列表                     |
| 最大文件大小         | 64 MiB（硬编码上限）         | 无固定上限                              |
| 空间利用率           | 低（每个 inode 预留 64 MiB） | 按需分配                                |
| 并发控制             | `Mutex<File>`（全局互斥）    | 细粒度锁或 io_uring                    |
| 数据持久性           | `sync_data()` 按需           | WAL + fsync                             |

**这是项目中最大的妥协**。固定分区方案意味着：
- 空间浪费严重（1 字节的文件也占用 64 MiB 的地址空间）
- 最大文件数受限于 flat file 的大小
- 不支持真正的稀疏文件

但这个方案的优势是**极其简单**——给定 `(inode, offset)` 就能 O(1) 定位到数据位置，没有任何间接层。

### 4.3 内存后端的局限

`MemoryWriteBatch` 的 `commit()` 不是真正原子的：

```rust
// 内存后端的 commit 实现（简化）
fn commit(self: Box<Self>) -> FsResult<()> {
    for op in self.ops {
        match op {
            PutInode { key, value } => self.metadata.put(&key, &value)?,
            DeleteInode { key }     => self.metadata.delete(&key)?,
            // ... 逐个应用，中间状态可见
        }
    }
    Ok(())
}
```

如果在 `commit` 过程中 panic，可能导致部分操作可见。这在测试中是可接受的，但意味着**内存后端不适合并发正确性测试**。

### 4.4 其他妥协

| 妥协                         | 现状                                            | 说明                                     |
|-----------------------------|--------------------------------------------------|------------------------------------------|
| **uid/gid 始终为 0**        | `create`/`mkdir` 硬编码 `uid=0, gid=0`          | 未实现真正的用户权限映射                   |
| **无 hardlink 实现**        | `nlink` 字段存在但 `link()` 未实现               | POSIX 硬链接语义较复杂                    |
| **无 symlink 实现**         | 未实现                                           | 需要新的 inode 类型                       |
| **无 xattr 支持**           | 未实现                                           | 需要新的 CF                              |
| **单线程数据 I/O**          | `RawDiskDataStore` 用 `Mutex<File>`              | 所有数据读写全局串行                      |
| **rpc crate 未使用**        | gRPC 层已实现但单机模式下未启用                   | 为未来分布式扩展预留                      |
| **FUSE 单线程事件循环**     | `polyfuse` 的默认事件循环                        | 未利用 FUSE 的多线程能力                  |

### 4.5 架构优势

尽管有以上妥协，当前架构在元数据引擎层面是**相当完备**的：

```
✅ RocksDB PCC 事务 — 引擎级行锁 + 死锁检测
✅ Delta 增量更新 — 消除父目录热点
✅ 后台压缩 — 控制读放大
✅ LRU 缓存 — 热点 inode 零 I/O 读取
✅ Trait 抽象 — MetadataStore / DataStore / DirectoryIndex / DeltaStore 可替换
✅ 141 个测试 — 覆盖存储层 + 服务层 + 集成测试
```

核心的元数据并发模型（PCC + Delta + Compaction）是经过论文调研和工程验证的方案，在单机场景下能提供正确且高效的并发访问。
