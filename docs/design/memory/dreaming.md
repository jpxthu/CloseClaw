# dreaming

## 概述

定期消费 SQLite 中已 mining 但未 dreaming 的 event，通过实体级频次评分和跨条目教训浓缩，将高价值行为规则写入 `data/memory/MEMORY.md`。

## 架构

dreaming 由三阶段 + 教训浓缩组成。Light / REM / Deep 为程序化处理（SQL 查询 + 统计），教训浓缩为 LLM 驱动，Dream Diary 为可选叙事摘要。

处理顺序：

1. **Light** — 程序化处理：增量读取（仅新 event）、去重（与 MEMORY.md 已有规则做语义去重）、按 entity type 分块
2. **REM** — 程序化处理：entity 级聚类（通过 event_entities 关联表统计 entity 频次），跨 session 共享相同 entity 的 event 被自动关联
3. **Deep** — 程序化处理：entity 级多维加权评分，对每个 entity 的关联 event 组打分，三道门槛过滤
4. **教训浓缩** — LLM 处理：对通过 Deep 门槛的 entity 组，将其关联 event 的 lesson 浓缩为单条行为规则
5. 输出：MEMORY.md（可执行的行为规则），写入 `data/memory/MEMORY.md`
6. **Dream Diary**（可选）— LLM 叙事摘要，写入独立日记文件，不参与升格链路

### Light 阶段

- 增量读取：仅处理上次 dreaming 后新产生的 event（从 SQL events 表按 `created_at` 过滤）
- 去重：与 MEMORY.md 已有规则做语义去重，避免产出重复规则
- 分块：按 entity type 分组，为 REM 阶段做准备

### REM 阶段

- entity 级聚类：通过 SQL 查询 `event_entities` 关联表，将共享相同 entity 的 event 归为同一 entity 组
- entity 频次统计：按 entity 级精确计数——同一 entity 在多个 session 中被关联，频次相应提升
- entity type 加权：根据 [entity-types](entity-types.md) 中定义的 type weight，对高频 entity 按 type 加权调整
- 跨 agent 模式：同一 entity 被不同 agent 的 session 关联时标记

### Deep 阶段

在 entity 级别进行多维评分。每个 entity 组作为一个候选单元：

| 维度 | 含义 |
|------|------|
| 频次 | 同一 entity 在多个 session 中被关联。entity 级精确计数 |
| 时效 | 最近关联的 event 权重更高（时间衰减） |
| 明确性 | owner 明确表述 vs agent 推断。owner 明确表述显著加分 |
| 类型权重 | 按 entity type 的 weight 调整，详见 [entity-types](entity-types.md) |
| 跨 agent | 多个 agent 的 event 共享此 entity |
| 负面信号 | 可能被后续 event 推翻 |

**三道门槛**（在 entity 组级别应用）：
- 绝对阈值：总评分低于下限的 entity 组直接丢弃
- 相对阈值：同类 entity 组间评分相对过低的丢弃
- 容量上限：MEMORY.md 条目数达到上限时，低分 entity 组溢出

### 教训浓缩

通过 Deep 评分的 entity 组，进入 LLM 驱动的教训浓缩：

- 输入：entity 组内所有关联 event 的 `lesson` 字段 + entity 信息 + 频次统计
- 处理：LLM 将多条相关教训浓缩为一条简洁的行为规则
- 输出要求：规则直接可执行（与 Miner 1 的 lesson 格式约束一致）

浓缩后的行为规则写入 MEMORY.md。原始 event 保留在 SQLite 中供 active-searcher 搜索使用。

### Dream Diary

Dream Diary 是 dreaming 完成后触发的可选 LLM 叙事摘要，将本轮升格结果以自然语言叙述归档。

- 触发条件：dreaming 完成后，由配置开关控制（开启 dreaming 后默认打开）
- 内容：本轮新增/更新的 MEMORY.md 条目摘要，以连贯散文形式呈现
- 产出：写入独立的日记文件，不参与记忆升格链路，仅供用户查阅

### 防污染

- dreaming 自身的产出不参与后续 dreaming 的 ingestion（防自循环）
- 写入 MEMORY.md 前确认源 event 仍存在且未被修改

### 触发

由 Daemon 层的 DreamingScheduler 定时任务驱动，在配置的做梦时段内执行。DreamingScheduler 的整体调度顺序（先 dreaming 后 mining）详见 README。

## 数据流

1. 输入：SQLite events 表（增量，mined=true 且未 dreaming，含 entity 关联）
2. Light：增量读取 → 去重 → 按 entity type 分块
3. REM：entity 级聚类（SQL JOIN event_entities）→ entity 频次统计（精确计数）→ 跨 agent 模式标记
4. Deep：entity 级多维评分 → type weight 加权 → 三道门槛过滤
5. 教训浓缩（LLM）：跨条目 lesson 浓缩 → 输出行为规则
6. 输出：MEMORY.md（可执行的行为规则）
7. Dream Diary（可选）：叙事摘要，写入日记文件

## 模块关系

- **上游**：
  - memory-miner：产出 event 并写入 SQLite，是 dreaming 的唯一数据来源
  - SQLite events / entities / event_entities 表：dreaming 的数据源
  - daemon 模块：DreamingScheduler 定时任务触发

- **下游**：
  - SQLite：dreaming 自身的处理结果不写回 SQL（只写 MEMORY.md）

- **无关**：
  - active-searcher 模块：两者无直接交互，各自独立消费 SQLite
  - session 模块：dreaming 是后台任务，不与 session 交互
