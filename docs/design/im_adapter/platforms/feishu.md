# 飞书插件

## 概述

飞书插件是 IMPlugin trait 的飞书平台实现。它封装飞书平台的协议适配和格式渲染，将飞书 webhook 事件转为统一中间结构，并将 LLM 结构化输出渲染为飞书消息格式后发送。

## 架构

飞书插件内部由 Adapter（协议通信）和 Renderer（格式转换）组成，通过 IMPlugin trait 对外暴露统一接口。

### Adapter

**Webhook 解析**：解析飞书 webhook 事件 payload，提取发送者 ID、会话 ID、消息正文等字段，产出 NormalizedMessage。

- text 类型消息：提取 `content.text` 字段作为消息正文
- post 类型消息：展开 title 和 content blocks 为文本（含有序/无序列表、文本样式、图片占位符等）
- 话题消息：提取 `thread_id`、`parent_id`、`root_id` 字段，用于出站定向回复
- 非文本消息（图片、文件、语音等）：不产 NormalizedMessage

**消息发送**：接收 RenderedOutput，按 msg_type 选择发送路径——text 类型走飞书文本消息接口，interactive 类型走飞书卡片消息接口。发送目标由 Gateway 传入的 (peer_id, thread_id) 确定。

### Renderer

将 LLM 的结构化内容块（ContentBlock[]）和 DSL 解析结果渲染为飞书消息格式。渲染分两步：输出类型决策和卡片组装。

**输出类型决策**：

| 条件 | 输出 |
|------|------|
| 纯文本，无格式标记、无换行、无 DSL | text 消息 |
| 含标题标记（`#` 开头） | interactive 卡片 |
| 含粗体、斜体、代码块、列表、引用、链接 | interactive 卡片 |
| 含换行符 | interactive 卡片 |
| 含 DSL 交互指令 | interactive 卡片 |
| 含多个 ContentBlock 或 Thinking/Tool 块 | interactive 卡片 |

**卡片组装**：

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

按钮指令由 Processor Chain 的 DslParser 解析后传入 Renderer。按钮渲染规则：首个按钮为 primary 样式，后续按钮为 default 样式，所有按钮平铺在单个 action 元素中。

### 代码文件

飞书插件代码位于 `src/im_adapter/platforms/feishu/`，各文件职责如下：

```
platforms/feishu/
├── mod.rs         — FeishuPlugin 结构体， impl IMPlugin trait
├── adapter.rs     — 入站：飞书 webhook 解析 → NormalizedMessage
│                  — 出站：按 msg_type 选择发送路径（text 消息 / card 消息）
│                  — token 管理与自动刷新
├── renderer.rs    — ContentBlock[] + DSL → 飞书卡片 JSON / 飞书文本
│                  — 输出类型决策（text vs card）
│                  — markdown → 飞书元素映射（标题/粗体/代码块/列表/引用/链接/分割线）
│                  — header 提取 + body 渲染
│                  — DSL 按钮注入
├── cleaner.rs     — 飞书平台消息清洗，实现 clean_content()
│                  — 移除 @ 提及语法
│                  — 移除 `<at>xxx</at>` 等飞书专属标记
│                  — ContentNormalizer 调用此回调完成平台残留清洗
└── tools/         — 飞书工具注册
    ├── mod.rs     — register_tools() 入口，注册所有飞书工具分组
    └── ...        — 各工具分组实现文件（im/calendar/task/bitable/doc/drive/sheet）
```

新增飞书工具分组时，在 `tools/` 下新增文件并注册。

## 数据流

### 入站路径

```
飞书 webhook
  ↓
[Adapter] 解析 FeishuEvent → NormalizedMessage { platform: "feishu", sender_id, peer_id, content, thread_id? }
  ↓
Processor Chain 入站处理
```

### 出站路径

```
Processor Chain 出站产出 ProcessedMessage
  ↓
Gateway 选择飞书插件
  ↓
[Renderer] 遍历 ContentBlock[] + DSL 指令 → 输出决策 → RenderedOutput { msg_type, payload }
  ↓
[Adapter] msg_type="text" → 飞书文本消息接口 / msg_type="interactive" → 飞书卡片消息接口
  ↓
发送到目标 (chat_id, thread_id?)
```

### 对外工具

飞书插件通过 IM Adapter 模块的工具注册入口注册以下工具分组到 ToolRegistry：

- **feishu_im**：飞书 IM 消息操作（发送、撤回、编辑、表情回应等）
- **feishu_calendar**：飞书日历管理
- **feishu_task**：飞书任务管理
- **feishu_bitable**：飞书多维表格操作
- **feishu_doc**：飞书文档操作
- **feishu_drive**：飞书云盘操作
- **feishu_sheet**：飞书电子表格操作

全部飞书工具默认延迟加载，首次调用时才初始化。各工具分组的详细参数见 tools 模块相关文档。

## 模块关系

- **互相调用**：Gateway——入站方向插件解析 webhook 产出 NormalizedMessage 交给 Gateway；出站方向 Gateway 选择插件调用渲染和发送
- **所属**：IM Adapter 模块的平台插件
- **无关**：其他平台插件（各自独立实现 IMPlugin trait）
