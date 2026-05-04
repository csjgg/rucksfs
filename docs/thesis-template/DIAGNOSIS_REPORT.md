# RucksFS 论文 AI 痕迹 & 语言诊断报告

> 扫描工具：`humanizer`（24-pattern）+ `ppw:de-ai`（三维度 + AI 词汇表）
> 扫描范围：`body/*.tex` 共 1413 行
> 扫描日期：2026-05-04
> **本报告仅诊断，不修改任何文件。**

---

## 🎯 总评

| 维度 | 评分 | 说明 |
|------|------|------|
| **humanizer 24 pattern** | 🟢 **优秀** | 英文摘要 24 类命中 2 类（1×"Additionally"、4×`---`），中文正文几乎全清 |
| **ppw:de-ai AI 词表** | 🟢 **优秀** | 70+ 个 AI 高频词中，命中词均为技术语境（如"成熟"描述 RocksDB 6.0 接口） |
| **意义膨胀 / 空泛断言** | 🟢 **优秀** | 零命中 "具有重要意义 / 奠定基础 / 划时代"；所有判断带数据或引用 |
| **促销式表达** | 🟢 **优秀** | 零命中 "卓越/先进/领先" 用作修饰词；"领先 X 倍" 全部指实测吞吐比值 |
| **翻译腔** | 🟢 **优秀** | 未发现 "不是单独存储的整体实体" 类翻译腔 |
| **术语一致性** | 🟡 **有问题** | 「本文」18 次 vs 「本课题」30 次，章节间未区分 |
| **段落参差** | 🟢 **优秀** | 长短句自然交替，未出现机械重复骨架 |

**结论：按 humanizer/de-ai 标准，这篇论文 AI 痕迹很低。**真正值得修的只有 2 处。

---

## 📊 按风险级别的命中清单

### 🔴 High Risk：0 条

无。未发现任何高风险命中（意义膨胀、促销式、空泛权威归因）。

---

### 🟡 Medium Risk：3 条

#### M1. 英文摘要中 `Additionally` × 1（humanizer #7 AI vocabulary）

**位置**：`body/abstract-en.tex` 实际文本未命中（grep 结果来自正则误触发边界）
**实测**：只是 `additionally` 作 "additionally involves" 中的副词用法，属正常技术描述
**建议**：**保留**。非模板开头，是"额外涉及"的技术说明。

#### M2. 英文摘要 em dash `---` × 4 处（humanizer #13）

**位置**：`body/abstract-en.tex` L3/L5/L7/L11
**上下文举例**：
> `file creation, deletion, renaming, and the like---account for ...`
> `RucksFS---a user-space file system backed by RocksDB---with ...`

**humanizer 规则**：LLM 滥用 em dash 模仿"有力"销售文风
**本文判断**：🟡 **部分可优化**
- L3 `renaming, and the like---account` → 可改逗号
- L5/L7 `RucksFS---...---with` → 其实是正当的同位语插入，英文学术写作允许
- L7 `manipulations---create inserts...` → 并列说明，可改冒号 + 分号结构
- L11 `modifying---create must first...` → 同上

**建议**：L3 可小改；L5/L7/L11 保留（符合英文学术规范）。

#### M3. 术语混用：「本文」vs「本课题」

**数据**：

| 章节 | 本文 | 本课题 | 倾向 |
|------|------|--------|------|
| abstract-ch | 0 | 3 | **本课题** |
| introduction | 0 | 27 | **本课题** |
| related_works | 6 | 0 | **本文** |
| method | 10 | 0 | **本文** |
| experiments | 1 | 0 | **本文** |
| conclusion | 1 | 0 | **本文** |

**humanizer 规则 #11 Elegant Variation**：AI 会不必要地换用同义词
**本文判断**：🟡 **可以优化**
- 事实上已天然形成"区分使用"模式：**绪论谈"本课题"（谈选题、贡献）**，**其余章节用"本文"（谈写作本身）**
- 但不彻底，还有 7 处漏网之鱼需要归位

**具体漏网位置**（建议修正方向）：

| 位置 | 现状 | 建议 |
|------|------|------|
| `abstract-ch.tex` L5/L7/L15 | "本课题基于…" / "本课题引入…" / "本课题采用…" | → 保留 |
| `introduction.tex` 27 处 | 全部 "本课题" | → 保留（绪论谈选题/贡献，合理） |
| `related_works.tex` L183 | "本课题借鉴了 Mantle" | → 改 **"本文"**（这是讨论本文的借鉴关系） |
| `related_works.tex` L187/L195/L201/L203/L205 | 6 处 "本课题" | → 改 **"本文"** |
| `method.tex` L12/L305 | "本文对元数据组织方式的分析" | → 保留（method 章已用"本文"） |

---

### 🟢 Optional（低风险，建议复审）：5 条

#### O1. `至关重要` × 1（introduction L142）
> "要实现这一简化，键的编码方式至关重要。"
- **ppw:de-ai 判断**：AI 高频词
- **本文语境**：技术结论句，属于作者判断
- **建议**：🟡 可替换为 "是核心关键" 或 "直接决定可行性"；也可保留

#### O2. `系统性地` × 7（introduction × 3，其余 × 4）
- **ppw:de-ai 判断**：AI 偏好副词
- **本文语境**：如 "系统性地使用 PCC 事务"、"系统性地应用于文件系统元数据"
- **建议**：🟢 保留。学术语境合理，不是填充词

#### O3. `深度` × 4
- 均为 "深度优化"、"深度调研" 等合成词
- **建议**：🟢 保留

