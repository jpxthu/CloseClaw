# 数据流总览

## 概述

本文档提供共享类型在全系统中的高层流动路径总览。各类型的完整数据流（字段级流动路径、判断分支、渲染差异）定义在 [shared-types.md](shared-types.md) 中，本文档仅做方向级概述和引用。

## 架构

共享类型按流动方向和生命周期分为入站、出站、特殊三类：

- **入站类型**：从外部消息进入系统，经处理后进入 LLM 对话
- **出站类型**：LLM 或斜杠指令产出，经处理后发送到外部
- **特殊类型**：生命周期跨越多个模块或方向，不纯粹属于入站或出站

## 数据流

### 入站方向

```
IM 平台 webhook / terminal stdin
  ↓
[IM Adapter 入站解析]
  平台格式 → NormalizedMessage { platform, sender_id, peer_id, thread_id?, account_id, content, message_type, media_refs, timestamp }
  ↓
[Processor Chain 入站]
  RawLog → SessionRouter → ContentNormalizer
  ↓
ProcessedMessage { content_blocks: [ContentBlock::Text], metadata: { session_key, message_type } }
  ↓
[Gateway 路由决策]
  非 text → 构造错误回复（简化出站，跳过 Verbosity/DslParser）
  / 开头 → SlashDispatcher
  普通 → Session → LLM
```

入站方向涉及两种共享类型：

- **[NormalizedMessage](shared-types.md#normalizedmessage)**：IM Adapter 产出 → Processor Chain 消费。各平台 Adapter 在入站解析时填充全部字段。消息过滤在 Adapter 解析阶段完成（空 text 丢弃，非 text 正常产出 NormalizedMessage）。
- **[ProcessedMessage](shared-types.md#processedmessage)**（入站形态）：Processor Chain 入站产出 → Gateway 消费。content_blocks 为单个 ContentBlock::Text，metadata 含 session_key。生命周期止于 Gateway 完成路由决策。

### 出站方向

```
LLM UnifiedResponse / SlashResult 变体
  ↓
ContentBlock[]
  ↓
[Processor Chain 出站]
  VerbosityFilter → DslParser → OutboundRawLog
  ↓
ProcessedMessage { content_blocks, metadata: { dsl_result } }
  ↓
[IM Adapter 渲染]
  批量模式：一次性渲染全部 ContentBlock[]
  流式模式：增量渲染，Text 逐行缓冲输出；Thinking/ToolUse/ToolResult 全块就绪后一次渲染；Image/Audio/File 不参与流式渲染，交由平台格式渲染器处理
  ↓
[中间件插入点] — Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件
  ↓
IM Adapter 发送
```

出站方向涉及四种共享类型：

- **[ContentBlock](shared-types.md#contentblock)**：7 种变体（Text / Thinking / ToolUse / ToolResult / Image / Audio / File），仅 Text 变体参与 DSL 解析，其余 6 种变体由 DslParser 透传。从 LLM / SlashResult 产出 → Processor Chain 出站消费 → IM Adapter 渲染。
- **[DslParseResult / DslInstruction](shared-types.md#dslparseresult-和-dslinstruction)**：DslParser 从 ContentBlock::Text 中解析 DSL 指令行，产出 DslInstruction 列表。经 [ProcessedMessage](shared-types.md#processedmessage) 和出站日志传递，生命周期始于 DslParser、终于 Renderer 渲染。
- **[ProcessedMessage](shared-types.md#processedmessage)**（出站形态）：Processor Chain 出站产出 → Gateway 出站日志 → IM Adapter 渲染。content_blocks 为经 DslParser 处理后的 ContentBlock[]，metadata 含 dsl_result（DslParseResult 的序列化值）。
- **[SlashResult](shared-types.md#slashresult)**：10 种变体，SlashDispatcher Handler 返回 → Gateway 构造 SideEffectContext 触发执行。Exec 变体在执行前经 [Permission 模块](../permission/README.md) 校验。回复内容进入出站 Processor Chain，Session 操作通过 SideEffectContext 完成。

### 跨方向类型

[ProcessedMessage](shared-types.md#processedmessage) 是唯一跨入站/出站方向使用的类型——入站和出站共用同一结构，content_blocks 和 metadata 按方向携带不同内容。入站和出站不区分类型，同一结构按方向呈现不同形态。

## 模块关系

- **入站上游**：IM Adapter（产出 NormalizedMessage）
- **入站下游**：Gateway（消费 ProcessedMessage 做路由决策）
- **出站上游**：Session（LLM 产出 ContentBlock[]）、SlashDispatcher（产出 SlashResult）
- **出站下游**：IM Adapter（消费 ProcessedMessage 渲染并发送）
- **无关**：LLM Provider（不接触 ProcessedMessage，只产出 ContentBlock[]）、Session 生命周期管理（通过 Gateway 间接消费）
