# Memory 模块

## 概述

Memory 模块为 agent 提供长期记忆能力——从会话中挖掘值得保留的信息，定期升格浓缩，在后续对话中适时注入，让 agent 跨 session 保持对用户偏好、历史决策和关键事实的记忆。

## 架构

Memory 体系由三模块组成，围绕一个共享记忆存储运转，两种触发机制互补：

```
触发 1：Sub-agent session 结束
  会话结束 hook → memory-miner 即时触发
  （生命周期明确的 session）

触发 2：定时 dreaming 任务（由 Daemon 层 DreamingScheduler 驱动）
  定时扫 archived 会话 → 先 dreaming（处理上轮 mining 产出的条目）→ 再 memory-miner（挖新会话的 transcript，供下轮 dreaming 消费）
  （owner 会话等无明确结束点的会话）

────────────────────────────────────────────

memory-miner     —— 独立 session 挖掘会话 transcript，产出结构化记忆条目
  │
  ▼
结构化记忆条目（Markdown 多文件，按来源会话组织）
  │
  ▼
dreaming         —— 定期触发，三阶段升格（Light→REM→Deep），写入 MEMORY.md
  │
  ▼
MEMORY.md（浓缩版长期记忆）→ system_prompt 直接注入

active-searcher  —— 每条消息触发搜索记忆条目，浓缩摘要插入消息列表
```

- **记忆条目 Schema**：每条条目包含类别（preference / decision / lesson / fact）、正文、时间戳、来源会话
- **存储格式**：Markdown 多文件，按来源会话组织，人类可读
- **搜索索引**：BM25 + 向量混合检索，支持增量更新

### 配置与开关

记忆功能通过配置开启，不配 = 全部关闭。

- **全局配置**：定时做梦时段、挖掘使用的模型、搜索模型
- **agent 覆盖**：每个 agent 可独立覆盖全局配置
- **不开不跑**：未配置的 agent 不触发任何记忆功能

### 子功能文档

| 子功能 | 简述 |
|--------|------|
| [memory-miner](memory-miner.md) | 会话 transcript 产出后，独立 session 挖掘，产出结构化记忆条目 |
| [active-searcher](active-searcher.md) | 每条消息触发搜索，浓缩摘要插入消息列表 |
| [dreaming](dreaming.md) | 定期三阶段记忆升格，将高价值条目写入 MEMORY.md |

## 数据流

### 记忆挖掘与升格

```
会话 transcript
    │
    ▼
memory-miner 挖掘（两种触发）
    │  触发 1：sub-agent session 结束 hook
    │  触发 2：Daemon DreamingScheduler 扫描 archived 且 mined=false 的会话
    │  输入：完整会话 transcript + 已有记忆条目 + 近期日常记忆
    │  处理：独立 session，专用挖掘 prompt
    │  输出：结构化记忆条目（类别、正文、时间戳、来源会话）
    │  完成后：标记 mined=true
    │
    ▼
dreaming 定时升格（Daemon DreamingScheduler 驱动）
    │  输入：结构化记忆条目（mined=true 且未 dreaming 的会话对应条目）
    │  处理：Light（去重分块）→ REM（模式提取）→ Deep（多维加权评分）
    │  输出：MEMORY.md（仅高价值条目）
    │
    ▼
MEMORY.md → system_prompt 直接注入

> MEMORY.md 的更新不会立刻反映到活跃 session 的 system prompt：system_prompt 静态层在 session 创建/恢复/compaction 时构建并缓存。dreaming 写入 MEMORY.md 后，活跃 session 读到的仍是缓存副本，直到下次 compaction 或新 session 创建才会刷新。
```

### 记忆注入（两条并行路径）

```
路径 1：Session 启动
  system_prompt 组装时读取 MEMORY.md → 注入 static system prompt

路径 2：每条消息
  当前消息 + 上下文 → active-searcher 搜索 → 浓缩摘要 → 插入消息列表（tool role）
  （注入时机与去重规则详见 active-searcher.md）
```

## 模块关系

- **上游**：
  - session 模块：sub-agent session 结束时触发 memory-miner（hook 机制）
  - daemon 模块：定时 dreaming 任务（DreamingScheduler），扫描 archived 会话触发 mining + dreaming

- **下游**：
  - system_prompt 模块：直接读取 MEMORY.md，作为 static system prompt 的长期记忆段来源
  - session 模块：active-searcher 的 tool role 摘要写入 `memory_injection` 槽位，由 session 下次组装消息时消费

- **无关**：
  - skills 模块：Memory 是基础设施层，不直接与 skills 交互
  - tools 模块：搜索索引的 BM25/向量检索是内部基础设施，不对 agent 暴露为 tool
