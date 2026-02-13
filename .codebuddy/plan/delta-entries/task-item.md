# 实施计划

- [ ] 1. 定义 `DeltaOp` 枚举与序列化/反序列化
   - 在 `server/src/delta.rs`（新建）中定义 `DeltaOp` 枚举：`IncrementNlink(i32)`、`SetMtime(u64)`、`SetCtime(u64)`、`SetAtime(u64)`
   - 实现 `DeltaOp::serialize() -> Vec<u8>` 和 `DeltaOp::deserialize(&[u8]) -> FsResult<DeltaOp>`，格式为 `[op_type: u8][payload: BE]`
   - 实现 delta fold 函数 `fold_deltas(base: &mut InodeValue, deltas: &[DeltaOp])`，按语义逐个 apply delta 到基线
   - 编写单元测试：round-trip 正确性、fold 正确性（空列表、单 delta、混合 delta）
   - _需求：1.1, 1.2, 1.3, 4.2, 4.3, 4.4, 8.1, 8.2_

- [ ] 2. 新增 Delta key 编码函数
   - 在 `storage/src/encoding.rs` 中新增 `encode_delta_key(inode: u64, seq: u64) -> [u8; 16]`（inode BE + seq BE）
   - 新增 `decode_delta_key(key: &[u8]) -> (u64, u64)` 反向解码
   - 新增 `delta_prefix(inode: u64) -> [u8; 8]` 用于前缀迭代
   - 编写单元测试：编码/解码 round-trip、同一 inode 的 key 按 seq 字节序排列
   - _需求：1.4, 1.5_

- [ ] 3. 定义 `DeltaStore` trait 并实现 `MemoryDeltaStore`
   - 在 `storage/src/lib.rs` 中定义 `DeltaStore` trait：`append_delta`、`scan_deltas`、`clear_deltas`
   - 在 `storage/src/memory.rs` 中实现 `MemoryDeltaStore`，使用 `BTreeMap<(Inode, u64), DeltaOp>` + per-inode `AtomicU64` 序列号
   - 编写单元测试：append → scan 返回正确顺序、clear 后 scan 为空、并发 append 序列号唯一
   - _需求：2.1, 2.6, 7.1, 7.2_

- [ ] 4. 实现 `RocksDeltaStore`（RocksDB `delta_entries` CF）
   - 在 `storage/src/rocks.rs` 中注册新的 `delta_entries` Column Family
   - 实现 `RocksDeltaStore`：`append_delta` 使用 WriteBatch 原子写入，`scan_deltas` 使用前缀迭代器，`clear_deltas` 使用 DeleteRange 或逐 key 删除
   - 实现 per-inode 序列号管理：内存中用 `DashMap<Inode, AtomicU64>` 跟踪，启动时通过扫描 CF 恢复最大序列号
   - 编写集成测试：append/scan/clear 的 RocksDB 持久化验证
   - _需求：2.2, 2.3, 2.4, 2.5, 7.1, 7.2, 7.3, 7.4_

- [ ] 5. 实现 Server 端 Folded State 缓存
   - 新建 `server/src/cache.rs`，实现基于 LRU 策略的 `InodeFoldedCache`（容量可配置，默认 10,000）
   - 提供方法：`get(inode)` 返回缓存的 folded `InodeValue`、`put(inode, value)` 插入、`apply_delta(inode, delta)` 就地更新、`invalidate(inode)` 失效
   - 使用 `lru` crate 或手写 LRU（基于 `LinkedHashMap`），内部使用 `Mutex` 保证线程安全
   - 编写单元测试：命中/未命中、apply_delta 后值更新、LRU 淘汰、invalidate 后重新加载
   - _需求：5.1, 5.2, 5.3, 5.4, 5.5, 5.6, 8.5_

- [ ] 6. 改造 `MetadataServer` 写路径：delta append 替代 read-modify-write
   - 为 `MetadataServer` 注入 `DeltaStore` 和 `InodeFoldedCache` 依赖
   - 改造 `create`：父目录属性更新改为 `append_delta(parent, [SetMtime, SetCtime])`，与子 inode 创建合并到同一 WriteBatch
   - 改造 `mkdir`：父目录追加 `[IncrementNlink(1), SetMtime, SetCtime]`
   - 改造 `unlink`：父目录追加 `[SetMtime, SetCtime]`（子 inode nlink 更新保持 read-modify-write）
   - 改造 `rmdir`：父目录追加 `[IncrementNlink(-1), SetMtime, SetCtime]`
   - 改造 `rename`：源/目标父目录分别追加对应 delta
   - 每次 append 后同步更新 `InodeFoldedCache`
   - _需求：3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 5.4_

- [ ] 7. 改造 `MetadataServer` 读路径：getattr/lookup 支持 delta fold
   - 改造 `getattr`：先查缓存，命中则直接返回；未命中则读基线 + scan delta + fold，结果写入缓存
   - 确保 `lookup` 返回的 `FileAttr` 经过 fold（通过复用 `getattr` 逻辑）
   - 确保无 delta 时行为与原有实现完全一致（无额外开销）
   - 编写集成测试：create 后 getattr 返回更新的 mtime、mkdir 后 nlink +1、连续操作后属性正确
   - _需求：4.1, 4.5, 4.6, 5.2, 5.3, 8.3_

- [ ] 8. 实现后台 Delta Compaction Worker
   - 在 `server/src/delta.rs` 中实现 `DeltaCompactionWorker`，作为独立后台 tokio task 运行
   - 周期性扫描 `delta_entries` CF，发现某 inode delta 数量超过阈值（默认 100）时触发合并
   - 合并流程：读基线 → fold delta → WriteBatch 原子写入新基线 + 删除 delta → invalidate 缓存 → 重置序列号
   - 实现 graceful shutdown：通过 `tokio::sync::watch` 或 `CancellationToken` 通知停止，等待当前 compaction 完成
   - 在 `MetadataServer` 启动/关闭时管理 compaction worker 生命周期
   - 编写测试：compaction 后基线正确、delta 已清除、getattr 结果一致
   - _需求：6.1, 6.2, 6.3, 6.4, 6.5, 6.6, 7.4, 8.4_

- [ ] 9. 全量回归测试与并发测试
   - 运行现有全部 99 个测试，确保零回归
   - 新增并发测试：在同一父目录下并发创建 100+ 文件，验证所有文件成功创建且父目录 nlink/时间戳正确
   - 新增 compaction 后一致性测试：大量 delta 累积 → 触发 compaction → 验证结果
   - _需求：8.6, 8.7_
