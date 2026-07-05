# UnifiedResponse

## 概述

UnifiedResponse 是 LLM Provider 的统一调用结果，封装 LLM 返回的结构化内容。UnifiedUsage 是 LLM 调用的 Token 用量统计。两者均由 LLM Provider 模块产出，经 [ContentBlock](content-block.md)[] 形式传递给出站 Processor Chain。

> **本文档定义的 UnifiedResponse、UnifiedUsage 在 common crate 中实现。引用本模块的下游文档通过 [ContentBlock](content-block.md)、[ProcessedMessage](processed-message.md) 等链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### UnifiedResponse

UnifiedResponse 是 LLM Provider 模块的统一输出结构，屏蔽不同 LLM 服务的返回格式差异。

> **文档编写中** — UnifiedResponse 的字段定义待各 LLM Provider 实现后细化。当前已知包含 ContentBlock[] 数组作为主要内容载体。

### UnifiedUsage

UnifiedUsage 是 LLM 调用的 Token 用量统计。

> **文档编写中** — UnifiedUsage 的字段定义待各 LLM Provider 实现后细化。当前已知包含输入/输出 token 计数等基本用量指标。

## 数据流

```
LLM Provider 调用（LLM Service → LLM Provider 适配层）
  ↓
UnifiedResponse { content_blocks: ContentBlock[], usage: UnifiedUsage }
  ↓
ContentBlock[] 进入出站 Processor Chain（VerbosityFilter → DslParser → OutboundRawLog）
  ↓
[ProcessedMessage](processed-message.md) → Gateway 出站日志 → IM Adapter 渲染发送
```

UnifiedUsage 在出站链路中用于 Token 用量统计和日志记录，不影响内容处理。

## 模块关系

- **生产者**：LLM Provider（对各 LLM 服务的统一封装，将不同格式的返回归一化为 UnifiedResponse）
- **消费者**：Gateway（消费 UnifiedResponse 的 content_blocks 进入出站 Processor Chain；记录 usage 做统计）、Session（记录 Token 用量）
- **无关**：IM Adapter（不接触 UnifiedResponse，只消费下游的 [ProcessedMessage](processed-message.md) 和 [ContentBlock](content-block.md)[]）、Processor Chain 入站（入站不涉及 UnifiedResponse）、SlashDispatcher（斜杠指令不依赖 LLM 响应）
