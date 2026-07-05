# 共享类型

## 概述

共享类型是跨模块传递的纯数据结构，被 2 个及以上模块共同消费。每个共享类型在其所属的子文档中唯一定义，各业务模块文档通过引用指向此处，不在自身文档中重复描述字段结构。

> **本文档是 common crate 中共享类型的权威清单。** 若代码中 common crate 存在本文档未收录的 pub struct/enum，该类型不属于跨模块共享类型，应移至对应领域模块的 crate。反之，本文档定义的所有类型，代码中均位于 common crate（或其子 crate）。

本文档不包含 trait 接口定义——核心 trait 见 [core-traits](../core-traits.md)。

### 子文件索引

| 文件名 | 包含类型 | 简述 |
|--------|----------|------|
| [inbound-message.md](inbound-message.md) | NormalizedMessage, MediaRef, MessageType | 平台无关的统一入站消息结构 |
| [content-block.md](content-block.md) | ContentBlock, ContentDelta, ContentBlockType | 跨模块传递的结构化内容单元 |
| [dsl-parse-result.md](dsl-parse-result.md) | DslParseResult, DslInstruction | DSL 指令解析输出 |
| [processed-message.md](processed-message.md) | ProcessedMessage | Processor Chain 的输出结构 |
| [slash-result.md](slash-result.md) | SlashResult, SideEffectContext | 斜杠指令 Handler 执行结果 |
| [llm-response.md](llm-response.md) | UnifiedResponse, UnifiedUsage | LLM Provider 调用结果 |
| [session-state.md](session-state.md) | PlanState, SessionCheckpoint, SessionStatus, PersistResult | 会话状态相关类型 |
| [prompt-fragment.md](prompt-fragment.md) | FragmentContext, PromptFragment, BootstrapMode, SectionType | System prompt 构建上下文和片段 |
| [runtime-config.md](runtime-config.md) | CompactConfig, ReasoningLevel, VerbosityLevel, PromptOverrides | 运行时配置相关类型 |
| [pending-message.md](pending-message.md) | PendingMessage | 待发送消息的排队结构 |
| [rendered-output.md](rendered-output.md) | RenderedOutput, StreamingOutput | 平台原生格式消息结构 |

## 架构

### 类型总览

共享类型按语义群组划分为以下范畴：

- **入站消息**：`[NormalizedMessage](inbound-message.md)` — 平台无关的入站消息结构，屏蔽各 IM 平台差异；附带子结构 `[MediaRef](inbound-message.md#mediaref)`（资源引用）和 `[MessageType](inbound-message.md#messagetype)`（消息类型枚举）
- **结构化内容**：`[ContentBlock](content-block.md)` — 跨模块传递的结构化内容单元，共 7 种变体（Text、Thinking、ToolUse、ToolResult、Image、Audio、File）；附带 `[ContentDelta](content-block.md#contentdelta)`（流式增量）和 `[ContentBlockType](content-block.md#contentblocktype)`（变体类型枚举）
- **DSL 解析**：`[DslParseResult](dsl-parse-result.md)` — DSL 指令解析输出，包含 `[DslInstruction](dsl-parse-result.md#dslinstruction)` 列表
- **处理结果**：`[ProcessedMessage](processed-message.md)` — Processor Chain 处理后的统一输出结构
- **斜杠指令**：`[SlashResult](slash-result.md)` — 斜杠指令执行结果，共 10 种变体；附带执行上下文 `[SideEffectContext](slash-result.md#sideeffectcontext)`
- **LLM 响应**：`[UnifiedResponse](llm-response.md)` — LLM Provider 统一调用结果；附带 `[UnifiedUsage](llm-response.md#unifiedusage)`（Token 用量）
- **会话状态**：`[PlanState](session-state.md#planstate)` — Plan Mode 规划状态枚举；`[SessionCheckpoint](session-state.md#sessioncheckpoint)` — 会话检查点；`[SessionStatus](session-state.md#sessionstatus)` — 会话状态枚举；`[PersistResult](session-state.md#persistresult)` — 持久化结果
- **System Prompt**：`[FragmentContext](prompt-fragment.md#fragmentcontext)` — 片段生成上下文；`[PromptFragment](prompt-fragment.md#promptfragment)` — 片段产出；附带 `[BootstrapMode](prompt-fragment.md#bootstrapmode)`（引导模式枚举）和 `[SectionType](prompt-fragment.md#sectiontype)`（片段类型枚举）
- **运行时配置**：`[CompactConfig](runtime-config.md#compactconfig)` — 压缩配置；`[ReasoningLevel](runtime-config.md#reasoninglevel)` — 推理深度等级；`[VerbosityLevel](runtime-config.md#verbositylevel)` — 出站信息展示等级；`[PromptOverrides](runtime-config.md#promptoverrides)` — 提示词覆盖
- **出站排队**：`[PendingMessage](pending-message.md)` — 待发送消息的排队结构
- **平台输出**：`[RenderedOutput](rendered-output.md#renderedoutput)` — 平台原生格式消息结构；`[StreamingOutput](rendered-output.md#streamingoutput)` — 流式输出块

