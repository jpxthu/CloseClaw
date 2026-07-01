# Memory 模块

## 概述

Memory 模块为 agent 提供长期记忆能力——从 session 中挖掘 event 和 lesson，通过 entity 关联构建跨 session 的记忆网络，在后续对话中适时注入，让 agent 跨 session 保持对用户偏好、历史决策和行为边界的学习。

## 架构

Memory 体系由三个子功能模块 + 一个类型体系组成，以 SQLite 为单一存储引擎。

**核心组件**
1. memory-miner：两段式挖掘
   - Miner 1：独立 session，从清洗后的 transcript 挖掘 event + lesson
   - Miner 2：独立 session，读取本 agent 的 entity/type 目录，为 event 分配 entity
2. event 持久化到 SQLite events 表，entity 写入 SQLite entities 表（per-agent 隔离，UNIQUE 键：agent_id + type + normalized_name），通过 event_entities 关联表维护多对多关系
3. dreaming：定期触发，消费 SQL events，entity 级频次评分 + 跨条目 lesson 浓缩
4. MEMORY.md：可执行的行为规则，dreaming 升格后写入（路径通过 `storage.memory_md_path` 配置）
5. Dream Diary（可选）：dreaming 的叙事摘要，写入独立日记文件
6. active-searcher：每条消息触发，从本 agent 的 entity 索引搜索匹配 entity，关联 event 注入消息列表

**触发机制**：两种互补的触发机制：

- **触发 1**：Sub-agent session 结束 hook → memory-miner 即时触发（生命周期明确的 session）
- **触发 2**：Daemon DreamingScheduler 定时触发，先执行 dreaming 再执行 mining
  1. dreaming：处理已 mining 的 event（增量读取）
  2. memory-miner：扫描 archived 且 mined=false 的 session，挖掘新 transcript
  dreaming 先于 mining 执行：若尚无已 mining 的 event（初始状态），dreaming 本次为空操作，mining 产出 event 后下一轮 dreaming 即可消费。适用于 owner session 等无明确结束点的 session

### 存储

SQLite 存储结构化索引数据（entities 表、events 表、entity_types 表、event_entities 关联表），提供 UNIQUE 约束保证 entity 不重复（per-agent 隔离），提供索引支持高效查询。

dreaming 产出 MEMORY.md（可执行的行为规则）和 Dream Diary（可选叙事摘要），不作为独立的记忆存储层——原始 event 和 entity 全部在 SQLite 中。

### 实体类型

沿用 [SAG](https://github.com/Zleap-AI/SAG)（MIT License）的 11 种 entity type：time / location / person / organization / subject / product / metric / action / work / group / tags。详见 [entity-types](entity-types.md)。

### 子功能目录

| 子功能 | 简述 |
|--------|------|
| [memory-miner](memory-miner.md) | 两段式挖掘：Miner 1 产出 event + lesson → Miner 2 分配 entity |
| [dreaming](dreaming.md) | 定期消费 SQL events，entity 级频次评分 + LLM 跨条目 lesson 浓缩，产出 MEMORY.md |
| [active-searcher](active-searcher.md) | 每条消息触发搜索匹配 entity 并注入消息列表 |
| [entity-types](entity-types.md) | 11 种 entity type 定义（沿用 SAG，MIT License） |
| [config](config.md) | 配置项与默认值，global + per-agent 覆盖 |

### 配置与开关

记忆功能通过配置显式开启，不配 = 全部关闭。mining、dreaming、search 各有独立开关。支持 global 配置和 per-agent 覆盖。配置支持热重载（修改后无需重启）。详见 [config](config.md)。

## 数据流

### 完整路径

1. memory-miner 挖掘（两种触发：sub-agent session 结束 hook 或 Daemon DreamingScheduler 定时扫描）
   - Miner 1（LLM session）：输入（transcript + 已有 event 列表 + 已有 MEMORY.md）→ 提取 event + lesson（类别 error/anger/decision）
   - Miner 2（LLM session）：Miner 1 的 event 列表 + 完整 entity/type 目录（SQL → 固定排序文本）→ 分配 entity
2. 写入 SQLite
   - events 表：event 持久化（标题、摘要、正文、类别、lesson、来源 session、时间戳）
   - entities 表：新 entity 写入（UNIQUE 约束自动去重）
   - event_entities 关联表
   - 标记 session mined=true（写入 session 模块的 sessions 表 mined、mined_at 字段）
3. dreaming 定时升格（Daemon DreamingScheduler 驱动）
   - 输入：mined=true 的 session 对应 event（从 SQL 增量读取）
   - 处理：Light（增量读取+去重+分块）→ REM（entity 级聚类+频次统计+跨 agent 标记）→ Deep（entity 级多维评分+type weight 加权+三道门槛过滤）→ LLM 跨条目 lesson 浓缩
   - 输出：MEMORY.md（可执行的行为规则，路径通过 `storage.memory_md_path` 配置）+ Dream Diary（可选叙事摘要，路径通过 `dreaming.diary.path` 配置）
   - 防污染：dreaming 产出不写回 SQLite，后续 dreaming 增量读取时自然不会包含自身产出
4. active-searcher 搜索：当前消息 + 上下文 → 提取查询概念 → SQL entities 索引命中匹配 → 关联 event → 去重过滤（per-session 已注入 ID 集合）→ 浓缩摘要 → 写入 `memory_injection` 槽位（tool role）
   - 特殊角色（memory-miner 自身、dreaming 浓缩 session）不触发 active-searcher

## 模块关系

- **上游**：
  - session 模块：产出会话 transcript，触发 memory-miner；spawn active-searcher 子 session；消费 `memory_injection` 槽位
  - daemon 模块：DreamingScheduler 定时触发 dreaming 和 memory-miner
  - SQLite（entity/type 目录）：Miner 2 分配 entity 时读取
- **下游**：
  - session 模块：active-searcher 写入 `memory_injection` 槽位，供消息组装时消费
  - System Prompt 静态层：MEMORY.md 通过 [MemoryFragmentProvider](../system_prompt/fragment-provider.md)（实现 [PromptFragmentProvider](../system_prompt/fragment-provider.md) trait）注入 system prompt 静态层 MemorySection
  - SQLite events/entities/event_entities 表：写入

- **无关**：
  - skills 模块：Memory 是基础设施层，不直接与 skills 交互
  - tools 模块：搜索索引是内部基础设施，不对 agent 暴露为 tool
  - system_prompt 模块：memory-miner 不直接写入 system prompt
