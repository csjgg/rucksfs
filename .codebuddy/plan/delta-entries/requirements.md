# Delta Entries 需求文档

## 引言

### 背景

RucksFS 当前的元数据更新采用经典的 **read-modify-write** 模式：每当需要修改 inode 属性（如父目录的 `mtime`、`ctime`、`nlink`）时，必须先从 RocksDB 读取当前值，在内存中修改，然后写回。这一模式在高并发场景（如在同一目录下并发创建大量文件）中存在严重的写冲突问题。

借鉴 Mantle (SOSP'25) 论文中的 **Delta Entries** 机制，本功能将引入一种 append-only 的增量更新方案：将对 inode 属性的修改记录为独立的增量条目（delta entries），追加写入到新增的 `delta_entries` CF 中，由后台线程周期性地将增量合并回基线 inode 值。同时，在 server 端维护一个内存缓存（folded state cache）来保证读取性能不受 delta 累积的影响。

### 核心目标

1. **消除父目录属性更新的 read-modify-write 瓶颈**：将 `create`/`mkdir`/`unlink`/`rmdir`/`rename` 等操作中对父目录的属性更新从同步 read-modify-write 改为 append-only delta 写入
2. **提升高并发目录操作的吞吐量**：在同一目录下并发创建/删除文件时，避免多个线程争抢同一 inode 的读写锁
3. **保证读取一致性**：通过 server 端 folded state 缓存，确保 `getattr`/`lookup` 返回的属性值始终反映所有已提交的 delta
4. **保证崩溃一致性**：利用 RocksDB WriteBatch 保证 delta 写入的原子性，后台 compaction 也使用 WriteBatch 保证 base+delta 合并的原子性

### 涉及的文件范围

| 模块 | 文件 | 变更类型 |
|------|------|---------|
| `storage` | `src/lib.rs` | 新增 `DeltaStore` trait |
| `storage` | `src/encoding.rs` | 新增 Delta key/value 编码函数 |
| `storage` | `src/rocks.rs` | 新增 `delta_entries` CF + `RocksDeltaStore` 实现 |
| `storage` | `src/memory.rs` | 新增 `MemoryDeltaStore` 实现（用于测试） |
| `server` | `src/lib.rs` | 修改 `MetadataServer`：写路径使用 delta append，读路径 fold delta，引入 folded state cache |
| `server` | `src/delta.rs`（新建）| Delta 类型定义（`DeltaOp` enum）、fold 逻辑、compaction worker |
| `server` | `src/cache.rs`（新建）| Folded inode state 缓存 |

---

## 需求

### 需求 1：Delta 类型定义与编码

**用户故事：** 作为一名 RucksFS 开发者，我希望有一组明确定义的 delta 操作类型和二进制编码格式，以便系统能够以 append-only 方式记录 inode 属性的增量修改。

#### 验收标准

1. WHEN 系统需要记录一次 inode 属性增量修改 THEN 系统 SHALL 使用 `DeltaOp` 枚举来表示该操作，至少包含以下变体：
   - `IncrementNlink` — nlink 增加指定值（i32，可为负数）
   - `SetMtime(u64)` — 设置 mtime 为指定时间戳
   - `SetCtime(u64)` — 设置 ctime 为指定时间戳
   - `SetAtime(u64)` — 设置 atime 为指定时间戳

2. WHEN 一个 `DeltaOp` 被序列化 THEN 系统 SHALL 生成一个固定格式的二进制值：`[op_type: u8][payload: variable]`，其中 `op_type` 唯一标识操作类型，`payload` 为操作参数的大端序编码。

3. WHEN 一个 `DeltaOp` 被序列化后再反序列化 THEN 结果 SHALL 与原始值完全一致（round-trip correctness）。

4. WHEN delta 的 key 被编码 THEN 系统 SHALL 使用格式 `[inode_id: u64 BE][sequence: u64 BE]`（共 16 字节），其中 `sequence` 为每个 inode 单调递增的逻辑序列号。

5. WHEN 多个 delta 按 key 字节序排列 THEN 同一 inode 的 delta SHALL 按序列号升序排列（保证 fold 时的因果序）。

---

### 需求 2：DeltaStore 存储层实现

**用户故事：** 作为一名 RucksFS 开发者，我希望有一个 `DeltaStore` trait 抽象和对应的 RocksDB/Memory 实现，以便上层逻辑能够以 trait 方式操作 delta entries，且实现可替换。

#### 验收标准

1. WHEN `DeltaStore` trait 被定义 THEN 它 SHALL 包含以下方法：
   - `append_delta(inode: Inode, ops: &[DeltaOp]) -> FsResult<()>` — 原子追加一个或多个 delta
   - `scan_deltas(inode: Inode) -> FsResult<Vec<DeltaOp>>` — 按序列号顺序返回指定 inode 的所有未合并 delta
   - `clear_deltas(inode: Inode) -> FsResult<()>` — 删除指定 inode 的所有 delta（用于 compaction 后清理）

2. WHEN `RocksDeltaStore` 被实例化 THEN 它 SHALL 在同一个 RocksDB 实例中使用名为 `delta_entries` 的新 Column Family。

3. WHEN `append_delta` 被调用时 THEN 系统 SHALL 为每个 `DeltaOp` 分配一个**per-inode 单调递增的序列号**，并将 `(encode_delta_key(inode, seq), serialize(op))` 写入 `delta_entries` CF。

4. IF 多个 `DeltaOp` 在同一次 `append_delta` 调用中提交 THEN 系统 SHALL 使用 RocksDB WriteBatch 保证原子性。

5. WHEN `scan_deltas` 被调用 THEN 系统 SHALL 使用前缀迭代器（prefix = `inode_id.to_be_bytes()`）扫描 `delta_entries` CF，按 key 字节序（即序列号升序）返回所有匹配的 delta。

6. WHEN `MemoryDeltaStore` 被使用 THEN 它 SHALL 使用内存数据结构（如 `BTreeMap<(Inode, u64), DeltaOp>`）提供与 `RocksDeltaStore` 相同的语义，用于单元测试。

---

### 需求 3：写路径改造 — Append-Only Delta

**用户故事：** 作为一名 RucksFS 开发者，我希望 `create`/`mkdir`/`unlink`/`rmdir`/`rename` 操作中对**父目录** inode 的属性修改（mtime、ctime、nlink 变化）改为追加 delta entries，以消除对父目录 inode 的 read-modify-write 瓶颈。

#### 验收标准

1. WHEN `create` 操作成功创建新文件 THEN 系统 SHALL 不再执行对父目录 inode 的 `load_inode` + `save_inode`（read-modify-write），而是调用 `delta_store.append_delta(parent, &[SetMtime(now), SetCtime(now)])`。

2. WHEN `mkdir` 操作成功创建新目录 THEN 系统 SHALL 对父目录追加 delta：`[IncrementNlink(1), SetMtime(now), SetCtime(now)]`。

3. WHEN `unlink` 操作成功删除文件 THEN 系统 SHALL 对父目录追加 delta：`[SetMtime(now), SetCtime(now)]`。

4. WHEN `rmdir` 操作成功删除目录 THEN 系统 SHALL 对父目录追加 delta：`[IncrementNlink(-1), SetMtime(now), SetCtime(now)]`。

5. WHEN `rename` 操作成功执行跨目录移动 THEN 系统 SHALL 分别对源父目录和目标父目录追加相应的 delta（时间戳更新；若移动的是目录，还需追加 nlink 增减 delta）。

6. IF 对子 inode 本身的属性修改（如 `unlink` 时子文件的 `nlink -= 1`，`rename` 时源文件的 `ctime` 更新）THEN 系统 SHALL 继续使用原有的 read-modify-write 模式（delta 机制仅用于父目录属性更新）。

7. WHEN delta append 与新建 inode / 目录项插入在同一操作中发生 THEN 系统 SHALL 将它们合并到同一个 WriteBatch 中，保证原子性。

---

### 需求 4：读路径改造 — Delta Fold

**用户故事：** 作为一名 RucksFS 开发者，我希望 `getattr`/`lookup` 等读操作能够正确反映所有已提交的 delta，即使这些 delta 尚未被 compaction 合并到基线 inode 中。

#### 验收标准

1. WHEN `getattr(inode)` 被调用 THEN 系统 SHALL 先读取基线 `InodeValue`（从 `inodes` CF），然后扫描该 inode 的所有未合并 delta（从 `delta_entries` CF），将 delta 按序列号顺序 fold 到基线上，返回 fold 后的 `FileAttr`。

2. WHEN fold 处理 `IncrementNlink(n)` delta THEN 系统 SHALL 将基线的 `nlink` 字段加上 `n`。

3. WHEN fold 处理 `SetMtime(t)` delta THEN 系统 SHALL 将基线的 `mtime` 字段设为 `max(base.mtime, t)`（取最大值，保证单调递增）。

4. WHEN fold 处理 `SetCtime(t)` / `SetAtime(t)` delta THEN 系统 SHALL 分别将 `ctime` / `atime` 字段设为 `max(base.ctime, t)` / `max(base.atime, t)`。

5. IF 某个 inode 没有任何未合并的 delta THEN `getattr` 的行为 SHALL 与当前实现完全一致（直接返回基线值，无额外开销）。

6. WHEN `lookup(parent, name)` 被调用 THEN 系统 SHALL 保持当前行为（通过 DirectoryIndex 查找 child inode），返回的 child `FileAttr` 通过 `getattr(child_inode)` 获取（已含 delta fold）。

---

### 需求 5：Server 端 Folded State 缓存

**用户故事：** 作为一名 RucksFS 开发者，我希望 server 端维护一个 inode folded state 的内存缓存，以避免每次读操作都需要扫描 delta entries（防止读放大）。

#### 验收标准

1. WHEN server 启动 THEN 系统 SHALL 创建一个基于 LRU 策略的 inode folded state 缓存，容量可配置（默认 10,000 条）。

2. WHEN `getattr(inode)` 被调用 AND 缓存命中 THEN 系统 SHALL 直接从缓存返回 folded `InodeValue`，不查询 RocksDB。

3. WHEN `getattr(inode)` 被调用 AND 缓存未命中 THEN 系统 SHALL 从 RocksDB 读取基线 + fold delta，将结果写入缓存后返回。

4. WHEN 一个新的 delta 被追加到 inode THEN 系统 SHALL 同步更新该 inode 在缓存中的 folded state（就地 apply delta），保证缓存与持久层一致。

5. IF 缓存容量已满且需要插入新条目 THEN 系统 SHALL 按 LRU 策略淘汰最久未访问的条目。

6. WHEN compaction worker 将某个 inode 的 delta 合并到基线后 THEN 系统 SHALL 使缓存中该 inode 的条目失效（invalidate），下次访问时重新从基线加载。

---

### 需求 6：后台 Delta Compaction

**用户故事：** 作为一名 RucksFS 开发者，我希望有一个后台线程定期将累积的 delta entries 合并回基线 inode 值，以控制 delta 的增长、减少存储开销并防止读放大。

#### 验收标准

1. WHEN server 启动 THEN 系统 SHALL 启动一个后台 compaction worker 线程。

2. WHEN compaction worker 发现某个 inode 的 delta 数量超过阈值（默认 100 条） THEN worker SHALL 执行合并操作。

3. WHEN compaction 执行时 THEN worker SHALL 按以下步骤操作：
   - (a) 读取基线 `InodeValue`
   - (b) 扫描并 fold 所有 delta
   - (c) 使用 RocksDB WriteBatch **原子地**写入新基线 + 删除所有已合并 delta
   - (d) 使缓存中该 inode 的条目失效

4. IF compaction 过程中发生崩溃 THEN 系统 SHALL 保证以下不变式之一成立：
   - 旧基线 + 所有 delta 完好（WriteBatch 未提交）
   - 新基线已写入 + delta 已清除（WriteBatch 已提交）

5. WHEN compaction worker 运行时 THEN 它 SHALL 不阻塞前台读写操作（compaction 在独立线程中执行）。

6. WHEN server 关闭（graceful shutdown）THEN 系统 SHALL 停止 compaction worker 并等待当前正在进行的 compaction 完成。

---

### 需求 7：序列号管理

**用户故事：** 作为一名 RucksFS 开发者，我希望每个 inode 的 delta 序列号严格单调递增，以保证 delta fold 时的因果序正确性。

#### 验收标准

1. WHEN 一个新的 delta 被追加到 inode THEN 系统 SHALL 分配一个比该 inode 当前最大序列号严格大 1 的新序列号。

2. IF 同一 inode 在高并发下同时追加 delta THEN 系统 SHALL 通过原子操作（如 `AtomicU64::fetch_add`）保证序列号的唯一性和单调性。

3. WHEN server 重启 THEN 系统 SHALL 通过扫描 `delta_entries` CF 中该 inode 的最大 key 来恢复序列号状态（或从 `system` CF 持久化的计数器恢复）。

4. IF 某个 inode 被 compaction 清除所有 delta 后 THEN 该 inode 的序列号 SHALL 重置为 0（因为不再有未合并 delta，新 delta 从 0 开始即可）。

---

### 需求 8：测试验证

**用户故事：** 作为一名 RucksFS 开发者，我希望 delta entries 功能有完整的测试覆盖，以确保 append、fold、compaction、缓存一致性和崩溃恢复的正确性。

#### 验收标准

1. WHEN delta 编码/解码被测试 THEN 测试 SHALL 验证所有 `DeltaOp` 变体的 round-trip 正确性。

2. WHEN delta fold 被测试 THEN 测试 SHALL 验证：
   - (a) 空 delta 列表 → 返回基线值
   - (b) 单个 `IncrementNlink(1)` → nlink 增加 1
   - (c) 多个时间戳 delta → 取最大值
   - (d) 混合 delta 序列的 fold 结果正确

3. WHEN 写路径被测试 THEN 测试 SHALL 验证：
   - (a) `create` 后父目录的 `mtime`/`ctime` 通过 `getattr` 获取时已更新
   - (b) `mkdir` 后父目录的 `nlink` 通过 `getattr` 获取时已增加 1
   - (c) `unlink` 后父目录的时间戳通过 `getattr` 获取时已更新
   - (d) `rmdir` 后父目录的 `nlink` 通过 `getattr` 获取时已减少 1

4. WHEN compaction 被测试 THEN 测试 SHALL 验证：
   - (a) compaction 后基线反映了所有 delta
   - (b) compaction 后 `delta_entries` CF 中该 inode 的 delta 已清除
   - (c) compaction 后 `getattr` 返回的值与 compaction 前一致

5. WHEN 缓存被测试 THEN 测试 SHALL 验证：
   - (a) 缓存命中时不查询 RocksDB
   - (b) 新 delta 追加后缓存中的值已同步更新
   - (c) compaction 后缓存条目已失效

6. WHEN 并发测试被执行 THEN 测试 SHALL 在同一父目录下并发创建 100+ 文件，验证所有文件创建成功且父目录的 `nlink` 和时间戳正确。

7. WHEN 现有测试套件运行 THEN 所有现有的 99 个测试 SHALL 继续通过（无回归）。
