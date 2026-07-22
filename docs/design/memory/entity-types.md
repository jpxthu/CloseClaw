# 实体类型

## 概述

entity type 定义了记忆系统中 entity 的分类体系。Miner 2 在生成 entity 时为每个 entity 指定一种类型，类型决定了该 entity 在 active-searcher 和 dreaming 评分时的权重。

类型体系沿用 [SAG](https://github.com/Zleap-AI/SAG)（MIT License）的 11 种分类。

## 架构

11 种实体类型定义：

| type | 名称 | 说明 | 权重 | 相似度阈值 |
|------|------|------|------|-----------|
| `time` | 时间 | 时间点、时期、日期、年份等时间表达 | 1.0 | 0.90 |
| `location` | 地点 | 国家、城市、地区、地点等物理位置 | 1.0 | 0.75 |
| `person` | 人物 | 人物和具名个体（含 agent 角色、用户身份） | 1.2 | 0.80 |
| `organization` | 组织 | 公司、机构、团队等组织 | 1.1 | 0.80 |
| `subject` | 主题 | 主要主题、概念和课题 | 1.5 | 0.78 |
| `product` | 产品 | 产品、服务、项目和命名交付物 | 1.1 | 0.80 |
| `metric` | 指标 | 数字、指标、度量、金额和统计数据 | 1.2 | 0.85 |
| `action` | 动作 | 重要动作、变更、决策和操作 | 1.3 | 0.78 |
| `work` | 作品 | 创作物、文档、论文、书籍、报告 | 1.0 | 0.80 |
| `group` | 群体 | 群体、社区、受众和人口 | 1.0 | 0.78 |
| `tags` | 标签 | 兜底标签，当无特定类型匹配时使用 | 0.5 | 0.70 |

**权重**：在 dreaming 和 active-searcher 的评分和排序中使用。`subject`（1.5）和 `action`（1.3）是最高价值类型。

**相似度阈值**：实体类型的向量相似度余弦距离门槛。在 Miner 2 entity 分配和 active-searcher 搜索时，entity 向量嵌入与查询/目标的余弦距离低于此门槛则不匹配。值越高匹配越严格。

### 约束

Miner 2 确保 entity 名称不超过 10 个单词。

### 存储

entity_types 存储在 SQLite 中。建表时写入 11 种类型的种子数据。每条类型记录包含：id、type、name、description、weight、similarity_threshold、is_default、is_active。

`is_default=true` 的类型为同 type 下的默认定义。当同 type 存在多条定义时（如 per-agent 覆盖），is_default=true 的为标准定义，Miner 2 分配 entity 和 active-searcher 搜索时优先匹配默认定义。`is_active=false` 的类型不参与类型解析。种子数据中所有类型 is_active 均为 true。

## 数据流

1. 系统初始化 / 首次启动：SQLite 建表 → 写入 11 种 entity_type 种子数据
2. 类型目录生成：合并 SQLite `entities` 表和 `entity_types` 表的数据，按 `type` → `normalized_name` 字母序排列，生成固定格式的文本列表，作为 Miner 2 prompt 的输入。首次启动时 entities 表为空，类型目录仅包含类型定义列表，不影响 Miner 2 正常分配。
3. Miner 2 entity 分配：
   - 输入：类型目录 + 待分配的 event 列表
   - 为每个 entity 指定一种 type
   - 输出：entity（type、name、description）→ 写入 SQL entities 表

## 模块关系

- **上游**：无。entity_types 是静态种子数据
- **下游**：
  - Miner 2：读取类型目录，在分配 entity 时指定 type
  - dreaming：按 type 权重调整 entity 评分
  - active-searcher：按 type 权重调整搜索命中排名
- **无关**：memory-miner 的 Miner 1（不涉 entity）、system_prompt 模块
