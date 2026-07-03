# 出站链路

## 概述

出站 Processor 链对 LLM 响应和斜杠指令回复执行内容过滤、DSL 解析和日志记录。

链处理完毕后产出 [ProcessedMessage](../common/shared-types.md#processedmessage)，由 Gateway 协调 IM Adapter 完成渲染和发送。流式和非流式出站统一经此链，VerbosityFilter 和 DslParser 对流式增量文本零开销透传。

## 架构

出站链由三个 Processor 按 priority 升序执行。各平台有各自的 IM Adapter（如飞书 Adapter + 飞书 Renderer），Gateway 根据目标平台选择对应 IM Adapter：

```
Gateway 从 Session 读取 ContentBlock[]，构造 [ProcessedMessage](../common/shared-types.md#processedmessage)
  ↓
Processor 链（出站，按 priority 升序执行）
  1. VerbosityFilter（priority 5）
     → 按 Session 当前 Verbosity 等级逐块过滤 ContentBlock[]
     → Full：不过滤；Normal：移除 Thinking 块；Off：仅保留 Text 块
  2. DslParser（priority 10）
     → 遍历 ContentBlock[] 中的 Text 块
     → 匹配并解析 DSL 指令行（::button[...] 等）
     → 从 Text 块中剥离 DSL 行
     → 解析结果写入 metadata
     → Thinking / ToolUse / ToolResult 块直接透传
  3. OutboundRawLog（priority 20）
     → 将处理后的 ContentBlock[] 和 metadata 写入出站日志文件
     → 仅在 raw_log_dir 配置时注册
  ↓
ProcessedMessage { content_blocks, metadata }
  ↓
Gateway 选择目标平台 IM 插件
  → 接收 ContentBlock[]（定义见 [common ContentBlock](../common/shared-types.md#contentblock)）+ DslParseResult（定义见 [common DslParseResult](../common/shared-types.md#dslparseresult-和-dslinstruction)）
  → 按块类型选择渲染策略
  → 输出平台原生格式并发送

IM Adapter 不在 Processor 链内——渲染是终结操作，输出后无后续处理器接力。

## 数据流

```
LLM 输出 UnifiedResponse（含 ContentBlock[]）
  → Session 写入 messages[]
    → Gateway 从 Session 读取 ContentBlock[]
      → 构造 [ProcessedMessage](../common/shared-types.md#processedmessage)，启动出站 Processor 链
        → VerbosityFilter（pri 5）：按 Verbosity 等级逐块过滤 ContentBlock[]
        → DslParser（pri 10）遍历 ContentBlock[]：
            - Text 块 → 逐行扫描 DSL → 解析 → 剥离 DSL 行
            - Thinking 块 → 透传
            - ToolUse 块 → 透传
            - ToolResult 块 → 透传
            输出：更新的 ContentBlock[] + metadata["dsl_result"]
        → OutboundRawLog（pri 20）：写入出站日志
            ↓
      → 链输出 [ProcessedMessage](../common/shared-types.md#processedmessage)
        → Gateway 选择目标平台 IM 插件
          → 插件内部渲染(content_blocks, dsl_result)：
            - Text 块 → 平台文本 / 富文本格式
            - Thinking 块 → 平台折叠内容
            - ToolUse 块 → 平台工具调用展示
            - ToolResult 块 → 平台工具结果展示
          → 输出平台原生格式
        → [中间件插入点] — Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件
        → IM Adapter 发送
        → 根据 (peer_id, thread_id) 发送到对应会话/话题
```

Verbosity 过滤等级定义见 [slash 模块 verbose 指令](../slash/verbose.md)。

关键判断点：
- VerbosityFilter 以单个 ContentBlock 为粒度过滤，Full 不过滤、Normal 移除 Thinking、Off 仅保留 Text 块
- DslParser 仅处理 ContentBlock::Text 类型，其他块透传
- 无 DSL 指令时 DslParser 输出与输入一致（零开销透传）
- OutboundRawLog 仅在 raw_log_dir 配置时注册，未配置时链中只有 VerbosityFilter + DslParser
- 无目标平台或平台不支持时，回退到纯文本输出
- 斜杠指令回复的 ContentBlock[] 经 Gateway 传入同一条出站链，与 LLM 回复走相同处理路径

## 模块关系

- **上游**：[Gateway](../gateway/README.md)（编排 Processor Chain 调度，将 ContentBlock[] 传入链）、Session（LLM 对话产出的 ContentBlock[]，经 Gateway 传递进入链，属数据流上游依赖）、SlashDispatcher（斜杠指令回复的 ContentBlock[]）
- **下游**：[IM Adapter](../im_adapter/README.md) 模块（消费 ContentBlock[] + DslParseResult（定义见 [common DslParseResult](../common/shared-types.md#dslparseresult-和-dslinstruction)），渲染为平台格式并发送）
- **链内**：
  - VerbosityFilter — 按 Session Verbosity 等级逐块过滤内容，优先于 DSL 解析
  - DslParser — 解析 DSL 指令，为渲染提供交互数据
  - OutboundRawLog — 将处理后的出站内容写入日志文件
- **无关**：入站 Processor 链（独立链路，与出站互不干扰）
