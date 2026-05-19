# 飞书渲染

## 概述

飞书渲染处理器将 LLM 的结构化内容块（ContentBlock[]）渲染为飞书消息格式（text 或 interactive card）。它是渲染层的飞书平台实现。

## 架构

飞书渲染分两步完成：输出类型决策和卡片组装。

**步骤一：输出类型决策**

| 条件 | 输出 |
|------|------|
| 纯文本，无格式标记、无换行、无 DSL | text 消息 |
| 含标题标记（`#` 开头） | interactive 卡片 |
| 含粗体、斜体、代码块、列表、引用、链接 | interactive 卡片 |
| 含换行符 | interactive 卡片 |
| 含 DSL 按钮指令 | interactive 卡片 |
| 含多个 ContentBlock 或 Thinking/Tool 块 | interactive 卡片 |

**步骤二：卡片组装**

```
ContentBlock[] + DSL 指令
  ↓
header 提取 — 首行 `# 标题` 作为卡片标题
  ↓
body 渲染
  ├── Text 块 → markdown 元素（飞书原生 markdown 渲染）
  ├── Thinking 块 → 可折叠推理区块
  ├── ToolUse 块 → 工具调用描述卡片
  └── ToolResult 块 → 工具结果内容块
  ↓
interactive 元素注入
  └── DSL 指令 → 飞书 button 等交互组件
  ↓
飞书卡片 JSON
```

## 数据流

```
渲染层接收 ContentBlock[] + DslParseResult
  ↓
遍历 ContentBlock[]：
  ├── Text 块
  │     → 解析 markdown 结构（标题、段落、列表、代码块等）
  │     → 映射为飞书 card element：
  │         - # 标题 → header.title（蓝色模板）
  │         - **粗体** / *斜体* → 飞书 markdown 原生渲染
  │         - ```代码块``` → 飞书 markdown 代码块
  │         - > 引用 → 飞书 markdown 引用
  │         - - 列表 → 飞书 markdown 列表
  │         - [链接](url) → 飞书 markdown 链接
  │         - --- 分割线 → 飞书 hr 元素
  ├── Thinking 块
  │     → 渲染为折叠区：标题"推理过程"，内容为推理文本
  ├── ToolUse 块
  │     → 渲染为信息条：工具名 + 输入参数摘要
  └── ToolResult 块
        → 渲染为内容块：工具返回结果（截断过长内容）
  ↓
DSL 指令渲染：
  ├── Button → 飞书 action button 元素
  │     - 首个按钮 → primary 样式
  │     - 后续按钮 → default 样式
  └── 其他指令 → 对应飞书交互组件
  ↓
组装完整卡片 JSON → RenderedOutput { msg_type: "interactive" | "text", payload }
```

## 模块关系

- **上游**：渲染层框架（提供 ContentBlock[] 和 DSL 解析结果）
- **下游**：飞书 IM Adapter（接收卡片 JSON 并调用飞书发送接口）
- **同层**：其他平台渲染器（共享渲染器接口，各自实现平台格式）
