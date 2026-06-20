# active-searcher

## 概述

每条消息（用户消息和 agent 消息）触发独立搜索，从 memory store 中所有条目检索相关内容，浓缩为摘要后排队到下一轮消息列表注入。确保记忆召回覆盖所有消息，不依赖主 agent 判断。

## 架构

active-searcher 是轻量 sub-agent，每条消息异步运行，使用独立低开销模型。

```
用户消息 / 主 agent 生成消息后
  │
  ▼
session 层 spawn active-searcher sub-agent（独立低开销模型）
  │
  ├── 输入：当前消息 + 最近 N 轮对话上下文
  ├── 搜索：BM25 + 向量混合检索 memory store 中所有条目（含 miner 产出和 dreaming 产出的 MEMORY.md）
  ├── 过滤：排除 per-session "已注入条目 ID 集合"中已有条目
  ├── 浓缩：匹配条目 → 文本摘要
  └── 输出：浓缩摘要（≤ 可配置字符上限）
        │
        ▼
     排队注入
```

**搜索模型**：独立配置，追求便宜 + 快，不等同于主对话模型。超时时间可配，超时则放弃本轮。

### 注入时机

摘要不随当前消息即时注入，而是排队等待下一轮：

- **agent 消息触发**：LLM 生成消息 A → active-searcher 匹配 → 摘要 C 排队。下一次用户输入 B（或工具返回 B）到达时，C 插入 B 之前，一起发给 LLM
- **用户消息触发**：用户消息 B → active-searcher 匹配 → 摘要 C 当场插入 B 之后，一起发给 LLM
- **C 的形式**：tool role 消息，不占对话轮次，用户不可见

```
消息序列示意（用户消息触发的场景）：

  [用户消息 B]
  [tool: memory 摘要 C]   ← 当场插入
  → 一起发给 LLM

消息序列示意（agent 消息触发的场景）：

  上一轮 LLM 生成消息 A → active-searcher 匹配，摘要 C 排队
  ...
  [tool: memory 摘要 C]   ← 下一轮插入
  [用户消息 B / 工具返回 B]
  → 一起发给 LLM
```

### 去重与重置

**去重**：per-session 维护"已注入条目 ID 集合"。搜索结果中的条目 ID 若已在集合中，过滤掉，不重复注入。

**重置时机**（清空集合）：
- 新 session 启动（全新 session ID）
- 上下文压缩发生后（历史消息被压缩，之前注入的摘要已不在上下文中）

**不复位时机**：同一 session idle 后恢复——上下文是接续的，之前注入的摘要还在消息列表里，集合继续沿用。

## 数据流

```
输入                      处理                    输出
─────                    ─────                  ─────
当前消息               ─→  BM25 + 向量混合搜索   ─→  浓缩摘要
最近 N 轮对话上下文     ─→  搜索 memory store          ≤ 字符上限
                        所有条目                  tool role 消息
                        去重过滤                  │
                        浓缩                      ▼
                        │                    插入消息列表
                        ▼                    （下一轮输入前）
                    超时 → 放弃本轮
```

## 模块关系

- **上游**：
  - session 模块：spawn active-searcher sub-agent，由 session 层的 sub-agent 调度能力驱动
  - memory store：搜索所有记忆条目（含 miner 产出和 MEMORY.md）
  - llm 模块：提供独立低开销模型，供 searcher sub-agent 使用

- **下游**：
  - session 模块：注入消息列表，由 session 在下一轮输入组装时合并 tool role 摘要

- **无关**：
  - system_prompt 模块：active-searcher 不修改 system prompt，只插入消息列表
