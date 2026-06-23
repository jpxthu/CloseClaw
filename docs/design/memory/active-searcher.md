# active-searcher

## 概述

每条消息触发独立搜索，从 SQLite entities 表中查找与当前 agent 相关的 entity，通过 entity-event 关联找到对应 event，浓缩为摘要后注入消息列表。

## 架构

active-searcher 以独立 session 形式运行，每条消息异步触发。

处理流程：

1. session 层 spawn active-searcher 子 session
2. 输入：当前消息 + 最近 N 轮对话上下文
3. 概念提取：LLM 从消息中提取查询概念（操作类型、涉及对象、场景特征）
4. 搜索：查询概念 → SQL entities 表匹配当前会话所属 agent 的 entity（精确/模糊匹配），type weight 加权排序
5. event 关联：命中 entity → SQL event_entities 关联表 → 获取相关 event（命中数 ≥ `search.min_entity_hits`，取前 `search.top_k_events` 条）
6. 去重过滤：排除本 session 已注入的 event ID，将本轮命中 event ID 加入已注入集合
7. 浓缩：相关 event → 文本摘要（≤ `search.max_summary_chars`）
8. 输出：浓缩摘要写入 session `memory_injection` 槽位（tool role）

**搜索模型**：默认继承全局模型。为降低延迟和成本，建议独立配置低价轻量模型。超时可配，超时则放弃本轮。

### 注入时机

active-searcher 将浓缩摘要写入 session 的 `memory_injection` 槽位，由 session 在下次消息组装时消费。槽位结构定义见 [session-injection](../../session/session-injection.md)。

**位置模式**：
- 用户消息触发 → 写入 `AfterCurrent`（摘要插入用户消息之后）
- agent 消息触发 → 写入 `BeforeNext`（摘要插入下一轮用户消息之前）

**角色排除**：memory-miner 自身、dreaming 浓缩 session 不触发 active-searcher。

## 数据流

1. 当前消息 + 最近 N 轮对话上下文 → LLM 提取查询概念
2. 查询概念 → SQL entities 表匹配本 agent 的 entity → 命中 entity
3. 命中 entity → event_entities 关联表 → 获取相关 event
4. 去重过滤 → 浓缩为文本摘要
5. 写入 session `memory_injection` 槽位（tool role，含位置模式 AfterCurrent/BeforeNext），供下次消息组装消费
6. 超时则放弃本轮

## 模块关系

- **上游**：
  - session 模块：spawn active-searcher 子 session
  - SQLite entities / event_entities / events 表：搜索数据源
- **下游**：
  - session 模块：写入 `memory_injection` 槽位，由 session 在下次消息组装时消费
- **无关**：
  - system_prompt 模块：active-searcher 不修改 system prompt，只插入消息列表
  - dreaming 模块：两者无直接交互，各自独立消费 SQLite
