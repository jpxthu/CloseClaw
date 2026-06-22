# Memory 模块

## 概述

Memory 模块为 agent 提供长期记忆能力——从会话中挖掘事件和教训，通过实体关联构建跨 session 的记忆网络，在后续对话中适时注入，让 agent 跨 session 保持对用户偏好、历史决策和行为边界的学习。

## 架构

Memory 体系由三个子功能模块 + 一个类型体系 + 两层存储组成：

**两层存储**
- SQLite：entity 索引、event 存储、entity_types 种子数据
- Markdown：人类可读的记忆条目正文（按来源会话组织）

**核心组件**
1. memory-miner：两段式挖掘
   - Miner 1：独立 session，从清洗后的 transcript 挖掘 event + lesson
   - Miner 2：独立 session，读取本 agent 的 entity/type 目录，为 event 分配 entity
2. event 持久化到 SQLite events 表，entity 写入 SQLite entities 表（per-agent 隔离，UNIQUE 键：agent_id + type + normalized_name）
3. dreaming：定期触发，消费 SQL events，entity 级频次评分 + 跨条目 lesson 浓缩
4. MEMORY.md：可执行的行为规则，由 dreaming 写入 `data/memory/MEMORY.md`
5. active-searcher：每条消息触发，从本 agent 的 entity 索引搜索匹配 entity，关联 event 注入消息列表

**触发机制**：两种互补的触发机制：

- **触发 1**：Sub-agent session 结束 hook → memory-miner 即时触发（生命周期明确的 session）
- **触发 2**：Daemon DreamingScheduler 定时触发，按先 dreaming 后 mining 的顺序调度
  1. dreaming：处理上轮 mining 已产出的 event（mined=true 且未 dreaming）
  2. memory-miner：扫描 archived 且 mined=false 的会话，挖掘新 transcript
  适用于 owner 会话等无明确结束点的会话

### 两层存储

| 层 | 存储引擎 | 存什么 | 用途 |
|---|---------|--------|------|
| 结构化索引层 | SQLite | entities 表、events 表、entity_types 表、event_entities 关联表 | 去重、索引、查询、频次统计 |
| 人类可读层 | Markdown 文件 | 记忆条目正文（按来源会话组织） | 人类查阅、prompt 注入 |

SQLite 提供 UNIQUE 约束保证 entity 不重复（per-agent 隔离），提供索引支持高效查询。Markdown 文件保留现有的按会话组织的可读记忆。

### 实体类型

沿用 SAG 的 11 种 entity type：time / location / person / organization / subject / product / metric / action / work / group / tags。详见 [entity-types](entity-types.md)。

### 子功能目录

| 子功能 | 简述 |
|--------|------|
| [memory-miner](memory-miner.md) | 两段式挖掘：Miner 1 产出 event + lesson → Miner 2 分配 entity |
| [dreaming](dreaming.md) | 定期消费 SQL events，entity 级频次评分 + LLM 跨条目 lesson 浓缩，产出 MEMORY.md |
| [active-searcher](active-searcher.md) | 每条消息触发搜索匹配 entity 并注入消息列表 |
| [entity-types](entity-types.md) | 11 种 entity type 定义（沿用 SAG，MIT License） |
| [config](config.md) | 配置项与默认值，global + per-agent 覆盖 |

### 配置与开关

记忆功能通过配置显式开启，不配 = 全部关闭。mining、dreaming、search 各有独立开关。支持 global 配置和 per-agent 覆盖。详见 [config](config.md)。

## 数据流

### 完整路径

1. memory-miner 挖掘（两种触发：sub-agent session 结束 hook 或 Daemon DreamingScheduler 定时扫描）
   - Miner 1（LLM session）：清洗后的 transcript → 提取 event + lesson
   - Miner 2（LLM session）：Miner 1 的 event 列表 + 完整 entity/type 目录（SQL → 固定排序文本）→ 分配 entity
2. 写入 SQLite
   - events 表：event 持久化（标题、摘要、正文、类别、lesson、来源会话、时间戳）
   - entities 表：新 entity 写入（UNIQUE 约束自动去重）
   - event_entities 关联表
   - 标记会话 mined=true
3. dreaming 定时升格（Daemon DreamingScheduler 驱动）
   - 输入：mined=true 且未 dreaming 的会话对应 event（从 SQL 读取）
   - 处理：Light（去重分块）→ REM（entity 级聚类+频次统计）→ Deep（entity 级多维评分）→ LLM 跨条目 lesson 浓缩
   - 输出：MEMORY.md（可执行的行为规则），写入 `data/memory/MEMORY.md`
4. active-searcher 搜索：当前消息 + 上下文 → 提取查询概念 → SQL entities 索引命中匹配 → 关联 event → 浓缩摘要 → 插入消息列表（tool role）

## 模块关系

- **上游**：
  - session 模块：产出会话 transcript，触发 memory-miner；spawn active-searcher 子 session
  - daemon 模块：DreamingScheduler 定时触发 dreaming 和 memory-miner
- **下游**：
  - session 模块：active-searcher 写入 `memory_injection` 槽位，供消息组装时消费

- **无关**：
  - skills 模块：Memory 是基础设施层，不直接与 skills 交互
  - tools 模块：搜索索引是内部基础设施，不对 agent 暴露为 tool
