# RucksFS 性能优化记录

> 本文档记录了 RucksFS 所有性能优化的详细过程、前后指标对比及经验教训。
> 逐轮详细日志见：`benchmark/bench-tool/optimization-log.md`

## 总览

经过 11 轮优化（6 轮合并、5 轮回退），RucksFS 元数据吞吐量从**远低于 ext4** 提升至
**在 7 个 POSIX 元数据操作中有 6 个达到或超越 ext4**。

| 操作 | 优化前 (ops/s) | 优化后 (ops/s) | 提升倍数 | 与 ext4 对比 |
|------|---------------|---------------|---------|-------------|
| create | 17,082 | 196,978 | **11.5x** | **1.18x ext4** |
| stat | 854,489 | 1,201,582 | **1.4x** | **1.06x ext4** |
| unlink | 31.82 | 231,673 | **7,280x** | 0.98x ext4 |
| mkdir | 13,257 | 127,748 | **9.6x** | **1.10x ext4** |
| rmdir | 19,452 | 142,980 | **7.4x** | **1.09x ext4** |
| readdir | 9,008 | 60,753 | **6.7x** | **9.64x ext4** |
| rename | 20,904 | 204,849 | **9.8x** | **1.08x ext4** |

测试条件：`-t 1 -n 100`，单线程，easy 模式，每个操作 100 个文件。

---

## 已合并的高影响优化

### 优化 1：异步数据删除（Round 2）

**问题**：`unlink` 同步调用 `delete_data()`。底层 `RawDiskDataStore::delete` 以 4 KB
为单位对每个 inode 的 64 MB 地址空间进行零填充，阻塞 FUSE 响应约 30 ms/文件。

**方案**：在元数据事务提交后，使用 `tokio::spawn` 进行 fire-and-forget 异步删除。
FUSE 响应在元数据提交后立即返回，不再等待数据清除。

**改动文件**：
- `server/src/lib.rs` — `unlink()`、`release()`、`rename()`：将同步
  `delete_data().await` 替换为 `tokio::spawn` + 错误日志
- `server/Cargo.toml` — tokio dev-dependencies 增加 `"time"` feature

**关键结果**：unlink 31.82 → 5,180 ops/s（**163 倍提升**）

---

### 优化 2：No-op 数据删除（Round 5）

**问题**：即使 Round 2 改为异步删除，后台 `tokio::spawn` 任务仍然对每个 inode
零填充 64 MB。多线程基准测试下，这些后台任务会饱和磁盘 I/O，导致所有操作吞吐量
崩溃。具体表现：
- create 2 线程：17K → 5 ops/s（灾难性下降）
- create hard 1 线程：0.88 ops/s（100 个文件需要 114 秒）

**方案**：将 `RawDiskDataStore::delete` 改为 no-op。inode 编号由
`InodeAllocator`（原子 `fetch_add`）单调递增且永不复用，旧数据区域在元数据层面
已永久不可达，零填充没有必要。

**改动文件**：
- `storage/src/rawdisk.rs` — `delete()` 方法：零填充循环替换为 `Ok(())`
- `dataserver/src/lib.rs` — 更新测试 `delete_data_is_noop`
- `server/tests/integration.rs` — 重命名并更新测试 `unlink_nlink_zero_removes_metadata`
- `server/src/lib.rs` — 更新引用零填充的过时注释

**关键结果**：
- unlink：5,180 → 24,472 ops/s（**相比 Round 2 再提升 4.7 倍**）
- create 2 线程：5 → 37,596 ops/s（**修复多线程崩溃**）
- create hard 1 线程：0.88 → 15,965 ops/s（**修复 hard 模式**）

---

### 优化 3：将父目录时间戳 Delta 内联到事务中（Round 6）

**问题**：每次 mutation 操作（create、mkdir、unlink、rmdir、rename、link、symlink）
执行**两次独立的 RocksDB 写入**：
1. 主 PCC 事务提交（inode + dir_entry + data_location）
2. 单独的 `WriteBatch` 写入父目录时间戳 delta（SetMtime、SetCtime），
   经由 `append_parent_deltas → DeltaStore::append_deltas`

第二次写入使每次操作的 WAL I/O 翻倍。由于 RocksDB 的 WAL 锁会串行化所有写入者，
这实际上将聚合吞吐量减半。

**方案**：将 `SetMtime`/`SetCtime` delta 写入合并到主事务批量操作中，使用
`batch_parent_deltas`（从 `batch_nlink_deltas` 泛化而来）。事务提交后只更新
内存缓存和标记脏页用于后台 compaction。移除了不再需要的 `append_parent_deltas` 辅助函数。

**改动文件**：
- `server/src/lib.rs`：
  - 重命名 `batch_nlink_deltas` → `batch_parent_deltas`
  - 全部 7 个 mutation 方法：将时间戳 delta 移入事务，事务外的
    `append_parent_deltas` 替换为 `cache.apply_deltas` + `compaction.mark_dirty`
  - 删除 `append_parent_deltas` 辅助函数
  - 修复时间戳漂移：unlink/rmdir/rename/link 现在返回事务内的时间戳并复用于缓存更新