### 重要结构引用规则

- NormalizedMessage 中的 `message_type` 字段取值来自 `[MessageType](inbound-message.md#messagetype)` 枚举
- NormalizedMessage 中的 `media_refs` 字段元素类型为 `[MediaRef](inbound-message.md#mediaref)`
- ProcessedMessage 的 `content_blocks` 字段元素类型为 `[ContentBlock](content-block.md)`
- ProcessedMessage 的 `metadata` 中 `dsl_result` 字段为 `[DslParseResult](dsl-parse-result.md)` 的 JSON 序列化
- SlashResult 各变体执行时通过 `[SideEffectContext](slash-result.md#sideeffectcontext)` 的回复通道产出 `[ContentBlock](content-block.md)`[]
- UnifedResponse 的 `content_blocks` 字段元素类型为 `[ContentBlock](content-block.md)`
- 出站 Processor Chain 读取 Session 的 `[VerbosityLevel](runtime-config.md#verbositylevel)` 配置
- IM Adapter 渲染方法产出的类型为 `[RenderedOutput](rendered-output.md#renderedoutput)`

## 数据流

### 入站方向

```
IM 平台 webhook / terminal stdin
  ↓
IM Adapter 入站解析（各平台插件）
  → 平台格式转 [NormalizedMessage](inbound-message.md) { platform, sender_id, peer_id, thread_id?, account_id, content, message_type, media_refs, timestamp }
  ↓
Processor Chain 入站
  → RawLog（记录日志）→ SessionRouter（计算 session_key）→ ContentNormalizer（文本标准化）
  → 产出 [ProcessedMessage](processed-message.md)
  ↓
Gateway 路由
  → SessionManager 查找/创建 session → LLM 对话 / SlashDispatcher
```

入站方向详细路径见 [inbound-message.md 数据流节](inbound-message.md#数据流)。

### 出站方向

```
LLM [UnifiedResponse](llm-response.md) / [SlashResult](slash-result.md) 变体
  ↓
ContentBlock[] 进入出站处理链路
  ↓
[Processor Chain 出站: VerbosityFilter → DslParser → OutboundRawLog]
  ↓
[ProcessedMessage](processed-message.md) { content_blocks, metadata[dsl_result] }
  ↓
[IM Adapter 渲染] — 按块类型选择渲染策略，输出平台原生格式 [RenderedOutput](rendered-output.md)
  ├─ 批量模式：一次性渲染全部 ContentBlock[]
  └─ 流式模式：增量渲染，Text 块逐行缓冲输出，非文本类块等全块就绪后一次渲染
  ↓
[中间件插入点] — Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件
  ↓
IM Adapter 发送到目标平台
```

ContentBlock[] 流式与非流式走同一条预处理管线——Verbosity 过滤和 DslParser 解析同时适用于批量和流式。两者的差异仅在渲染阶段：批量模式一次性渲染，流式模式增量渲染。

各共享类型流动路径的详细描述见各子文件的数据流节。

## 模块关系

### 生产者/消费者总览

| 类型 | 生产者 | 消费者 |
|------|--------|--------|
| [NormalizedMessage](inbound-message.md) | IM Adapter 各平台插件 | Processor Chain 入站 |
| [ContentBlock](content-block.md) | Session（LLM 产出）、SlashDispatcher、ContentNormalizer | Processor Chain 出站、IM Adapter |
| [DslParseResult](dsl-parse-result.md) | Processor Chain 出站（DslParser） | IM Adapter Renderer、CLI TerminalRenderer |
| [ProcessedMessage](processed-message.md) | Processor Chain（入站/出站） | Gateway、IM Adapter |
| [SlashResult](slash-result.md) | SlashDispatcher（各 Handler） | Gateway |
| [FragmentContext](prompt-fragment.md) | System Prompt Builder | PromptFragmentProvider 实现者 |
| [PromptFragment](prompt-fragment.md) | PromptFragmentProvider 实现者 | System Prompt Builder |
| [RenderedOutput](rendered-output.md) | IM Adapter Renderer | Gateway、IM Adapter |
| [VerbosityLevel](runtime-config.md) | slash 模块（VerboseHandler） | Processor Chain 出站（VerbosityFilter）、Session |
| [PlanState](session-state.md) | mode 模块 | Session、mode 模块 |

各类型的详细模块关系见对应子文件的模块关系节。
