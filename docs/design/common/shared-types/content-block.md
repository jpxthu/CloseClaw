# ContentBlock

## 概述

ContentBlock 是跨模块传递的结构化内容单元。主要用于出站方向——入站方向仅使用 ContentBlock::Text 单个变体作为 ProcessedMessage 的内容载体。所有出站内容——LLM 回复和斜杠指令回复——均以 ContentBlock[] 数组形式传递，贯穿 Verbosity 过滤、DSL 解析、出站日志记录和平台渲染全链路。

> **本文档定义的 ContentBlock、ContentDelta、ContentBlockType 在 common crate 中实现。引用本模块的下游文档通过 [NormalizedMessage](inbound-message.md)、[ProcessedMessage](processed-message.md) 等链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### ContentBlock

ContentBlock[] 的类型定义服务于出站方向——LLM 和斜杠指令以 ContentBlock[] 产出结构化内容。入站方向经 Processor Chain 处理后，标准化文本以 ContentBlock::Text 形式放入 [ProcessedMessage](processed-message.md) 的 content_blocks 字段，入站不涉及 ContentBlock 的其他变体。

ContentBlock 共 7 种变体，按语义和渲染策略分为两类：

**文本类变体**：

| 变体 | 语义 | 渲染行为 |
|------|------|----------|
| Text | 文本内容，可含 markdown 格式标记和 DSL 指令行。ContentBlock 中唯一参与 DSL 解析的变体 | DSL 行由 DslParser 剥离后渲染纯文本/富文本。终端输出 ANSI 格式化文本，IM 平台按平台能力输出 markdown 元素 |
| Thinking | LLM 推理过程，终端用户可选的思考展示 | 默认折叠展示（终端 ANSI dim 样式包裹，IM 平台折叠区块）。流式模式下等待全块就绪后一次渲染。DslParser 透传 |

**非文本类变体**（DslParser 透传）：

| 变体 | 语义 | 渲染行为 |
|------|------|----------|
| ToolUse | 工具调用请求，含工具名和参数 | 渲染为工具调用信息展示（终端文本，IM 平台卡片）。参数以原始结构渲染 |
| ToolResult | 工具执行结果 | 渲染为结果内容展示。终端按宽度截断，IM 平台富格式渲染 |
| Image | 图片引用，含资源标识和访问地址 | 终端渲染为占位符文本 `[image: name]`，IM 平台渲染为图片元素 |
| Audio | 音频引用，含资源标识和访问地址 | 终端渲染为占位符文本 `[audio: name]`，IM 平台渲染为音频元素 |
| File | 文件引用，含资源标识和访问地址 | 终端渲染为占位符文本 `[file: name]`，IM 平台渲染为文件元素 |

#### 变体处理规则

- **Text 是唯一可能包含 DSL 指令的变体**。DslParser 仅遍历 Text 块逐行扫描 DSL，解析后从 Text 块中移除 DSL 行。其余 6 种变体由 DslParser 透传
- **流式渲染差异化**：Text 块逐行缓冲输出（以句末标点或换行符为行边界）；Thinking/ToolUse/ToolResult 块等待全块就绪后一次交付渲染；Image/Audio/File 块不参与流式渲染，交由平台格式渲染器处理
- **输出格式决策**：各平台 Renderer 按 ContentBlock 类型组合选择输出格式——纯文本块（不含 Thinking/ToolUse/ToolResult 块）→ 纯文本消息；含 Thinking/ToolUse/ToolResult 块或多块 → 富格式/卡片消息
- **Verbosity 过滤**以单个 ContentBlock 为粒度执行——每个 ContentBlock 到达时按当前 Session 的 verbosity 等级判断其可见性，流式模式下逐块实时过滤。Verbosity 等级定义见 [runtime-config VerbosityLevel](runtime-config.md#verbositylevel)

### ContentDelta

ContentDelta 是流式模式下 ContentBlock 的增量更新单元。

> **文档编写中** — ContentDelta 的具体字段和流式合并规则待定。当前流式模式下 Text 块逐行缓冲输出，非文本类块等全块就绪后一次渲染。

### ContentBlockType

ContentBlockType 是 ContentBlock 变体类型的枚举标识，用于 Renderer 按类型选择渲染策略。

| 值 | 对应变体 | 渲染策略 |
|----|----------|----------|
| `text` | Text | 文本/富文本内容 |
| `thinking` | Thinking | 折叠展示 |
| `tool_use` | ToolUse | 工具调用信息卡片 |
| `tool_result` | ToolResult | 结果内容展示 |
| `image` | Image | 图片元素 |
| `audio` | Audio | 音频元素 |
| `file` | File | 文件元素 |

## 数据流

ContentBlock[] 的出站流动路径：

```
LLM UnifiedResponse / SlashResult 变体
  ↓
ContentBlock[] 进入出站处理链路
  ↓
[Processor Chain 出站: VerbosityFilter → DslParser → OutboundRawLog]
  ↓
ProcessedMessage { content_blocks, metadata[dsl_result] }
  ↓
[IM Adapter 渲染] — 按块类型选择渲染策略，输出平台原生格式
  ├─ 批量模式：一次性渲染全部 ContentBlock[]
  └─ 流式模式：增量渲染，Text 块逐行缓冲输出，非文本类块等全块就绪后一次渲染
  ↓
[中间件插入点] — Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件。中间件为 Gateway 内部的拦截链，具体中间件类型和注册机制由 Gateway 管理
  ↓
IM Adapter 发送到目标平台
```

ContentBlock[] 流式与非流式走同一条预处理管线——Verbosity 过滤和 DslParser 解析同时适用于批量和流式。DslParser 对流式增量文本零开销透传。两者的差异仅在渲染阶段：批量模式一次性渲染，流式模式增量渲染。

入站方向：ContentNormalizer 将标准化后的文本包装为单个 `ContentBlock::Text` 放入 [ProcessedMessage](processed-message.md) 的 `content_blocks[0]` 字段。

## 模块关系

- **生产者**：Session（LLM 对话产出 UnifiedResponse，含 ContentBlock[]）、SlashDispatcher（斜杠指令回复以 [SlashResult](slash-result.md) 变体产出 ContentBlock[]）、Processor Chain 入站 ContentNormalizer（入站方向包装标准化文本为 ContentBlock::Text 放入 ProcessedMessage.content_blocks）
- **消费者**：Processor Chain 出站（VerbosityFilter → DslParser → OutboundRawLog）→ IM Adapter（按块类型渲染为平台原生格式并发送）
- **无关**：IM Adapter 入站链（入站方向产 NormalizedMessage，不涉及 ContentBlock[]）、Session 生命周期管理（不直接操作 ContentBlock[]，仅通过 Gateway 间接消费）、LLM Provider（LLM 调用产出 ContentBlock[]，但不参与 ContentBlock 的结构定义和处理流程）、Gateway（Gateway 编排 Processor Chain 调度，不直接执行内容过滤/解析）
