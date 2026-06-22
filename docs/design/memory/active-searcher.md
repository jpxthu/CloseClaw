# active-searcher

## 概述

每条消息触发独立搜索，从 SQLite entities 索引中查找相关 entity，通过 entity-event 关联找到对应 event，浓缩为摘要后注入消息列表。确保记忆召回覆盖所有消息，不依赖主 agent 判断。

## 架构

active-searcher 以独立 session 形式运行，每条消息异步触发，使用独立低开销模型。

处理流程：

1. session 层 spawn active-searcher 子 session（独立低开销模型）
2. 输入：当前消息 + 最近 N 轮对话上下文
3. 概念提取：LLM 从消息中提取查询概念
   - 操作类型：当前 agent 正在执行或即将执行的操作
   - 涉及对象：操作涉及的对象类型
   - 场景特征：当前语境的场景描述
4. 搜索
   - 主路径：查询概念 → SQL entities 表匹配 entity name/normalized_name
   - 补量路径：消息 embedding × entity embedding 向量检索，补充不足条目
5. event 关联：命中 entity → SQL event_entities 关联表 → 获取相关 event
6. 过滤：排除 per-session "已注入 event ID 集合"中已有 event
7. 浓缩：相关 event → 文本摘要（≤ 可配置字符上限）
8. 输出：浓缩摘要排队注入消息列表

**搜索模型**：独立配置，追求便宜 + 快，不等同于主对话模型。超时时间可配，超时则放弃本轮。

### Entity 索引查询

active-searcher 从消息中提取查询概念后，通过 SQL 精确查询 entities 表。

**查询概念提取**：LLM 从当前消息和最近 N 轮上下文中提取查询概念。概念包括：
- 操作类型：当前 agent 正在执行或即将执行的操作
- 涉及对象：操作涉及的对象类型
- 场景特征：当前语境的场景描述

**SQL entity 匹配**：将查询概念与 entities 表做匹配：
- 精确匹配 `normalized_name`
- 模糊匹配（LIKE 或子串匹配）
- 向量匹配（embedding 余弦距离）

**entity type 加权**：匹配到的 entity 按其 type weight 加权排序（subject 1.5 > action 1.3 > person 1.2 > ... > tags 0.5）。

### Event 关联

命中 entity 后，通过 SQL `event_entities` 关联表找到对应 event。一个 entity 可能关联多个 event（跨 session），一个 event 也可能关联多个 entity。

**排序**：entity 命中数多的 event 排在前——一个 event 关联多个命中 entity 说明与当前消息高度相关。

### 注入时机

active-searcher 将浓缩摘要写入 session 的 `memory_injection` 槽位，由 session 在下次消息组装时消费。

**去重**：每个 session 维护一个"已注入 event ID 集合"。searcher 在输出浓缩摘要时排除该集合中已有的 event，并将本轮命中的 event ID 加入集合。

**更新**：每次注入时，如果有新的匹配结果，更新 `memory_injection` 槽位。如果无匹配，槽位保持为空。

### 特殊角色处理

某些 session 角色（如 memory-miner 自身、dreaming 浓缩 session）不触发 active-searcher。

## 数据流

1. 当前消息 + 最近 N 轮对话上下文 → LLM 提取查询概念
2. 查询概念 → 主路径（SQL entities 表精确/模糊/向量匹配，type weight 加权排序）或补量路径（向量检索）
3. 命中 entity → event_entities 关联表 → 获取相关 event
4. 去重过滤（排除已注入 event ID）
5. 浓缩为文本摘要（≤ 字符上限）
6. 写入 session `memory_injection` 槽位（tool role），供下次消息组装消费
7. 超时则放弃本轮

特殊角色跳过：memory-miner 自身和 dreaming 浓缩 session 不触发 active-searcher。

## 模块关系

- **上游**：
  - session 模块：spawn active-searcher 子 session
  - SQLite entities / event_entities / events 表：搜索数据源
- **下游**：
  - session 模块：写入 `memory_injection` 槽位，由 session 在下次消息组装时消费

- **无关**：
  - system_prompt 模块：active-searcher 不修改 system prompt，只插入消息列表
  - dreaming 模块：两者无直接交互，各自独立消费 SQLite
