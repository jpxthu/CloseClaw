# DSL 解析器

## 概述

DslParser 从 LLM 输出的消息中识别和解析 DSL 指令。DSL 指令用于定义消息中的交互元素，如按钮、表单等。

## 架构

DslParser 在出站方向以最高优先级首先执行：

```
输入：消息文本（来自 UnifiedResponse 的消息内容块，仅提取文本类型，跳过推理、工具调用等非文本内容）
  ↓
逐行扫描，匹配 DSL 指令模式
  ↓
解析 DSL 语法，生成结构化指令
  ↓
写入 ProcessedMessage.metadata["dsl_result"]
  ↓
输出：ProcessedMessage（content 不变，metadata 追加解析结果）
```

DslParser 只解析 DSL 指令，不修改消息内容本身。解析结果附着在 metadata 中，后续由 Gateway 提取并传递给 Renderer，用于渲染平台交互元素。

## 数据流

```
消息文本（来自 UnifiedResponse 的消息内容块）
  → Processor 链按 priority 调度 DslParser
    → 扫描文本中的 DSL 指令行
      → 有 DSL 指令：解析为结构化数据，存入 metadata
      → 无 DSL 指令：透传，metadata 中 dsl_result 为空
  → Gateway 从 metadata 提取 DslParseResult
    → 传递给 Renderer 用于渲染交互按钮等元素
```

## 模块关系

- **上游**：Processor 链框架（按 priority 调度执行）
- **下游**：Gateway（解析结果通过 metadata 传递给 Renderer，用于渲染交互元素）
- **无关**：MarkdownToCard（已由 Renderer 替代，与 DslParser 无调用关系）
