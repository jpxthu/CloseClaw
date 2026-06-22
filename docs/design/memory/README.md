# Memory 模块

## 概述

Memory 模块为 agent 提供长期记忆能力——从会话中挖掘事件和教训，通过实体关联构建跨 session 的记忆网络，在后续对话中适时注入，让 agent 跨 session 保持对用户偏好、历史决策和行为边界的学习。

## 架构

Memory 体系由三个子功能模块 + 一个类型体系 + 两层存储组成：

```
存储层
  SQLite    —— entity 索引、event 存储、entity_types 种子数据
  Markdown  —— 人类可读的记忆条目正文（按来源会话组织）

────────────────────────────────────────────

memory-miner     —— 两段式挖掘
  Miner 1：独立 session，从清洗后的 transcript 挖掘 event + lesson
  Miner 2：独立 session，读取完整 entity/type 目录，为 event 分配 entity
  │
  ▼
event 持久化到 SQLite events 表
entity 写入 SQLite entities 表（UNIQUE 约束自动去重）
  │
  ▼
dreaming         —— 定期触发，消费 SQL events，entity 级频次评分 + 跨条目 lesson 浓缩
  │
  ▼
MEMORY.md        —— 可执行的行为规则，由 system_prompt 直接注入

active-searcher  —— 每条消息触发，从 SQL entities 索引搜索相关 entity，匹配 event 注入消息列表
```

**触发机制**：两种互补

```
触发 1：Sub-agent session 结束
  session 结束 hook → memory-miner 即时触发
  （生命周期明确的 session）

触发 2：Daemon DreamingScheduler 定时任务
  定时任务按顺序执行两个阶段：
    1. dreaming：处理上轮 mining 已产出的 event（mined=true 且未 dreaming）
    2. memory-miner：扫描 archived 且 mined=false 的会话，挖掘新 transcript
  适用于 owner 会话等无明确结束点的会话
```

### 两层存储

| 层 | 存储引擎 | 存什么 | 用途 |
|---|---------|--------|------|
| 结构化索引层 | SQLite | entities 表、events 表、entity_types 表、event_entities 关联表 | 去重、索引、查询、频次统计 |
| 人类可读层 | Markdown 文件 | 记忆条目正文（按来源会话组织） | 人类查阅、prompt 注入 |

SQLite 提供 UNIQUE 约束保证 entity 不重复，提供索引支持高效查询。Markdown 文件保留现有的按会话组织的可读记忆。

### 实体类型

沿用 SAG 的 11 种 entity type：time / location / person / organization / subject / product / metric / action / work / group / tags。详见 [entity-types](entity-types.md)。

### 子功能目录

| 子功能 | 简述 |
|--------|------|
| [memory-miner](memory-miner.md) | 两段式挖掘：Miner 1 产出 event + lesson → Miner 2 分配 entity |
| [dreaming](dreaming.md) | 定期消费 SQL events，entity 级频次评分 + LLM 跨条目 lesson 浓缩，产出 MEMORY.md |
| [active-searcher](active-searcher.md) | 每条消息触发，从 SQL entities 索引搜索匹配 entity，浓缩相关 event 注入消息列表 |
| [entity-types](entity-types.md) | 11 种 entity type 定义（沿用 SAG） |

### 配置与开关

记忆功能通过配置开启，不配 = 全部关闭。

- **全局配置**：定时做梦时段、挖掘使用的模型、搜索模型
- **agent 覆盖**：每个 agent 可独立覆盖全局配置
- **不开不跑**：未配置的 agent 不触发任何记忆功能

## 数据流

### 完整路径

```
memory-miner 挖掘（两种触发）
    │  触发 1：sub-agent session 结束 hook
    │  触发 2：Daemon DreamingScheduler 扫描 archived 且 mined=false 的会话
    │
    ├─→ Miner 1（LLM session）
    │     输入：清洗后的会话 transcript
    │     处理：挖掘 event + lesson（不涉 entity）
    │     输出：event 列表（标题、摘要、正文、类别、lesson）
    │
    ├─→ Miner 2（LLM session）
    │     输入：Miner 1 的 event 列表 + 完整 entity/type 目录（SQL → 固定排序文本）
    │     处理：为每个 event 分配 entity（从目录选或新建）
    │     输出：event 附 entity 列表
    │
    ▼
写入 SQLite
    · events 表：event 持久化（标题、摘要、正文、类别、lesson、来源会话、时间戳）
    · entities 表：新 entity 写入（UNIQUE 约束自动去重）
    · event_entities 关联表
    完成后：标记会话 mined=true
    │
    ▼
dreaming 定时升格（Daemon DreamingScheduler 驱动）
    │  输入：mined=true 且未 dreaming 的会话对应 event（从 SQL 读取）
    │  处理：Light（去重分块）→ REM（entity 级聚类+频次统计）→ Deep（entity 级多维评分）
    │       → LLM 跨条目 lesson 浓缩
    │  输出：MEMORY.md（可执行的行为规则）
    │
    ▼
MEMORY.md → system_prompt 直接注入
    system_prompt 组装时读取 MEMORY.md → 注入 static system prompt

active-searcher 搜索
    当前消息 + 上下文 → 提取查询概念 → SQL entities 索引命中匹配 → 关联 event
        → 浓缩摘要 → 插入消息列表（tool role）
```

## 模块关系

- **上游**：
  - session 模块：产出会话 transcript，触发 memory-miner；spawn active-searcher 子 session
  - daemon 模块：DreamingScheduler 定时触发 dreaming 和 memory-miner
- **下游**：
  - system_prompt 模块：读取 MEMORY.md 注入 system prompt
  - session 模块：active-searcher 写入 `memory_injection` 槽位，供消息组装时消费

- **无关**：
  - skills 模块：Memory 是基础设施层，不直接与 skills 交互
  - tools 模块：搜索索引是内部基础设施，不对 agent 暴露为 tool
