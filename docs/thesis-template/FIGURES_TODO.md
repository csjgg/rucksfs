# 待画的图 — 交付说明

本文档给**画图的人**（或任何其它画图工具/AI）使用。一共 3 张图，都用于 RucksFS 毕业论文（中文）。

## 通用规范

- **风格**：严格**黑白**或灰阶，不要任何彩色；学术论文插图。
- **字体**：正文用中文学术字体（宋体或 Noto Sans CJK）；代码/变量用等宽字体。
- **字号**：主体文字至少 10–12 pt（按论文排版后仍能清楚阅读）；说明性小字可到 8–9 pt。
- **格式**：输出 **矢量 PDF**（首选）或高分辨率 SVG。放到 `docs/thesis-template/images/`。
- **背景**：白色；方框填充用浅灰（gray90–gray95）区分层级，不要纯黑填充。
- **引用方式**：LaTeX 用 `\includegraphics[width=0.9\textwidth]{images/文件名.pdf}`。

---

## 图 1：LSM-tree 架构（放 related_works.tex §2.2.1 LSM-tree 数据结构）

**文件名**：`lsm_tree.pdf`
**论文中的 caption**：`LSM-tree 的写入、读取与 Compaction 路径`
**label**：`fig:lsm_tree`

### 画什么

一张横向布局的图，分**左右两大块**：

**左侧（内存区）**：
- 上面一个 **WAL** 方框（标注"追加日志，fsync 持久化"）
- 中间一个 **Active MemTable** 方框（标注"跳表结构，≈ 64 MiB"）
- 下面一个 **Immutable MemTable** 方框（标注"冻结，只读，等待刷盘"）

**右侧（磁盘层）**：画成**金字塔状**，层越深越宽：
- **L0**：3 个小 SST 方框横排（旁注"可重叠"）
- **L1**：5 个 SST 方框横排（旁注"不重叠，≈ 256 MiB"）
- **L2**：8 个 SST 方框横排（旁注"≈ 2.56 GiB"）
- L2 下面放一行省略号 `... Ln (按 ~10× fanout 指数增长) ...`

**左上方或图外**：两个触发点 **Put / Write** 和 **Get / Read**

### 连线

**三种不同样式的箭头**，要清晰可辨：

1. **写路径（粗实线）**：
   - `Put → WAL`，标签 "① 先写 WAL"
   - `Put → Active MemTable`，标签 "② 写入 MemTable"
   - `Active → Immutable`，标签 "③ 满则冻结"
   - `Immutable → L0`（箭头落在 L0 的 SST 条上），标签 "④ Flush"

2. **读路径（虚线）**：
   - `Get → Active → Immutable → L0 → L1 → L2`，依次查找（可只画一条从 Get 出发、经各层的虚线即可，或画 R1 / R2 / R3 多条分叉）

3. **Compaction 路径（点线）**：
   - `L0 → L1`（标 "Compaction"）
   - `L1 → L2`（标 "Compaction"）
   - 表示"后台把上层 SST 合并下沉"

### 图例

右上角或左下角**独立小框**，包含 **实际样式的示例线 + 对应文字说明**：

```
━━━━→ 写路径
- - -→ 读路径
······→ Compaction
```

**不要**画一个只有文字没有示例线的空框（之前的错误）。

### 参考

- 搜 "RocksDB LSM-tree architecture" 的经典画法；或参考 MyRocks / BigTable 论文的 LSM 插图
- 要传达的核心信息：**写是顺序追加+下沉，读是多层合并查找**

---

## 图 2：DeltaOp 懒折叠机制（放 method.tex §3.4.3 增量更新与读写一致性）

**文件名**：`delta_lazy_fold.pdf`
**论文中的 caption**：`DeltaOp 增量追加与懒折叠机制`
**label**：`fig:delta_lazy_fold`

### 画什么

**三列结构**（横向排列）：

**左列（触发源）**，三个方框垂直排列：
1. **写操作**：`create / unlink / mkdir / rmdir`
2. **读操作**：`getattr / lookup`
3. **后台折叠线程**：`DeltaCompactionWorker`（小字注"每 5 秒扫描 dirty inode"）

**中列（缓存层）**，一个**虚线边框**方框：
- `InodeFoldedCache`（小字注"已折叠结果的 LRU 缓存"）

