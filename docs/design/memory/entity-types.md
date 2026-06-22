# 实体类型

## 概述

实体类型（entity type）定义了记忆系统中 entity 的分类体系。miner 在生成 entity 时为每个 entity 指定一种类型，类型决定了该 entity 在搜索时的权重和 dreaming 时的聚类行为。

类型体系直接沿用 SAG 的 11 种分类，不做自创。后续根据 CloseClaw 的实际运行数据进行调整。

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

**权重**：在 dreaming 和 searcher 的评分和排序中使用。`subject`（1.5）和 `action`（1.3）是最高价值类型。

**相似度阈值**：用于向量相似度匹配的最低余弦距离门槛。值越高匹配越严格。

### 存储

entity_types 存储在 SQLite 中。建表时写入 11 种类型的种子数据。每条类型记录包含：id、type、name、description、weight、similarity_threshold、is_default、is_active。

`is_default=true` 的类型在其所属 type 下优先匹配。`is_active=false` 的类型不参与类型解析。

## 数据流

1. 系统初始化 / 首次启动：SQLite 建表 → 写入 11 种 entity_type 种子数据
2. Miner 2 在分配 entity 时
   - 输入：完整 entity 目录（合并 entities 表和 entity_types 表，从 SQL 生成，按 type → name 固定排序）
   - 为每个 entity 指定一种 type
   - 输出：entity（type、name、description）→ 写入 SQL entities 表

**类型目录生成**：在 miner 2 运行前，合并 SQLite `entities` 表和 `entity_types` 表的数据，按 `type` → `normalized_name` 字母序排列，生成固定格式的文本列表，作为 miner 2 prompt 的一部分。固定排序旨在最大化 KV Cache 命中率。

## 模块关系

- **上游**：无。entity_types 是静态种子数据
- **下游**：
  - miner 2：读取类型目录，在分配 entity 时指定 type
  - dreaming：按 type 权重调整 entity 评分
  - active-searcher：按 type 权重调整搜索命中排名
- **无关**：memory-miner 的 Miner 1（不涉 entity）、system_prompt 模块
