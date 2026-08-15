# memory-miner

## 概述

会话 transcript 产生后，通过两段独立的 LLM session 进行记忆挖掘。Miner 1 和 Miner 2 各负责一个阶段，串行执行。

触发方式有两种：即时 hook（sub-agent session 结束）和定时任务（DreamingScheduler 扫描 archived 且 mined=false 的 session）。

## 架构

memory-miner 由两个独立的 LLM session，串行执行：

1. 输入：清洗后的 session transcript + 已有 event 列表（同 agent 近期 event）+ 已有 MEMORY.md
2. **Miner 1（LLM session）**：挖掘 event + lesson，不接触 entity
   - 输出：event 列表（标题、摘要、正文、类别；error/anger 附 lesson 字段）
3. **Miner 2（LLM session）**：为 event 分配 entity
   - 输入：Miner 1 的 event 列表 + 本 agent 的 entity/type 目录
   - 输出：event 附 entity 列表（含新建 entity，在步骤 4 统一写入）
4. 写入 SQLite：events 表 + entities 表 + event_entities 关联表
   - events 表：event 持久化（标题、摘要、正文、类别、lesson、来源 session、时间戳），并附带遗忘截止时间 expires_at（创建时 = 当前时间 + `forgetting.initial_ttl_days`）
   - entities 表：以 agent_id 作为隔离键，UNIQUE(agent_id, type, normalized_name) 保证 per-agent 隔离与同名去重
   - event_entities 关联表：event 与 entity 的多对多关系
5. 标记 session mined=true（写入 sessions 表的 mined、mined_at 字段）
6. 遗忘清理：周期性扫描 expires_at 已过期的 event 并删除；删除后不再被任何 event_entities 关联的 entity（0 引用）一并删除

### Miner 1：事件与教训挖掘

Miner 1 以独立 LLM session 运行，使用专用挖掘 prompt。不与用户交互，不阻塞主流程。

**输入**：
- 清洗后的完整会话 transcript：从 session JSONL 提取 user 和 assistant 文本内容，去除 thinking 块、tool-call XML、内部上下文标记、MEDIA/NO_REPLY 行、连续空行，仅保留消息类型和消息正文，以 markdown 格式输出（格式由 `mining.transcript_clean_rules.format` 控制，默认 `md`；未达 `min_turns` / `min_owner_msgs` 阈值的会话跳过 mining）
- 已有 event 列表（同 agent 的近期 event，从 SQLite events 表读取，避免重复）
- 已有 MEMORY.md（从 `storage.memory_md_path` 读取，避免产出已有规则已覆盖的教训）

**处理**：
- 从 transcript 中提取值得长期保留的事件，优先挖掘以下高信号信息：
  - **error**：agent 的明确错误（判断失误、操作不当、误解需求）
  - **anger**：owner 的不满和纠正（"你是不是搞错了"、"你动动脑子"等）
  - **decision**：owner 的明确产品决策和设计选择
  - **insight**：agent 经过反复尝试发现的简单规律，值得沉淀为经验的发现
- 每个事件包含：标题、摘要、正文、类别（error / anger / decision / insight），error / anger 附带 lesson 字段
- **去重**：对比已有 event 列表（同 agent 近期 event，时间窗口由 `mining.dedup_window_days` 配置）做语义去重；对比已有 MEMORY.md 跳过已被长期记忆覆盖的规则。去重命中已有 event（同一语义信息被再次识别）时不再新建，改为大幅延长该 event 的 expires_at（延长 `forgetting.reidentify_extension_days` 天），lesson 保持原值
- **lesson 提炼**：从 error 和 anger 事件中提炼 lesson（简洁的行为指导，直接可执行，不引用具体 agent 名和消息编号）。decision / insight 事件可不附带 lesson
- **不记录**：纯偏好陈述（"我喜欢 X"）、背景事实（已在 MEMORY.md/USER.md 中记录的）、单次临时讨论、技术实现细节

**输出**：event 列表（最多 `mining.max_events_per_session` 条，取最高信号事件），每个 event 含类别（error / anger / decision / insight），error 和 anger 附带 lesson 字段，decision / insight 不附 lesson。

### Miner 2：实体分配

Miner 2 以独立 LLM session 运行。在 Miner 1 完成后触发。

**输入**：
- Miner 1 产出的 event 列表（每个 event 的标题、摘要、正文）
- 本 agent 的 entity/type 目录：合并 SQLite entities 表（本 agent 名下所有 entity）+ entity_types 表（11 种类型定义），按 type → normalized_name 固定排序生成文本列表。entity 作用域为 per-agent，entity 名称限制在 10 个单词以内

**处理**：
- 为每个 event 分配 entity——优先匹配已有实体，无匹配时创建新实体。匹配判定分两级：
  1. **名字优先**：将候选 entity 的 name 归一化为 normalized_name，若本 agent 下已存在同 type + 同 normalized_name 的实体，直接复用，不新建
  2. **向量兜底**：名字未命中时，计算候选 entity 与已有同 type 实体的向量相似度并取最高分——若 ≥ 该 type 的 similarity_threshold 则复用最高分实体，否则新建
- 新建 entity 指定 type（从 11 种 entity type 中选择）、name、description；name 归一化后不超过 10 个单词
- 复用已有实体时保留已有实体的描述，不覆盖

**输出**：event 附 entity 列表（含新建 entity，在架构步骤 4 统一写入 SQLite）。UNIQUE(agent_id, type, normalized_name) 约束作为兜底去重，跨 agent 同名 entity 独立存储。

### 挖掘原则

- **error + anger 优先**：agent 犯的错和 owner 的纠正/不满是最高信号，优先于 decision 类事件
- **lesson 从 error/anger 中提炼**：不单独记录偏好和事实——它们信号低且容易与已有文档重复
- Miner 1 只读 SQLite events 表做去重，不读写 entities/entity_types 表
- Miner 2 全神贯注于 entity 分配，能看见完整的 entity/type 目录确保命名一致性
- 与已有 event 去重：相同语义信息不重复记录

## 数据流

1. 输入：清洗后的会话 transcript + 已有 event 列表 + 已有 MEMORY.md
2. Miner 1（LLM session）：提取 event（类别 error / anger / decision / insight）→ 提炼 lesson（error / anger）；去重命中已有 event 时大幅延长其 expires_at
3. Miner 2（LLM session）：读取完整 entity/type 目录（SQL → 固定排序文本）→ 为 event 分配 entity（名字优先 + 向量兜底匹配，无匹配则新建）
4. 写入 SQLite：events 表（event 持久化，含 expires_at）+ entities 表（agent_id 隔离 + UNIQUE 去重）+ event_entities 表（关联）
5. 标记会话 mined=true（写入 sessions 表的 mined、mined_at 字段）
6. 遗忘清理：周期性删除 expires_at 过期的 event，级联删除 0 引用 entity

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