**右列（RocksDB 列族）**，两个方框垂直排列：
1. **inodes 列族**：基础 InodeValue 记录（小字注"低频更新"）
2. **delta_entries 列族**：DeltaOp 增量序列（小字注"IncrementNlink / SetMtime / SetCtime / SetAtime"）

### 连线

**三条不同路径**：

1. **写路径（粗实线）**：`写操作 → delta_entries 列族`，标签"① 追加一条 DeltaOp（无读取，无锁争抢）"
2. **读路径（虚线）**：
   - `读操作 → InodeFoldedCache`，标"② 先查缓存"
   - `InodeFoldedCache → inodes 列族`，标"③a 未命中：读基础值"
   - `InodeFoldedCache → delta_entries`，标"③b 叠加未折叠增量"
3. **后台折叠（点线）**：
   - `Worker → delta_entries`，标"④ 读取累积增量（>32 条触发）"
   - `Worker → inodes 列族`，标"⑤ 折叠后写回基值并清除已消费增量"

### 图例

同图 1，右上角小框内用 **实际样式示例线 + 文字**：
- ━━━━→ 写路径
- - - - → 读路径
- ······→ 后台折叠

### 要传达的核心信息

**写只追加，不触基值** → 读时动态合成 → 阈值到了后台才真正折叠回基值

---

## 图 3：rename 统一加锁顺序（放 method.tex §3.5.3 rename 的算法 2 之后）

**文件名**：`rename_lock_order.pdf`
**论文中的 caption**：`rename 操作的统一加锁顺序`
**label**：`fig:rename_lock_order`

### 画什么

**顶部一行**：展示涉及的 4 个 inode 按编号排序后排列
```
┌─────┬──────┬──────┬──────┐
│ #5  │ #17  │ #42  │ #99  │
│源父 │目标父│被移动│被覆盖│
└─────┴──────┴──────┴──────┘
              ↑
  涉及的 inode（按编号升序排序后加锁）
```

**中间两条时间线**（水平的，从左到右表示时间推进）：

**时间线 1：事务 T₁（先到）**
- 起点标 "T₁ 开始"
- 水平排四个动作方框：
  - `lock(#5) 成功`
  - `lock(#17) 成功`
  - `lock(#42) 成功`
  - `commit，释放全部锁`

**时间线 2：事务 T₂（后到）**
- 起点标 "T₂ 开始"
- 水平排两个方框：
  - `lock(#5) 阻塞等待`（灰色填充 + 虚线框，表示阻塞状态）
  - `获得 #5 后继续加后续锁`

**跨时间线的两条点线箭头**（表示 T₁ 影响 T₂）：
- 从 T₁ 的 `lock(#5) 成功` 垂直向下指到 T₂ 的 `lock(#5) 阻塞等待`，标"#5 被 T₁ 持有"
- 从 T₁ 的 `commit 释放` 斜向指到 T₂ 的 `获得 #5 后继续`，标"释放后唤醒 T₂"

**底部放一段说明框**：

> **为什么不会死锁**：所有事务按 inode 编号升序加锁，锁序全局一致，因此不会出现 "T₁ 等 #17、T₂ 等 #5" 的循环等待。

### 要传达的核心信息

**统一锁序 → 后到的只会排队，不会和先到形成循环依赖 → 死锁被消除**

---

## 补充：之前 main.tex 里已有的图（质量尚可，不用重画）

论文里原有的这 7 张 TikZ 图质量是合格的，**不需要替换**：

| label | 位置 |
|-------|------|
| `fig:ext4-create-path` | intro - ext4 写入路径 |
| `fig:three_challenges` | intro - 三个设计问题 |
| `fig:overall_arch` | method - 整体架构 |
| `fig:cf_kv_layout` | method - 列族布局 |
| `fig:key_mapping` | method - 键映射 |
| `fig:perf-create-curve` | experiments - create 吞吐曲线（pgfplots）|
| `fig:delta-nodelta-curve` | experiments - Delta vs NoDelta 曲线（pgfplots）|

---

## 交付后集成到论文

画好 3 张 PDF 后，将它们放到 `docs/thesis-template/images/`，然后我会帮你把 LaTeX 里插入三段 `\includegraphics` 到对应位置，不用你动 tex 源码。

也可以让 AI 工具（ChatGPT + 自带的画图能力、diagrams.net / draw.io、Miro、或者 Figma）按此描述画。先画一张出来给我确认风格，再画剩下两张。
