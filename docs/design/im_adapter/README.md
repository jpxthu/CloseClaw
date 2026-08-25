# IM Adapter

## 概述

- 关联需求文档：[requirements/im_adapter.md](../../requirements/im_adapter.md)
- IM Adapter 模块提供跨消息平台的插件化适配框架。每个消息平台封装为一个独立插件，包含协议适配（Adapter）和格式渲染（Renderer）两部分。Gateway 按平台选择对应插件，不关心插件内部实现。

## 架构

### 插件体系

IM Adapter 模块不包含业务逻辑，由三层组成：

- **插件接口层**：IMPlugin trait 是统一插件契约，完整接口定义见 [common/core-traits](../common/core-traits.md#implugin)。每个消息平台实现此 trait，提供入站解析、渲染、发送、生命周期四组方法。terminal 平台的实现位于 [CLI 模块](../cli/README.md)，不在此目录。
- **通用渲染能力**：代码块语法高亮和流式增量渲染是跨平台通用机制，以 IM Adapter 内的通用组件形式提供。各平台插件组合持有对应组件并在渲染时委托调用，按需覆盖平台差异化部分。
- **平台插件**：每个消息平台的数据和渲染实现。IM 平台（飞书、Discord 等）的插件放在 `platforms/` 子目录下。terminal 平台的实现位于 CLI 模块。

模块运行时注册表由 Gateway 维护：

- **Plugin Registry**：platform → IMPlugin 的映射。Gateway 通过 platform 字段选择插件。
- **插件注册机制**：分两阶段。① **编译期发现**——构建时自动扫描 `platforms/` 目录，为每个平台生成模块声明，新增平台无需改动 Gateway 等核心代码、重启即自动生效。② **运行时注册**——系统启动时遍历已发现的平台插件，读取配置中该平台的启用状态，仅对已启用平台执行注册，写入 Gateway 维护的 Plugin Registry。不在 `platforms/` 下的插件（如 CLI 模块的 terminal）通过显式注册加入 Plugin Registry。各已启用平台的初始化相互独立——单个插件初始化失败仅记录日志并跳过该平台，不影响其他已启用平台的正常加载与运行。新平台默认禁用，需在配置中显式添加并启用方可使用。

平台插件为自包含模块，内部结构统一：

```
platforms/<平台名>/
├── mod.rs         — 插件注册（实现 IMPlugin trait）
├── adapter.rs     — 入站：webhook 解析 → NormalizedMessage
│                  — 出站：API 调用发送消息
│                  — token 管理与刷新
├── renderer.rs    — ContentBlock[] + DSL → 平台原生格式

└── tools/         — 平台工具注册
    ├── mod.rs     — 工具注册入口
    └── ...        — 各工具分组文件
```

各文件职责单一、无循环依赖。新增平台时按此布局创建目录即可。

### 对外工具

IM Adapter 模块通过 [ToolRegistrar](../tools/tool-registrar.md) trait 向 ToolRegistry 注册平台插件工具。飞书平台注册的工具分组见 [飞书插件](platforms/feishu.md)，各工具分组详细参数见 [tools 模块文档](../tools/README.md)。全部飞书工具默认延迟加载。

```
im_adapter/
├── README.md               ← 本文件（插件架构+通用能力索引）
├── code-render.md           ← 代码块语法高亮（平台无关）
├── streaming-render.md      ← 流式增量渲染（平台无关）
└── platforms/
    └── feishu.md            ← 飞书插件
```

### IMPlugin trait 契约

IMPlugin trait 的完整接口契约定义见 [common/core-traits](../common/core-traits.md#implugin)。本文档聚焦 IM Adapter 模块对插件实现和注册编排的具体职责。

每个消息平台插件实现 IMPlugin trait，包含入站解析、渲染、发送、生命周期四组方法。渲染和发送拆为两步——渲染结果是数据，发送是副作用——Gateway 在两步之间可插入审计、频率限制等中间件（详见 [Gateway 出站中间件](../gateway/outbound-flow.md#出站中间件)）。

NormalizedMessage 是插件产出的统一中间结构，屏蔽各平台差异。完整字段定义及身份映射规则见 [common 共享类型](../common/shared-types.md)。

IM Adapter 负责在入站解析时填充 NormalizedMessage 的全部字段——各平台插件将原生格式转为统一结构，Processor Chain 和 Gateway 下游消费时不感知平台差异。

**引用/回复消息的处理**：若 IM 平台支持消息引用/回复功能，Adapter 在解析时取出被引用消息的文本内容，截断至 500 字符（超出追加 `...`），渲染为 markdown blockquote 格式（`> 引用内容`），拼接在 `content` 字段之前。不传递独立的引用消息字段。

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
- 含 Image/Audio/File 块 → 富格式消息

例外：terminal 渠道无富格式消息形态，恒输出 text 消息——富内容在 payload 内转为 ANSI 样式文本（见 [cli/Terminal Renderer](../cli/renderer.md)）。

## 数据流

### 入站路径

```
1. IM 平台 webhook 到达。
2. IMPlugin 入站：平台格式解析 → NormalizedMessage { platform, sender_id, peer_id, content, ... }。← 日志：入站解析（平台、消息类型、解析耗时）
3. Processor Chain 入站依次执行 RawLog → SessionRouter → ContentNormalizer。
4. 产出 [ProcessedMessage](../common/shared-types.md#processedmessage) → Gateway 路由决策。
```

### 出站路径

```
1. LLM 输出 ContentBlock[]。
2. Processor Chain 出站（DslParser）。
3. 产出 [ProcessedMessage](../common/shared-types.md#processedmessage)。
4. IMPlugin 渲染：渲染接口接收 ContentBlock[] 与 DSL 解析结果（定义见 [common DslParseResult](../common/shared-types.md#dslparseresult--dslinstruction)），产出 RenderedOutput { msg_type, payload }。← 日志：出站渲染（平台、渲染耗时）
5. 中间件插入点：Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件。
6. IMPlugin 发送：发送接口将渲染结果按 peer_id、thread_id 投递到平台。← 日志：平台 API 发送（平台、目标、耗时）
```

出站路径中，Renderer 不属 Processor Chain——渲染是终结操作，输出后无后续处理器接力。Gateway 根据目标 platform 选择对应插件，调用插件内部的 Renderer 完成渲染，再通过 Adapter 发送。

peer_id 和 thread_id 来源于入站时 IM Adapter 填入 NormalizedMessage 的对应字段，经 Session 上下文存储后在出站时取出，由 Gateway 传递给 IMPlugin 的发送方法。

## 模块关系

- **上游**：Gateway（出站方向：调用 IM Adapter 完成渲染和发送）、Config（accounts.json：入站解析时查询身份映射表，将 sender_id 转为 account_id）
- **下游**：Processor Chain（入站方向：消费 IM Adapter 产出的 NormalizedMessage）、debug_log（入站解析、出站渲染、平台发送各环节记录调试日志）
- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：ToolRegistrar、IMPlugin、StreamingRenderer；消费：IdentityResolver）
- **无关**：Session（IMPlugin 不直接参与 session 生命周期管理；peer_id/thread_id 经 Session 上下文存储后由 Gateway 在出站时取出传入）、LLM Provider（IMPlugin 不调用 LLM）、Slash Command（IMPlugin 不参与指令解析）