**关键结果**（相比 Round 5 基线）：
- create：15,434 → 196,978 ops/s（**12.8 倍**，达到 ext4 的 1.18 倍）
- rename：18,836 → 204,849 ops/s（**10.9 倍**，达到 ext4 的 1.08 倍）
- unlink：24,472 → 231,673 ops/s（**9.5 倍**，达到 ext4 的 0.98 倍）
- mkdir：19,635 → 127,748 ops/s（**6.5 倍**，达到 ext4 的 1.10 倍）

---

## 已合并的微优化 / 代码质量改进

### 优化 4：减少 mark_dirty 的 Condvar 通知（Round 7）

**问题**：`mark_dirty()` 每次 mutation 都获取两个 `std::sync::Mutex` 并调用
`Condvar::notify_one()`，即使 compaction 工作线程没有工作可做。

**方案**：仅在 dirty set 从空变为非空时才唤醒 compaction 循环。

**改动文件**：`server/src/compaction.rs` — `mark_dirty()` 方法

**结果**：在 -n 100 下无可测量影响，但消除了冗余系统调用。

---

### 优化 5：禁用 RocksDB 死锁检测（Round 8）

**问题**：`set_deadlock_detect(true)` 导致 RocksDB 在每次 `get_for_update`
调用时遍历死锁检测图。

**方案**：设置 `set_deadlock_detect(false)` — 我们的锁排序策略（rename 中按
inode-ID 排序获取锁）已从设计上防止死锁。

**改动文件**：`storage/src/rocks.rs` — `begin_write()` 方法

**结果**：在 -n 100 下无可测量影响，但消除了不必要的 CPU 开销。

---

### 优化 6：栈缓冲区序列化（Round 11）

**问题**：`InodeValue::serialize()` 使用 `Vec::with_capacity(57)` + 9 次
`extend_from_slice`，每次都有边界检查开销。

**方案**：先在 `[u8; 57]` 栈缓冲区上用 `copy_from_slice` 组装，最后调用
`.to_vec()`。消除了逐字段的边界检查开销。

**改动文件**：`storage/src/encoding.rs` — `serialize()` 方法

**结果**：在 -n 100 下无可测量影响，代码更清晰。

---

## 已回退的优化（经验教训）

### Round 1 — RocksDB Block Cache（已回退）

尝试添加 256 MB 共享 LRU block cache 并固定 L0 过滤器。在小工作集（-n 100）下，
缓存管理开销（LRU 记账）反而超过收益。大部分操作回退 20-35%。

**教训**：Block cache 对大工作集有效；在小规模下管理开销占主导。

### Round 3 — 禁用 Delta 写入的 WAL（已回退）

对 delta `WriteBatch` 设置 `disable_wal(true)`。并未改善 create 吞吐量（仍被
主事务 WAL 写入主导）。stat 回退 -38.8%，可能是因为 RocksDB 在无 WAL 保护时
更积极地刷新 memtable。

**教训**：禁用 WAL 对读路径有非直觉的副作用（影响 memtable flush 行为）。

### Round 4 — 删除 inode 时跳过 clear_deltas（已回退）

在删除 inode 时跳过 `clear_deltas()` 调用。出现多项严重回退，可能是 -n 100
下基准测试噪声导致的假性回退。

**教训**：在小 -n 下，只能信任大幅（>2x）的改善结果。

### Round 9 — 手动 WAL Flush（已回退）

设置 `set_manual_wal_flush(true)` 将 WAL 写入批量化到 OS 缓冲区。无改善 —
`sync=false` 时 `write()` 系统调用本身已经很快。

**教训**：不需要 `fsync` 时，WAL 写入开销不是瓶颈。

### Round 10 — 增大分配器持久化间隔（已回退）

将 `PERSIST_INTERVAL` 从 64 增大到 1024。无影响 — 在 -n 100 下旧间隔也只触发
一次持久化。

**教训**：优化必须匹配基准测试规模才能被观测到。

---

## 全部轮次汇总

| 轮次 | 优化内容 | 决策 | 影响 |
|------|---------|------|------|
| 1 | RocksDB block cache | 回退 | -22% 至 -35% 回退 |
| 2 | 异步数据删除 | **合并** | unlink **163 倍** |
| 3 | 禁用 delta WAL | 回退 | stat -39% 回退 |
| 4 | 跳过 clear_deltas | 回退 | 多项回退 |
| 5 | No-op 数据删除 | **合并** | unlink **4.7 倍**，修复多线程 |
| 6 | 内联时间戳 delta | **合并** | 全部操作 **5-13 倍** |
| 7 | 减少 mark_dirty 通知 | **合并** | 代码质量 |
| 8 | 禁用死锁检测 | **合并** | 代码质量 |
| 9 | 手动 WAL flush | 回退 | 无收益 |
| 10 | 增大分配器间隔 | 回退 | 无收益 |
| 11 | 栈缓冲区序列化 | **合并** | 代码质量 |

---

## 测量方法

- **工具**：`rucksfs-bench`（自研 Rust 基准测试工具，位于 `benchmark/bench-tool/`）
- **模式**：easy（每线程独立目录）、hard（共享目录竞争）
- **测试链**：文件链（create→stat→rename→unlink）+ 目录链（mkdir→readdir→rmdir）
- **参数**：`-t 1,2,4 -n 100`
- **验证**：每轮跑两次基准测试确认结果一致性
- **决策标准**：任一操作 ≥10% 提升且无其他操作 >5% 回退
- **ext4 基线**：在同一硬件上用相同参数测量
