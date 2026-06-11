# IM Adapter

## 概述

IM Adapter 模块提供跨消息渠道的插件化适配框架。每个消息渠道封装为一个独立插件，包含协议适配（Adapter）和格式渲染（Renderer）两部分。Gateway 按渠道选择对应插件，不关心插件内部实现。

## 架构

### 插件体系

IM Adapter 模块不包含业务逻辑，由三层组成：

- **插件接口层**：定义 IMPlugin trait，统一插件契约。每个消息渠道实现此 trait，提供入站解析、格式渲染、消息发送、生命周期管理、平台清洗五组方法。terminal 渠道的实现位于 [CLI 模块](../cli/README.md)，不在此目录。
- **通用渲染能力**：代码块语法高亮和流式增量渲染是跨平台通用机制，作为 IMPlugin trait 的默认实现提供。各平台插件自动继承，按需覆盖平台差异化部分。
- **平台插件**：每个消息渠道的数据和渲染实现。IM 渠道（飞书、Discord 等）的插件放在 `platforms/` 子目录下。terminal 渠道的实现位于 CLI 模块。

模块运行时注册表由 Gateway 维护：

- **Plugin Registry**：platform → IMPlugin 的映射。Gateway 通过 platform 字段选择插件。
- **插件注册机制**：Gateway 启动时自动扫描 `platforms/` 目录，发现所有实现了 IMPlugin trait 的插件并加载。不在 `platforms/` 下的插件（如 CLI 模块的 terminal）通过显式注册加入 Plugin Registry。用户可通过配置文件控制各平台的启用/禁用。新增 IM 平台 = 新增目录 + 实现 trait，Gateway 代码和配置均无需改动。

平台插件为自包含模块，内部结构统一：

```
platforms/<平台名>/
├── mod.rs         — 插件注册，impl IMPlugin trait
├── adapter.rs     — 入站：webhook 解析 → NormalizedMessage
│                  — 出站：API 调用发送消息
│                  — token 管理与刷新
├── renderer.rs    — ContentBlock[] + DSL → 平台原生格式
├── cleaner.rs     — 平台消息清洗（@ 语法、mention 等），实现 clean_content() 回调
└── tools/         — 平台工具注册
    ├── mod.rs     — register_tools() 入口
    └── ...        — 各工具分组文件
```

各文件职责单一、无循环依赖。新增平台时按此布局创建目录即可。

### 对外工具

IM Adapter 模块通过工具注册入口向 ToolRegistry 注册平台插件工具，由 tools 模块在启动编排时调用。当前飞书平台注册以下工具分组：feishu_im、feishu_calendar、feishu_task、feishu_bitable、feishu_doc、feishu_drive、feishu_sheet。全部飞书工具默认延迟加载。工具详情见 [飞书插件](platforms/feishu.md)，各工具分组详细参数见 tools 模块文档。

```
im_adapter/
├── README.md               ← 本文件（插件架构+通用能力索引）
├── code-render.md           ← 代码块语法高亮（平台无关）
├── streaming-render.md      ← 流式增量渲染（平台无关）
└── platforms/
    └── feishu.md            ← 飞书插件
```

### IMPlugin trait 契约

每个消息渠道插件实现统一接口，包含以下方法分组：

**入站**：解析 webhook payload 为 NormalizedMessage。消息过滤（空内容、非文本消息）在解析阶段完成——Adapter 对不支持的消息类型不产 NormalizedMessage。

**清洗**：`clean_content(raw: &str) -> String`，接收平台原生文本，移除平台专属标记（飞书 @ 语法、Discord mention 等），产出清洗后的纯文本。由 Processor Chain 的 ContentNormalizer 调用。各平台按需实现，不需要的平台空实现透传。

**渲染**：接收 ContentBlock[] 和 DSL 解析结果，按平台能力选择输出格式（纯文本或富格式）。渲染是纯数据转换，无副作用。

**发送**：接收 RenderedOutput，封装为平台请求格式，以指定目标（peer_id + thread_id）调用平台发送 API。

**生命周期**：

| 方法 | 说明 |
|------|------|
| `init()` | 启动时初始化（连接池、token 等）。不需要的插件空实现 |
| `shutdown()` | 关闭时清理资源。不需要的插件空实现 |

渲染和发送拆为两步：渲染结果是数据，发送是副作用。Gateway 在两步之间可插入审计、频率限制等中间件。

NormalizedMessage 是插件产出的统一中间结构，屏蔽各平台差异：

| 字段 | 说明 |
|------|------|
| `platform` | 平台标识，如 `"feishu"` |
| `sender_id` | 发送者的平台内 ID |
| `peer_id` | 会话对端（群聊 chat_id 或私聊对方 ID） |
| `thread_id` | 话题 ID，可选。不参与 session key 计算，仅用于出站定向回复 |
| `account_id` | CloseClaw 本地账号标识，由 sender_id 通过身份映射得到。参与 session 路由 |
| `content` | 消息文本内容 |
| `message_type` | 消息类型（text / image / file / audio）。非文本类型时 content 可为空 |
| `media_refs` | 图片/文件/音频的引用列表（key + URL）。由 Adapter 负责下载到本地临时路径 |
| `quoted_message` | 被引用的消息内容，可选。最多嵌套一层 |
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

- **互相调用**：Processor Chain——入站方向 IM Adapter 产出 NormalizedMessage 供 Processor Chain 消费；出站方向 Processor Chain 产出 ProcessedMessage 后，IM Adapter 渲染并发送
- **互相调用**：Gateway——入站方向 IM Adapter 解析 webhook 产出 NormalizedMessage，经 Processor Chain 处理后传入 Gateway 路由决策；出站方向 Gateway 选择对应平台插件调用渲染和发送
- **下游**：无——渠道插件是链路终点，负责与外部平台通信
- **无关**：Session（IMPlugin 不参与 session 生命周期管理）、LLM Provider（IMPlugin 不调用 LLM）、Slash Command（IMPlugin 不参与指令解析）
