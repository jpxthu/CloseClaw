# 飞书插件

## 概述

飞书插件是 IMPlugin trait 的飞书平台实现。它封装了飞书平台的协议适配（Adapter）和格式渲染（Renderer），将飞书 webhook 事件转为统一中间结构，并将 LLM 结构化输出渲染为飞书消息格式后发送。

## 架构

飞书插件内部由两个协作组件构成：Adapter 负责飞书 API 通信，Renderer 负责格式转换。两部分通过 IMPlugin trait 对外暴露统一接口，Gateway 无须感知内部拆分。

### Adapter

飞书 Adapter 管理飞书 Open API 的通信。包括三个职责：

**Webhook 解析**：解析飞书 webhook 事件 payload，提取 sender_id、chat_id、消息内容等字段，产出 NormalizedMessage。text 类型消息提取 content.text 字段，post 类型消息展开 title 和 content blocks 为纯文本（图片 segment 输出占位符），非文本消息（图片、文件、语音等）不产 NormalizedMessage。

**Token 管理**：通过飞书 tenant_access_token 接口获取凭证，内部缓存并提前刷新。Token 有效期约 2 小时，提前 5 分钟触发主动刷新。

**消息发送**：接收 RenderedOutput，按 msg_type 选择发送路径——text 类型走飞书文本消息接口，interactive 类型走飞书卡片消息接口。卡片消息发送后返回 message_id，供后续更新使用。消息更新通过 PATCH 接口覆盖已有卡片内容。

### Renderer

飞书 Renderer 将 LLM 的结构化内容块（ContentBlock[]）和 DSL 解析结果渲染为飞书消息格式。渲染分两步完成：输出类型决策和卡片组装。

**步骤一：输出类型决策**

| 条件 | 输出 |
|------|------|
| 纯文本，无格式标记、无换行、无 DSL | text 消息 |
| 含标题标记（`#` 开头） | interactive 卡片 |
| 含粗体、斜体、代码块、列表、引用、链接 | interactive 卡片 |
| 含换行符 | interactive 卡片 |
| 含 DSL 交互指令 | interactive 卡片 |
| 含多个 ContentBlock 或 Thinking/Tool 块 | interactive 卡片 |

**步骤二：卡片组装**

```
ContentBlock[] + DSL 指令
  ↓
header 提取 — 首行 # 标题作为卡片标题（蓝色模板）
  ↓
body 渲染
  ├── Text 块 → markdown 元素（飞书原生 markdown 渲染）
  ├── Thinking 块 → 平台支持时渲染为折叠推理区块
  ├── ToolUse 块 → 工具调用描述卡片
  └── ToolResult 块 → 工具结果内容块
  ↓
interactive 元素注入
  └── DSL 指令 → 飞书 button 等交互组件
  ↓
飞书卡片 JSON
```

### Markdown 元素映射

| Markdown | 飞书卡片元素 |
|---------|-----------|
| `# 标题` | header.title（蓝色模板），标题行不进入 body |
| `## 标题` 及以下 | markdown 元素，原生渲染 |
| `**粗体**` / `*斜体*` | 飞书原生 markdown 渲染 |
| `` `行内代码` `` | 飞书 markdown 行内代码 |
| ` ```lang\n代码块\n``` ` | 飞书 markdown 代码块（平台自行高亮） |
| `> 引用` | 飞书 markdown 引用块 |
| `- 列表` / `1. 列表` | 飞书 markdown 列表 |
| `[链接](url)` | 飞书 markdown 链接 |
| `---` | hr 分割线元素 |

### DSL 按钮渲染

按钮指令由 Processor Chain 中的 DslParser 在链阶段解析，Renderer 从 metadata 读取。按钮渲染规则：

- 首个按钮 → primary 样式
- 后续按钮 → default 样式
- 所有按钮平铺在单个 action 元素中

## 数据流

### 入站路径

```
飞书 webhook
  ↓
[Adapter.handle_webhook]
  解析 FeishuEvent JSON → 提取 sender_id / chat_id / content
  → NormalizedMessage { platform: "feishu", sender_id, peer_id: chat_id, content, ... }
  ↓
Processor Chain 入站处理
```

### 出站路径

```
Processor Chain 出站产出 ProcessedMessage
  ↓
Gateway 选择飞书插件 → 调用插件内部 Renderer
  ↓
[Renderer.render]
  ├── 遍历 ContentBlock[]：
  │     ├── Text 块 → 解析 markdown，映射为飞书 card element
  │     ├── Thinking 块 → 平台支持时渲染为折叠区
  │     ├── ToolUse 块 → 渲染为工具调用信息
  │     └── ToolResult 块 → 渲染为结果内容块
  ├── DSL 指令 → 渲染为 action button
  └── 输出决策 → RenderedOutput { msg_type, payload }
  ↓
[Adapter.send]
  msg_type="text" → POST /im/v1/messages（纯文本）
  msg_type="interactive" → POST /im/v1/messages（卡片 JSON）
  ↓
飞书用户收到消息
```

## 模块关系

- **上游**：Gateway（调用插件进行入站解析和出站渲染发送）、Processor Chain（消费 Adapter 产出的 NormalizedMessage、为 Renderer 提供 DSL 解析结果）
- **下游**：飞书 Open API（插件直接与外部平台通信）
- **所属**：IM Adapter 模块的平台插件
- **无关**：CLI 和 Discord 等其他平台插件（各自独立实现 IMPlugin trait，无直接依赖）
