# IM Adapter

## 概述

IM Adapter 模块提供跨消息平台的插件化适配框架。每个消息平台封装为一个独立插件，包含协议适配（Adapter）和格式渲染（Renderer）两部分。Gateway 按平台选择对应插件，不关心插件内部实现。

## 架构

### 插件体系

IM Adapter 模块不包含业务逻辑，由三层组成：

- **插件接口层**：定义 IMPlugin trait，统一插件契约。每个消息平台实现此 trait，提供入站解析、格式渲染、消息发送、生命周期管理四组方法。terminal 平台的实现位于 [CLI 模块](../cli/README.md)，不在此目录。
- **通用渲染能力**：代码块语法高亮和流式增量渲染是跨平台通用机制，作为 IMPlugin trait 的默认实现提供。各平台插件自动继承，按需覆盖平台差异化部分。
- **平台插件**：每个消息平台的数据和渲染实现。IM 平台（飞书、Discord 等）的插件放在 `platforms/` 子目录下。terminal 平台的实现位于 CLI 模块。

模块运行时注册表由 Gateway 维护：

- **Plugin Registry**：platform → IMPlugin 的映射。Gateway 通过 platform 字段选择插件。
- **插件注册机制**：编译期自动扫描 `platforms/` 目录发现所有平台插件并注册。不在 `platforms/` 下的插件（如 CLI 模块的 terminal）通过显式注册加入 Plugin Registry。启动时仅加载配置文件中显式启用的平台。新增 IM 平台 = 新增目录 + 实现 trait，Gateway 代码无需改动。新平台默认禁用，需在配置文件中显式添加并启用方可使用。

平台插件为自包含模块，内部结构统一：

```
platforms/<平台名>/
├── mod.rs         — 插件注册，impl IMPlugin trait
├── adapter.rs     — 入站：webhook 解析 → NormalizedMessage
│                  — 出站：API 调用发送消息
│                  — token 管理与刷新
├── renderer.rs    — ContentBlock[] + DSL → 平台原生格式

└── tools/         — 平台工具注册
    ├── mod.rs     — register_tools() 入口
    └── ...        — 各工具分组文件
```

各文件职责单一、无循环依赖。新增平台时按此布局创建目录即可。

### 对外工具

IM Adapter 模块通过工具注册入口向 ToolRegistry 注册平台插件工具，由 tools 模块在启动编排时调用。当前飞书平台注册以下工具分组：feishu_im、feishu_calendar、feishu_task、feishu_bitable、feishu_doc、feishu_drive、feishu_sheet。全部飞书工具默认延迟加载。工具详情见 [飞书插件](platforms/feishu.md)，各工具分组详细参数见 [tools 模块文档](../tools/README.md)。

```
im_adapter/
├── README.md               ← 本文件（插件架构+通用能力索引）
├── code-render.md           ← 代码块语法高亮（平台无关）
├── streaming-render.md      ← 流式增量渲染（平台无关）
└── platforms/
    └── feishu.md            ← 飞书插件
```

### IMPlugin trait 契约

每个消息平台插件实现统一接口，包含以下方法分组：

**入站**：解析 webhook payload 为 NormalizedMessage。消息过滤在解析阶段完成——空内容消息在解析阶段丢弃，不产 NormalizedMessage。非文本消息（image/file/audio）正常产 NormalizedMessage（message_type 标记类型，media_refs 存储引用，content 可为空），由下游 Gateway 统一处理。

**渲染**：接收 ContentBlock[]（定义见 [common ContentBlock](../common/shared-types.md#contentblock)）和 DSL 解析结果，按平台能力选择输出格式（纯文本或富格式）。渲染是纯数据转换，无副作用。

**发送**：接收 RenderedOutput，封装为平台请求格式，以指定目标（peer_id + thread_id）调用平台发送 API。

**生命周期**：

| 方法 | 说明 |
|------|------|
| `init()` | 启动时初始化（连接池、token 等）。不需要的插件空实现 |
| `shutdown()` | 关闭时清理资源。不需要的插件空实现 |

渲染和发送拆为两步：渲染结果是数据，发送是副作用。Gateway 在两步之间可插入审计、频率限制等中间件。

NormalizedMessage 是插件产出的统一中间结构，屏蔽各平台差异。完整字段定义及身份映射规则见 [common 共享类型](../common/shared-types.md)。

IM Adapter 负责在入站解析时填充 NormalizedMessage 的全部字段——各平台插件将原生格式转为统一结构，Processor Chain 和 Gateway 下游消费时不感知平台差异。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [代码块渲染](code-render.md) | 代码块语法高亮，按平台选择渲染策略 |
| [流式渲染](streaming-render.md) | 流式增量输出，行缓冲 + 块类型路由 |
| [飞书插件](platforms/feishu.md) | 飞书平台完整插件实现 |

### 平台渲染选择

各消息平台插件根据内容特征自动选择输出格式：

- 纯文本、无格式标记、无 DSL → text 消息
- 含 markdown 格式（标题/粗体/斜体/代码块/列表/引用/链接/分割线）或换行或 DSL → 富格式消息
- 含 Thinking/ToolUse/ToolResult 块 → 富格式消息

## 数据流

### 入站路径

```
IM 平台 webhook
  ↓
[IMPlugin 入站]
  平台格式解析 → NormalizedMessage { platform, sender_id, peer_id, content, ... }
  ↓
Processor Chain 入站
  RawLog → SessionRouter → ContentNormalizer
  ↓
ProcessedMessage → Gateway 路由决策
```

### 出站路径

```
LLM 输出 ContentBlock[]
  ↓
Processor Chain 出站（DslParser）
  ↓
ProcessedMessage { content_blocks, metadata[dsl_result] }
  ↓
Gateway 记录出站日志
  ↓
[IMPlugin 渲染]
  Renderer.render(content_blocks, dsl_result)
  ↓
  RenderedOutput { msg_type, payload }
  ↓
[中间件插入点] — Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件
  ↓
[IMPlugin 发送]
  Adapter.send(payload, peer_id, thread_id)
  ↓
  IM 平台
```

出站路径中，Renderer 不属 Processor Chain——渲染是终结操作，输出后无后续处理器接力。Gateway 根据目标 platform 选择对应插件，调用插件内部的 Renderer 完成渲染，再通过 Adapter 发送。

peer_id 和 thread_id 来源于入站时 IM Adapter 填入 NormalizedMessage 的对应字段，经 Session 上下文存储后在出站时取出，由 Gateway 传递给 IMPlugin 的发送方法。

## 模块关系

- **上游**：Gateway（出站方向：调用 IM Adapter 完成渲染和发送）、Config（accounts.json：入站解析时查询身份映射表，将 sender_id 转为 account_id）
- **下游**：Processor Chain（入站方向：消费 IM Adapter 产出的 NormalizedMessage）
- **无关**：Session（IMPlugin 不参与 session 生命周期管理）、LLM Provider（IMPlugin 不调用 LLM）、Slash Command（IMPlugin 不参与指令解析）
