# 飞书插件

## 概述

飞书插件是 IMPlugin trait 的飞书平台实现。它封装飞书平台的协议适配和格式渲染，将飞书 webhook 事件转为统一中间结构，并将 LLM 结构化输出渲染为飞书消息格式后发送。

## 架构

飞书插件内部由 Adapter（协议通信）和 Renderer（格式转换）组成，通过 IMPlugin trait 对外暴露统一接口。飞书插件作为独立平台插件按需启用——仅在配置文件中显式配置并启用飞书平台时才加载，未配置则不加载。

### Adapter

**Webhook 解析**：解析飞书 webhook 事件 payload，提取发送者 ID、会话 ID、消息正文等字段，产出 NormalizedMessage。

- text 类型消息：提取 `content.text` 字段作为消息正文
- post 类型消息：展开 title 和 content blocks 为文本，保留标题、段落、有序/无序列表（含多级嵌套）、文本样式（粗体/删除线/下划线及组合）、@提及（渲染为 `@用户名`）、引用块、内嵌媒体占位符 `[图片]`
- 表情消息：消息中的表情符号渲染为文本占位符形式（如 `[OK]`、`[赞]`）
- 话题消息：提取 `thread_id`、`parent_id`、`root_id`，按 `thread_id > root_id > parent_id` 优先级合并为一个 `thread_id` 值。该值不参与 session 路由，仅用于出站时作为飞书 API 的 `root_id` 参数定向回复到原话题
- 非文本消息（图片、文件、语音等）：产出 NormalizedMessage（message_type 标记类型，media_refs 存储引用，content 可为空），交由下游 Gateway 统一处理。图片/文件/语音的语义理解暂未设计
- account_id：以 sender_id 为键查询账户绑定表（accounts.json）获取 CloseClaw 本地账户 ID，参与 session 路由。一个账户可绑定多个平台的 sender_id
- 引用/回复消息：提取被引用消息的文本内容，截断至 500 字符（超出追加 `...`），以 markdown blockquote 格式拼接在正文前

**事件区分**：Adapter 解析阶段须区分消息事件与交互事件——消息事件转为 NormalizedMessage，交互事件不经过 NormalizedMessage 入站通路：

- `card-action-trigger`（卡片按钮/选择器点击）：属于工具调用的回执，走 tool_result 通道注入对话（见 [common 共享类型](../../common/shared-types.md) 的卡片交互事件建模边界）
- `reaction.created`（表情回应）：感知用户对消息的表情回应并记录日志（平台、消息、表情），作为 Agent 可观测的交互信号
- `bot.added`（机器人加入群聊）：识别机器人入群事件并记录日志；该群聊会话由后续消息到达时按群聊粒度路由创建

**凭证管理**：飞书凭证（token 等）由飞书插件自行管理与刷新，不跨模块传递，不进入任何日志（日志脱敏遵循 [debug_log 框架](../../debug_log/README.md)）。

**消息发送**：接收 RenderedOutput，按 msg_type 选择发送路径——text 类型走飞书文本消息接口，interactive 类型走飞书卡片消息接口。发送目标由 Gateway 传入的 (peer_id, thread_id?) 确定。发送失败时记录日志并降级，不导致进程崩溃，Agent 继续运行。

### Renderer

将 LLM 的结构化内容块（ContentBlock[]）和 DSL 解析结果渲染为飞书消息格式。渲染分两步：输出类型决策和卡片组装。

**输出类型决策**：

| 条件 | 输出 |
|------|------|
| 纯文本，无格式标记、无换行、无 DSL | text 消息 |
| 含标题标记（`#`/`##` 等任意级别） | interactive 卡片 |
| 含粗体、斜体、代码块、列表、引用、链接、分割线 | interactive 卡片 |
| 含换行符 | interactive 卡片 |
| 含 DSL 交互指令（批量模式） | interactive 卡片 |
| 含 Thinking/ToolUse/ToolResult 块（单个或多个） | interactive 卡片 |

**卡片组装**：

```
ContentBlock[] + DSL 指令 → 卡片组装：
1. header 提取 — 首行 # 标题作为卡片标题（蓝色模板）
2. body 渲染（并行处理各块类型）：
   - Text 块 → markdown 元素（飞书原生 markdown 渲染）
   - Thinking 块 → 平台支持时渲染为折叠推理区块
   - ToolUse 块 → 工具调用描述卡片
   - ToolResult 块 → 工具结果内容块
   - Image 块 → 飞书图片元素（url 为访问地址）
   - Audio 块 → 飞书音频元素（url 为访问地址）
   - File 块 → 飞书文件元素（url 为访问地址）
3. interactive 元素注入 — DSL 指令 → button/selector 渲染为飞书交互组件（见下方 DSL 渲染）
4. 产出飞书卡片 JSON
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

### DSL 交互元素渲染

DSL 指令（button / selector）由 Processor Chain 的 DslParser 解析后传入 Renderer，渲染为飞书交互组件：

- **button**：首个按钮为 primary 样式，后续按钮为 default 样式，所有按钮平铺在单个 action 元素中
- **selector**：渲染为飞书原生下拉选择组件；平台能力不足时降级为「每个选项一个按钮」

批量模式下 DslParser 解析 DSL 指令后传入 Renderer 渲染交互元素。流式模式下 DSL 指令不产生渲染输出（仅用于日志记录和出站历史写入），交互元素仅在批量模式渲染（详见 [Gateway 出站路径](../../gateway/README.md#出站路径)）。

## 数据流

### 入站路径

```
飞书 webhook
  ↓
[Adapter] 解析 FeishuEvent → NormalizedMessage { platform: "feishu", sender_id, peer_id, account_id, content, thread_id?, message_type, media_refs, ... }
  ↓
Processor Chain 入站处理
```

### 出站路径

```
Processor Chain 出站产出 [ProcessedMessage](../../common/shared-types.md#processedmessage)
  ↓
Gateway 选择飞书插件
  ↓
[Renderer] 遍历 ContentBlock[] + DSL 指令 → 输出决策 → RenderedOutput { msg_type, payload }
  ↓
[Adapter] msg_type="text" → 飞书文本消息接口 / msg_type="interactive" → 飞书卡片消息接口
  ↓
发送到目标 (peer_id, thread_id?)
```

**调试日志**：飞书插件在以下环节记录调试日志（经 [debug_log 框架](../../debug_log/README.md)）：入站解析（平台、消息类型、解析耗时）、出站渲染（平台、渲染耗时）、平台 API 发送（平台、目标、耗时）。

### 对外工具

飞书插件通过 IM Adapter 模块的工具注册入口注册以下工具分组到 ToolRegistry：

- **feishu_im**：飞书 IM 消息操作（发送、撤回、编辑、表情回应等）
- **feishu_calendar**：飞书日历管理
- **feishu_task**：飞书任务管理
- **feishu_bitable**：飞书多维表格操作
- **feishu_doc**：飞书文档操作
- **feishu_drive**：飞书云盘操作
- **feishu_sheet**：飞书电子表格操作

全部飞书工具默认延迟加载，首次调用时才初始化。各工具分组的详细参数见 [tools 模块文档](../../tools/README.md)。

## 模块关系

- **互相调用**：Gateway——入站方向插件解析 webhook 产出 NormalizedMessage 交给 Gateway；出站方向 Gateway 选择插件调用渲染和发送
- **所属**：IM Adapter 模块的平台插件
- **无关**：其他平台插件（各自独立实现 IMPlugin trait）
