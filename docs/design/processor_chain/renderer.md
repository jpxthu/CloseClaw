# 渲染处理器

## 概述

渲染处理器是出站链中将 LLM 结构化输出转为平台原生消息的 Processor。每个 IM 平台提供一个渲染 Processor 实现，与其他 Processor 在同一链中按 priority 顺序执行。

## 架构

渲染处理器在出站链中位于 DslParser 之后，消费清理后的结构化内容块和 DSL 解析结果：

```
DslParser 输出
  ↓
渲染 Processor（priority 20）
  ├── ContentBlock[] 输入
  │     ├── Text        — 文本内容（含 markdown 格式）
  │     ├── Thinking    — 推理追踪
  │     ├── ToolUse     — 工具调用请求
  │     └── ToolResult  — 工具执行结果
  ├── metadata["dsl_result"] 输入
  │     └── 交互指令集（按钮、选择器等）
  ↓
渲染逻辑
  ├── 消息类型决策 — text 或富格式
  ├── 块类型映射   — 每种 ContentBlock 对应的平台展示方式
  ├── 格式转换     — markdown → 平台原生格式
  └── DSL 注入     — 交互指令 → 平台交互元素
  ↓
ProcessedMessage（content 为平台原生格式 payload）
```

各平台渲染 Processor 共享相同的输入结构，差异仅在输出格式：
- 飞书 → interactive card JSON
- CLI → ANSI 彩色文本
- 其他平台扩展 → 对应平台的原生消息格式

## 数据流

```
DslParser 输出（clean ContentBlock[] + metadata["dsl_result"]）
  ↓
渲染 Processor 接收上下文
  ↓
遍历 ContentBlock 数组：
  ├── Text 块
  │     → 解析 markdown 格式（标题、粗体、代码块、列表等）
  │     → 映射为平台对应的展示元素
  │     → 含 DSL 嵌入标记时，替换为交互元素占位
  ├── Thinking 块
  │     → 渲染为折叠区块（平台支持时）
  │     → 平台不支持时，渲染为加粗斜体文本或省略
  ├── ToolUse 块
  │     → 渲染为工具调用信息卡片
  ├── ToolResult 块
  │     → 渲染为工具返回结果区块
  └── DSL 指令
        → 渲染为平台交互控件（按钮、选择器等）
  ↓
输出决策：
  ├── 纯文本、无格式、无 DSL → text 消息
  └── 含格式、多内容块、或有 DSL → 富格式消息
  ↓
ProcessedMessage { content: platform_payload, metadata }
```

## 模块关系

- **上游**：DslParser（提供清理后的 ContentBlock[] 和 DSL 解析结果）
- **下游**：Gateway（提取平台 payload 传递给 IM Adapter 发送）
- **平台实现**：
  - [飞书渲染](renderer-feishu.md) — 飞书 interactive card 渲染规则
  - [代码块渲染](code-render.md) — 代码块语法高亮
  - [流式渲染](streaming-render.md) — 流式增量输出
- **无关**：入站 Processor 链（独立链路，不经过渲染处理器）
