# RenderedOutput

## 概述

RenderedOutput 是 IMPlugin 渲染方法产出的平台原生格式消息结构。渲染产出数据，发送执行副作用——Gateway 在两步之间插入中间件（审计、频率限制等）。StreamingOutput 是流式模式下逐个产出的渲染块，用于支持 IM 平台的流式消息更新。

> **本文档定义的 RenderedOutput、StreamingOutput 在 common crate 中实现。引用本模块的下游文档通过 [ContentBlock](content-block.md)、[ProcessedMessage](processed-message.md) 等链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### RenderedOutput

| 字段 | 类型 | 说明 |
|------|------|------|
| `msg_type` | string | 消息格式类型（如 `"text"`、`"interactive"`），由 Renderer 按内容特征选择 |
| `payload` | any | 平台原生格式的消息体，结构由各平台 Renderer 定义。Gateway 中间件和 Adapter 发送不解析 payload 内容 |

**输出格式决策**：各平台 Renderer 按 ContentBlock 类型组合选择 msg_type——纯文本块（不含 Thinking/ToolUse/ToolResult）→ `"text"`；含 Thinking/ToolUse/ToolResult 块或多块 → `"interactive"`。

### StreamingOutput

StreamingOutput 是流式模式下逐个产出的渲染块，用于支持 IM 平台的流式消息更新。

> **文档编写中** — StreamingOutput 的具体字段定义待流式渲染方案确定后细化。

## 数据流

RenderedOutput 的流动嵌入在 IM Adapter 出站渲染流程中：

```
ContentBlock[] + DslParseResult（经 Processor Chain 出站处理后）
  ↓
IMPlugin.render() → RenderedOutput { msg_type, payload }
  ↓
[Gateway 中间件插入点] — 审计、频率限制等
  ↓
IMPlugin.send(payload, peer_id, thread_id) → 平台发送 API
```

RenderedOutput 的生命周期：IMPlugin 渲染产出 → Gateway 中间件 → IMPlugin 发送后销毁。

## 模块关系

- **生产者**：IM Adapter 各平台 Renderer（IMPlugin.render() 产出）
- **消费者**：Gateway（中间件插入点，在渲染和发送之间）；IM Adapter（IMPlugin.send() 消费 payload 发送）
- **无关**：Processor Chain（RenderedOutput 在 Processor Chain 之后产出，不经过链处理）、LLM Provider（不接触 RenderedOutput）
