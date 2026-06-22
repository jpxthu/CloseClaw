# memory-miner

## 概述

会话 transcript 产生后，通过两段独立的 LLM session 进行记忆挖掘。Miner 1 专注于从 transcript 中提取事件和教训，Miner 2 读取完整实体目录为事件分配实体。两段分离让各自专注于一个任务，避免注意力分散。

触发方式有两种：即时 hook（sub-agent session 结束）和定时任务（archived 会话扫描）。

## 架构

memory-miner 由两个独立的 LLM session，串行执行：

1. **Miner 1（LLM session）**：挖掘 event + lesson，不接触 entity
   - 输出：event 列表（标题、摘要、正文、类别、lesson）
2. **Miner 2（LLM session）**：为 event 分配 entity
   - 输入：Miner 1 的 event 列表 + 完整 entity/type 目录
   - 输出：event 附 entity 列表（新建的 entity 立即写入 SQL）
3. 写入 SQLite：events 表 + entities 表（UNIQUE 自动去重）+ event_entities 关联表
4. 写入 Markdown：人类可读记忆条目（按来源会话组织）
5. 标记会话 mined=true

### 两种触发机制

- **触发 1**：Sub-agent session 结束 hook → 即时触发 mining。适用于生命周期明确的 session（子 agent 完成/失败后）
- **触发 2**：Daemon DreamingScheduler 定时扫描 archived 且 mined=false 的会话 → 触发 mining。适用于 owner 会话等无明确结束点的会话。DreamingScheduler 的整体调度顺序（先 dreaming 后 mining）详见 README

### Miner 1：事件与教训挖掘

Miner 1 以独立 LLM session 运行，使用专用挖掘 prompt。不与用户交互，不阻塞主流程。

**输入**：
- 清洗后的完整会话 transcript（按可配置规则清洗格式）
- 已有 event 列表（同 agent 的近期 event，避免重复）
- 已有 MEMORY.md（避免产出已有规则已覆盖的教训）

**处理**：
- 从 transcript 中提取值得长期保留的事件
- 每个事件包含：标题、摘要、正文、类别（preference / decision / lesson / fact）
- 对涉及错误或纠正的事件，提炼一条教训（lesson）——简洁的行为指导，直接可执行，不引用具体 agent 名和消息编号

**输出**：event 列表。每个 event 带有 lesson（可为空，纯偏好/决策类事件不需要教训）。

### Miner 2：实体分配

Miner 2 以独立 LLM session 运行。在 Miner 1 完成后触发。

**输入**：
- Miner 1 产出的 event 列表（每个 event 的标题、摘要、正文）
- 完整 entity 目录：从 SQLite entities 表读取所有已有 entity + entity_types 表读取所有类型定义，合并为固定排序的文本列表。排序规则：先按 `type` 字母序，再按 `normalized_name` 字母序。固定排序旨在最大化 KV Cache 命中率

**处理**：
- 为每个 event 分配 entity——从已有 entity 目录中选择相关者，或在目录中没有匹配 entity 时创建新 entity
- 每个 entity 指定 type（从 11 种 entity type 中选择）、name、description

**输出**：event 附 entity 列表。新 entity 立即写入 SQLite entities 表（UNIQUE 约束自动去重：同 source + 同 type + 同 normalized_name 的 entity 合并）。

### 挖掘原则

- 捕捉三类高信号信息：agent 犯的错、owner 的纠正和不满、owner 的明确决策
- Miner 1 全神贯注于"这个 session 发生了什么、有什么教训"，不接触 entity
- Miner 2 全神贯注于 entity 分配，能看见完整的 entity/type 目录确保命名一致性
- 与已有 event 去重：相同语义信息不重复记录

## 数据流

1. 输入：清洗后的会话 transcript + 已有 event 列表 + 已有 MEMORY.md
2. Miner 1（LLM session）：提取 event → 提炼 lesson
3. Miner 2（LLM session）：读取完整 entity/type 目录（SQL → 固定排序文本）→ 为 event 分配 entity → 新建 entity 立即写入 SQL
4. 写入 SQLite：events 表（event 持久化）+ entities 表（UNIQUE 自动去重）+ event_entities 表（关联）
5. 写入 Markdown（人类可读记忆条目，按来源会话组织）
6. 标记会话 mined=true

## 模块关系

- **上游**：
  - session 模块：产出会话 transcript，触发 mining
  - daemon 模块：DreamingScheduler 定时触发 mining
  - entity 目录（SQLite）：Miner 2 读取已有 entity 和 entity_type
- **下游**：
  - SQLite events 表：event 持久化写入
  - SQLite entities 表：新 entity 写入
  - dreaming 模块：消费 event 进行升格
  - active-searcher 模块：搜索时命中 entity → 关联 event

- **无关**：
  - system_prompt 模块：memory-miner 不直接写入 system prompt