#### O4. `既...也 / 既...又` × 2（negative parallelism 变体）
- `introduction.tex` "既关注…也补足…"
- 一共 2 处，远低于 humanizer "连续使用 3+ 次" 阈值
- **建议**：🟢 保留

#### O5. `不是...而是` × 15（humanizer #9 negative parallelism）
- 单次阈值：humanizer 允许偶尔使用；超过 **3 处即触发**
- **本文判断**：🟡 **偏高，但语义必要**
- 抽样看，15 处里大多数是**技术对比**而非修辞：
  - "路径不是单独保存的对象，而是由 dentry 串联出来的结果" ✓ 必要对比
  - "核心思想不是存'整条路径'，而是存'父目录到子名字'的边关系" ✓ 必要对比
  - "事务模型首先要回答的问题不是'如何加锁'，而是'哪些修改必须…'" ✓ 必要对比
- **建议**：🟢 **保留**。技术论述的核心修辞，砍掉反而破坏语义。

---

## 🌐 英文摘要（humanizer 24-pattern 全覆盖）

| # | Pattern | 中文标签 | 命中 | 备注 |
|---|---------|---------|------|------|
| 1 | Significance inflation | 意义膨胀 | **0** | ✅ |
| 2 | Notability emphasis | 名誉强调 | 0 | ✅ |
| 3 | -ing superficial analyses | -ing 堆叠 | 1 | "involved" 是正常用法，非 AI 模式 |
| 4 | Promotional language | 促销式 | 0 | ✅ |
| 5 | Vague attributions | 模糊引用 | 0 | ✅ |
| 6 | Challenges & Future Prospects | 套路章节结构 | 0 | ✅ 摘要无此结构 |
| 7 | AI vocabulary | AI 词汇 | **1** | "Additionally" → 技术副词用法，可保留 |
| 8 | Copula avoidance (serves as) | 避用 is | 0 | ✅ |
| 9 | Negative parallelisms | 否定平行 | 0 | ✅ |
| 10 | Rule of three | 三件套 | 1 | "create, deletion, renaming"→ 真实 POSIX 操作枚举 |
| 11 | Elegant variation | 同义词替换 | 0 | ✅ |
| 12 | False ranges | 伪区间 | 0 | ✅ |
| 13 | **Em dash overuse** | 破折号滥用 | **4** | 🟡 L3 可改；L5/L7/L11 属同位语，保留 |
| 14 | Boldface overuse | 加粗滥用 | 0 | ✅ 摘要无加粗 |
| 15 | Inline header lists | 内联标题列表 | 0 | ✅ |
| 16 | Title case | 标题大写 | 0 | ✅ |
| 17 | Emojis | 表情 | 0 | ✅ |
| 18 | Curly quotes | 花引号 | 0 | ✅ |
| 19 | Chatbot artifacts | 对话残留 | 0 | ✅ |
| 20 | Cutoff disclaimers | 训练截止说明 | 0 | ✅ |
| 21 | Sycophantic tone | 谄媚语气 | 0 | ✅ |
| 22 | Filler phrases | 填充短语 | 0 | ✅ |
| 23 | Excessive hedging | 过度对冲 | 0 | ✅ |
| 24 | Generic positive conclusion | 空泛结尾 | 0 | ✅ |

**英文摘要总命中：5 条 / 24 pattern，全为低风险。**

---

## 💡 最终建议（按工作量排序）

### 🔧 最少改动方案（5 分钟，推荐）

**只改 2 处**：
1. **`abstract-en.tex` L3**：`---account for` → `, account for`
2. **`related_works.tex`**：6 处 "本课题" → "本文"（让"本课题仅限绪论"的分工贯彻到底）

这两处都是**纯机械替换**，不涉及语义调整，风险极低。

### 🛠️ 稍多改动方案（15-20 分钟）

在上面基础上再加：
3. `introduction.tex` L142 "键的编码方式至关重要" → "键的编码方式是核心关键"
4. 抽查 `不是...而是` 的 15 处，把 3-4 处纯修辞的合并（如 "核心思想不是存全路径，而是存边关系" → "核心思想是以边关系表达目录结构，而非以全路径"）

### 🙅 不推荐的改动

按 `AGENT_RULES.md` **"不为反检测牺牲规范性"**，以下改动**会破坏质量**：
- 强行拆短句：你的长句都有必要的状语和补语
- 删"系统性地"/"至关重要"：替换成口语化会降低学术规范
- 打乱"第一/第二/第三"并列：本文 4 个设计目标的并列结构最清晰
- 改中文摘要"本课题"：按章节分工已经自洽

---

## 📌 关于"AI 检测率"

按 humanizer + ppw:de-ai 的严格标准，这篇论文：

1. **零** High Risk 命中
2. **3** 处 Medium Risk，其中 1 处（em dash）可机械修
3. **5** 处 Optional，均属正常学术用法

**这已是 humanizer 衡量下的优秀文本。** 如果学校用的是通用 AIGC 检测工具（如笔灵、知网 AIGC、GPTZero），建议跑一次实测再针对性调整，而不是盲目扩大修改范围。

---

## ❓ 下一步

三个选项：

- **A. 最少改动方案**：我只改 em dash + 6 处"本课题→本文"（5 分钟）
- **B. 最少改动方案 + 补 3 张图**：先改文字，再画 DeltaOp / LSM-tree / rename 锁序图
- **C. 只要图不要改字**：文字保持原样，只补 3 张 TikZ 图

你选哪个？
