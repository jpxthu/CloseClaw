# 配置

## 概述

Memory 模块的配置控制挖掘、做梦和搜索三个子系统的行为。所有功能必须显式开启，未配置则不激活。支持全局配置和 per-agent 覆盖。

配置集成到 CloseClaw 的 ConfigProvider 体系。

## 架构

### 配置层级

Memory 配置作为 CloseClaw 配置体系的一部分。

### 层叠覆盖

```
CloseClaw 全局配置
  │
  ▼
per-agent 覆盖（agents/<agent_id>/config.json 中的 memory 段）
  │  字段级合并：per-agent 声明的字段覆盖全局配置，未声明的继承
  ▼
最终生效配置
```

### 启用规则

mining、dreaming、search 各有独立开关，均为 `true` 时对应功能才激活。

### 存储

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `storage.db_path` | string | `memory/memory.db` | SQLite 数据库文件路径 |
| `storage.markdown_path` | string | `memory/entries/` | Markdown 记忆条目目录 |
| `storage.memory_md_path` | string | `memory/MEMORY.md` | dreaming 产出的行为规则文件路径 |

路径均相对于 CloseClaw 数据根目录（`~/.closeclaw/`，见 [platform](../platform/README.md)）。

### 挖掘（memory-miner）

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `mining.enabled` | bool | `false` | 必须显式设为 `true` 才启用挖掘 |
| `mining.model` | string | 继承全局默认模型 | Miner 1 和 Miner 2 使用的模型 |
| `mining.max_events_per_session` | int | `10` | 每次挖掘最多产出的 event 数 |
| `mining.transcript_clean_rules` | object | `{}` | transcript 清洗规则配置 |

**transcript_clean_rules**：

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `min_turns` | int | `5` | 最少对话轮数 |
| `min_owner_msgs` | int | `5` | 最少 owner 消息数 |
| `format` | string | `md` | transcript 输出格式 |

### 做梦（dreaming）

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `dreaming.enabled` | bool | `false` | 必须显式设为 `true` 才启用做梦 |
| `dreaming.schedule` | string | `0 3 * * *` | cron 表达式，由 Daemon DreamingScheduler 消费 |
| `dreaming.model` | string | 继承全局默认模型 | 教训浓缩步骤使用的模型 |

**评分权重**：

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `dreaming.scoring.frequency_weight` | float | `1.0` | entity 跨 session 频次权重 |
| `dreaming.scoring.recency_weight` | float | `0.5` | 时效衰减权重 |
| `dreaming.scoring.explicitness_weight` | float | `1.5` | owner 明确表述加分权重 |
| `dreaming.scoring.cross_agent_weight` | float | `1.3` | 跨 agent 共享 entity 加分权重 |
| `dreaming.scoring.negative_signal_weight` | float | `-0.5` | 负面信号减分权重 |

**门槛与容量**：

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `dreaming.threshold.absolute` | float | `2.0` | 绝对阈值 |
| `dreaming.threshold.relative` | float | `0.3` | 相对阈值 |
| `dreaming.capacity.max_rules` | int | `20` | MEMORY.md 最大规则数 |

**Dream Diary**：

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `dreaming.diary.enabled` | bool | `true` | dreaming 开启后默认打开 |
| `dreaming.diary.path` | string | `memory/diary/` | 日记文件目录 |

### 搜索（active-searcher）

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `search.enabled` | bool | `false` | 必须显式设为 `true` 才启用搜索 |
| `search.model` | string | 独立低开销模型 | 概念提取使用的模型，追求便宜 + 快 |
| `search.context_turns` | int | `5` | 提取查询概念时携带的最近对话轮数 |
| `search.timeout_ms` | int | `3000` | 搜索超时（毫秒） |
| `search.max_summary_chars` | int | `500` | 浓缩摘要最大字符数 |
| `search.min_entity_hits` | int | `1` | 最少 entity 命中数 |
| `search.top_k_events` | int | `3` | 最终注入的 event 摘要数上限 |

### per-agent 覆盖

agent 配置文件中声明 memory 段，字段级合并覆盖全局配置。示例：

- 声明 `memory.dreaming.threshold.absolute` 为 `3.0`，覆盖全局默认值 `2.0`
- 声明 `memory.search.context_turns` 为 `8`，覆盖全局默认值 `5`

未声明的字段继承全局配置。无需覆盖的场景不声明 memory 段即可。

## 数据流

```
CloseClaw 全局配置 → ConfigProvider 加载 → per-agent 字段级合并 → 传递给 memory 各子模块
```

配置热重载：修改配置后无需重启，下次触发时自动读取最新配置。

## 模块关系

- **上游**：config 模块（通过 CloseClaw ConfigProvider 体系加载和合并配置）
- **下游**：memory-miner、dreaming、active-searcher（各子模块读取自己命名空间下的配置）
- **无关**：entity-types（种子数据，不通过配置控制）
