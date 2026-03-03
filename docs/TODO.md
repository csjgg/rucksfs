# RucksFS — 待办事项 & 已知问题

> 本文档记录项目已知的待修复问题、架构待办和改进建议。
> 按优先级排序，✅ 表示已完成，⬜ 表示待处理。

---

## 架构说明

### Demo 定位

Demo 是一个**单机一体化**的完整 FUSE 文件系统（编译为单个二进制），而非简化的演示程序。它与分布式版本的唯一区别是：所有组件（MetadataServer + DataServer + Client）运行在同一个进程内，不经过 gRPC。

- **Demo 的 metadata 后端应使用 RocksDB**（`--persist` 模式），而不是仅使用内存后端
- 内存后端（`MemoryMetadataStore` / `MemoryDirectoryIndex` / `MemoryDeltaStore` / `MemoryDataStore`）仅用于**单元测试和集成测试**
- Demo 的目标是编译出一个可以直接挂载使用的、完整的单机 FUSE 文件系统

---

## 待处理问题

### ⬜ MemoryWriteBatch::commit 不是真正原子的（P2 — 低优先级）

- **影响**：仅影响内存后端（测试环境），生产环境不受影响
- **问题**：每个操作分别获取/释放 RwLock，两个 batch 的 commit 可能交错执行，读者可能看到中间状态
- **建议**：在 commit 内一次性获取所有 store 的 write lock 后再执行操作；或在注释中标注此限制

### ⬜ RocksDB 路径下 batch + insert_child 双写（P2 — 低优先级）

- **影响**：幂等无害，浪费少量 I/O
- **问题**：`create`/`mkdir` 中 batch commit 已写入 dir_entry 到 RocksDB，commit 后又调用 `self.index.insert_child()` 再写一次
- **建议**：通过 trait method（如 `index.is_persistent()`）区分，仅在 MemoryDirectoryIndex 路径下调用 `insert_child`

### ⬜ LRU cache get() 复杂度为 O(n)（P2 — 性能优化）

- **影响**：10000 容量的缓存下，每次 `get`/`put` 需要 O(10000) 的锁内线性扫描
- **问题**：`inner.order.retain(|&i| i != inode)` 每次调用都全量扫描 VecDeque
- **建议**：使用 `lru` crate 或 `linked-hash-map` crate 替换自制 LRU，获得 O(1) 的 get/put

---

## 未来改进

### ⬜ Chunk/Slice 数据模型

- 文件按 64MB Chunk 分片管理元数据
- `open` 时返回完整的数据映射信息
- `report_write` 的 Chunk 范围计算和 Slice 分配

### ⬜ 延迟 GC 机制

- `unlink` 记录 PendingDelete
- 后台 GcWorker 异步清理 Chunk 元数据和数据

### ⬜ fsck / 一致性检查工具

- 检测并清理孤儿 inode（无 dir_entry 指向的 inode）
- 验证 nlink 计数一致性
- 修复 next_inode 计数器
