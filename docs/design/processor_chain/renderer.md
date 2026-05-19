# 渲染处理器

## 概述

渲染层（Renderer）是消息传递格式到展示格式的唯一转换点。它接收 Processor 链处理后的 ContentBlock[] 和 DSL 解析结果，按平台渲染为原生消息格式。

Renderer 是独立的渲染层，不属于 Processor 链。每个 IM 平台提供一个 Renderer 实现，由 Gateway 根据目标平台选择。

## 架构

```
Processor 链出站输出（ProcessedMessage { content_blocks, metadata }）
  ↓
Renderer 层
  ├── ContentBlock[] 输入
  │     ├── Text        — 文本内容（含 markdown 格式）
  │     ├── Thinking    — 推理追踪
  │     ├── ToolUse     — 工具调用请求
  │     └── ToolResult  — 工具执行结果
  ├── DslParseResult 输入
  │     └── 交互指令集（按钮、选择器等）
  ↓
渲染逻辑
  ├── 消息类型决策 — text 或富格式
  ├── 块类型映射   — 每种 ContentBlock 对应的平台展示方式
  ├── 格式转换     — markdown → 平台原生格式
  └── DSL 注入     — 交互指令 → 平台交互元素
  ↓
RenderedOutput { msg_type, payload }
  ↓
IM Adapter 发送
```

各平台 Renderer 共享相同的输入结构，差异仅在输出格式：
- 飞书 → interactive card JSON
- CLI → ANSI 彩色文本

## 数据流

```
Processor 链输出（ContentBlock[] + metadata["dsl_result"]）
  ↓
Gateway 选择 Renderer（根据目标平台）
  ↓
Renderer 接收 ContentBlock[] 和 DslParseResult
  ↓
遍历 ContentBlock[]：
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
RenderedOutput { msg_type, payload }
  ↓
Gateway 提取 payload → IM Adapter 发送
```

## 模块关系

- **上游**：Processor 链出站输出（ContentBlock[] + DSL 解析结果）
- **下游**：IM Adapter（接收 RenderedOutput 并发送）
- **平台实现**：
  - [飞书渲染](renderer-feishu.md) — 飞书 interactive card 渲染规则
  - [代码块渲染](code-render.md) — 代码块语法高亮
  - [流式渲染](streaming-render.md) — 流式增量输出
- **无关**：入站 Processor 链（不经过渲染层）
