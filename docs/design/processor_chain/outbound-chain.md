# 出站链路

## 概述

本文档描述 Processor 链框架如何调度出站链路的两个 Processor（DslParser 和平台渲染 Processor），以及 Gateway 在链调度中的角色。

## 架构

出站链路由两个 Processor 组成，按 priority 顺序执行：

```
ProcessedMessage { content: ContentBlock[] }
  ↓
DslParser（priority 10）
  → 遍历 Text 块，识别 DSL 指令行
  → 解析结果写入 metadata["dsl_result"]
  → 从 Text 块中剥离 DSL 指令行
  ↓
<Platform>Renderer（priority 20）
  → 按 ContentBlock 类型选择渲染策略
  → 消费 metadata 中的 DSL 指令，渲染平台交互元素
  → 输出平台原生格式 payload
  ↓
ProcessedMessage { content: platform_payload, metadata }
  ↓
Gateway 提取 payload → IM Adapter 发送
```

Gateway 负责：
- 从 Session 中读取消息的 ContentBlock 数组
- 根据目标平台将对应渲染 Processor 注册到链中
- 启动 Processor 链执行
- 从链输出中提取平台 payload 并交给 IM Adapter

渲染 Processor 的实现完全在 Processor 框架内，Gateway 无需感知渲染细节。

## 数据流

```
Session 消息（含 ContentBlock[]）
  ↓ Gateway 构造 ProcessedMessage
DslParser.process(ctx)
  → 输入：含 ContentBlock 数组的消息上下文
  → 处理：遍历 Text 块，匹配 DSL 语法 → 解析为结构化指令 → 剥离 DSL 行；
    Thinking/ToolUse/ToolResult 块不参与 DSL 解析，直接透传
  → 输出：clean Text 块（含透传的非 Text 块）+ metadata["dsl_result"]（DslParseResult）
  ↓
<Platform>Renderer.process(ctx)
  → 输入：清理后的 ContentBlock[] + metadata 中的 DSL 结果
  → 处理：
      - Text 块 → 平台文本/富文本格式
      - Thinking 块 → 平台折叠内容
      - ToolUse 块 → 平台工具调用展示
      - ToolResult 块 → 平台工具结果展示
      - DSL 指令 → 平台交互元素（按钮、选择器等）
  → 输出：平台原生格式 payload（JSON 或结构化数据）
  ↓
Gateway 提取 payload → IM Adapter
```

关键判断点：
- DslParser 不处理非 Text 块（Thinking、Tool 块直接透传）
- 渲染 Processor 根据消息的整体内容决定输出类型（text 或富格式）
- 若链中无匹配的渲染 Processor，Gateway 回退到纯文本输出

## 模块关系

- **上游**：Session（提供 ContentBlock[]）；LLM Provider 为数据来源（生成 UnifiedResponse 和 ContentBlock 类型，但不直接调用 processor_chain）
- **下游**：IM Adapter（接收渲染后的平台消息并发送）
- **链内**：
  - DslParser — DSL 指令解析，为渲染 Processor 提供交互数据
  - 各平台渲染 Processor — 将 ContentBlock[] 转为平台原生格式
- **无关**：入站 Processor 链（独立链路，与出站互不干扰）
