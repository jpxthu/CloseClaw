# IM Adapter

## 概述

IM Adapter 模块提供跨消息渠道的插件化适配框架。每个消息渠道封装为一个独立插件，包含协议适配（Adapter）和格式渲染（Renderer）两部分。Gateway 按渠道选择对应插件，不关心插件内部实现。

## 架构

### 插件体系

IM Adapter 模块不包含业务逻辑，由三层组成：

- **插件接口层**：定义 IMPlugin trait，统一插件契约。每个消息渠道实现此 trait，提供 platform 标识、入站消息解析、出站消息发送和格式渲染四项能力。terminal 渠道的实现位于 [CLI 模块](../cli/README.md)，不在此目录。
- **通用渲染能力**：代码块语法高亮和流式增量渲染是跨平台通用机制，由模块顶层提供，各平台插件直接复用。
- **平台插件**：每个消息渠道的数据和渲染实现。IM 渠道（飞书、Discord 等）的插件放在 `platforms/` 子目录下。每个插件内部包含——Adapter（webhook 解析、token 管理、API 调用）和 Renderer（ContentBlock[] → 平台原生格式）。terminal 渠道的实现位于 CLI 模块。

模块运行时注册表由 Gateway 维护：

- **Plugin Registry**：platform → IMPlugin 的映射。Gateway 通过 platform 字段选择插件。

### 对外工具

IM Adapter 模块暴露 `register_tools(registry)` 方法，由 tools 模块在启动编排时调用，将平台插件工具注册到 ToolRegistry。当前飞书平台注册以下工具分组：feishu_im、feishu_calendar、feishu_task、feishu_bitable、feishu_doc、feishu_drive、feishu_sheet。全部飞书工具默认延迟加载。工具详情和参数见 [platforms/feishu.md](platforms/feishu.md)。

```
im_adapter/
├── README.md               ← 本文件（插件架构+通用能力索引）
├── code-render.md           ← 代码块语法高亮（平台无关）
├── streaming-render.md      ← 流式增量渲染（平台无关）
└── platforms/
    └── feishu.md            ← 飞书插件
```

### IMPlugin trait 契约

每个消息渠道插件实现统一接口，包含三组方法：

**入站**：解析 webhook payload 为 NormalizedMessage。消息过滤（空内容、非文本消息）在解析阶段完成——Adapter 对不支持的消息类型不产 NormalizedMessage。

**出站**：接收 RenderedOutput，根据 msg_type 分发到平台发送 API。渲染后的 payload 由 Adapter 封装为平台请求格式并发送。

**渲染**：接收 ContentBlock[] 和 DSL 解析结果，按平台能力选择输出格式（纯文本或富格式）。渲染由平台 Renderer 完成，实际调用封装在插件内部。

NormalizedMessage 是插件产出的统一中间结构，屏蔽各平台差异：

| 字段 | 说明 |
|------|------|
| `platform` | 平台标识，如 `"feishu"` |
| `sender_id` | 发送者的平台内 ID |
| `peer_id` | 会话对端（群聊 chat_id 或私聊对方 ID） |
| `thread_id` | 话题 ID，可选。不参与 session key 计算，仅用于出站定向回复 |
| `account_id` | CloseClaw 本地账号标识，由 sender_id 通过身份映射得到。参与 session 路由 |
| `content` | 消息文本内容 |
| `timestamp` | 消息发送时间 |

### 子功能索引

| 文档 | 内容 |
|------|------|
| [代码块渲染](code-render.md) | 代码块语法高亮，按平台选择渲染策略 |
| [流式渲染](streaming-render.md) | 流式增量输出，行缓冲 + 块类型路由 |
| [飞书插件](platforms/feishu.md) | 飞书平台完整插件实现 |

### 平台渲染选择

各消息渠道插件根据内容特征自动选择输出格式：

- 纯文本、无格式标记、无 DSL → text 消息
- 含 markdown 格式（标题/粗体/斜体/代码块/列表/引用/链接/分割线）或换行或 DSL → 富格式消息
- 含 Thinking/ToolUse/ToolResult 块 → 富格式消息

## 数据流

### 入站路径

```
IM 渠道 webhook
  ↓
[IMPlugin 入站]
  平台格式解析 → NormalizedMessage { platform, sender_id, peer_id, content, ... }
  ↓
Processor Chain 入站（RawLog → SessionRouter → ContentNormalizer）
  ↓
ProcessedMessage → Gateway 路由决策
```

### 出站路径

```
LLM 输出 ContentBlock[]
  ↓
Processor Chain 出站（DslParser → RawLog）
  ↓
ProcessedMessage { content_blocks, metadata[dsl_result] }
  ↓
[IMPlugin 渲染]
  Renderer.render(content_blocks, dsl_result) → RenderedOutput { msg_type, payload }
  ↓
[IMPlugin 发送]
  Adapter.send(payload, peer_id, thread_id) → IM 渠道
```

出站路径中，Renderer 不属 Processor Chain——渲染是终结操作，输出后无后续处理器接力。Gateway 根据目标 platform 选择对应插件，调用插件内部的 Renderer 完成渲染，再通过 Adapter 发送。

## 模块关系

- **上游**：Gateway（选择插件并调用渲染和发送）、Processor Chain（消费 NormalizedMessage 入站、消费 ContentBlock[] 出站）
- **下游**：无——渠道插件是链路终点，负责与外部平台通信
- **无关**：Session（IMPlugin 不参与 session 生命周期管理）、LLM Provider（IMPlugin 不调用 LLM）、Slash Command（IMPlugin 不参与指令解析）
