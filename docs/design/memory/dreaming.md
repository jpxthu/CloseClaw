# dreaming

## 概述

定期对结构化记忆条目进行三阶段升格（Light → REM → Deep），筛选高价值条目写入 MEMORY.md，作为长期记忆的精选版本。MEMORY.md 由 system_prompt 模块直接读取注入，不做二次浓缩。

## 架构

dreaming 由三阶段组成，每阶段职责单一：

```
结构化记忆条目（memory-miner 产出）
  │
  ▼
Light 阶段  —— 程序化处理：增量读取（仅新条目）、去重、按来源会话分块
  │
  ▼
REM 阶段   —— 程序化处理：统计提取、概念标签聚合
  │
  ▼
Deep 阶段  —— 程序化处理：多维加权评分，对每个候选条目打分，过门槛后写入 MEMORY.md
  │
  ▼
MEMORY.md  —— 长期记忆精选版本，由 system_prompt 在 session 启动时直接注入
```

**三阶段均为程序化处理**，不经过 LLM。

**触发**：由 Daemon 层的 DreamingScheduler 定时任务驱动，在配置的做梦时段内执行。任务处理对象：`mined=true` 且尚未 dreaming 的会话对应条目。DreamingScheduler 的整体调度顺序（先 dreaming 后 mining）详见 README。

### Light 阶段

- 增量读取：仅处理上次 dreaming 后新产生的条目
- 去重：与 MEMORY.md 已有条目做语义去重（BM25 相似度 + 向量相似度）
- 分块：按来源会话分组，保持条目上下文关联

### REM 阶段

- 统计提取：跨条目统计关键词共现、类别分布
- 标签聚合：将语义相似的条目打上概念标签，为 Deep 阶段提供特征

### Deep 阶段

多维加权评分，维度包含：

| 维度 | 含义 | 权重可调 |
|------|------|----------|
| 频次 | 相似信息在多个会话中出现 | 正向 |
| 时效 | 最近产生的条目权重更高 | 正向（时间衰减） |
| 明确性 | owner 明确表述 vs agent 推断 | owner 明确表述加分 |
| 持久性 | 决策/偏好 vs 临时事实 | 决策/偏好加分 |
| 关联度 | 与已有 MEMORY.md 条目关联紧密 | 关联紧密加分 |
| 负面信号 | 可能被后续对话推翻 | 减分 |

**三道门槛**：
- 绝对阈值：总评分低于下限的条目直接丢弃
- 相对阈值：同类条目间评分相对过低的丢弃
- 容量上限：MEMORY.md 条目数达到上限时，低分条目溢出

### 防污染

- dreaming 自身的产出不参与后续 dreaming 的 ingestion（防自循环）
- 写入 MEMORY.md 前确认源条目仍存在且未被修改（防写入脏数据）

## 数据流

```
输入                    处理                    输出
─────                  ─────                  ─────
结构化记忆条目    ─→    Light                 ─→   MEMORY.md
（增量）               · 增量读取                  精选条目
                       · 去重                      
                       · 分块                      
                       │
                       ▼
                      REM
                       · 统计提取
                       · 标签聚合
                       │
                       ▼
                      Deep
                       · 多维评分
                       · 三道门槛过滤
```

## 模块关系

- **上游**：
  - memory-miner：产出结构化记忆条目，是 dreaming 的唯一数据来源
  - memory store：读写记忆条目和 MEMORY.md
  - daemon 模块：DreamingScheduler 定时任务触发

- **下游**：
  - system_prompt 模块：消费 MEMORY.md（关系详见 README 模块级下游）

- **无关**：
  - active-searcher：dreaming 不参与实时搜索注入，两者无直接交互
