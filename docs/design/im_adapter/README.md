# IM Adapter

## 概述

- 关联需求文档：[requirements/im_adapter.md](../../requirements/im_adapter.md)
- IM Adapter 模块提供跨消息平台的插件化适配框架。每个消息平台封装为一个独立插件，包含协议适配（Adapter）和格式渲染（Renderer）两部分。Gateway 按平台选择对应插件，不关心插件内部实现。

## 架构

### 插件体系

IM Adapter 模块不包含业务逻辑，由三类组件组成：

- **插件接口层**：IMPlugin trait 是统一插件契约，完整接口定义见 [common/core-traits](../common/core-traits.md#implugin)。每个消息平台实现此 trait，提供入站解析、渲染、发送、生命周期四组方法。terminal 平台的实现位于 [CLI 模块](../cli/README.md)，不在此目录。
- **通用渲染能力**：代码块语法高亮和流式增量渲染是跨平台通用机制，以 IM Adapter 内的通用组件形式提供。各平台插件组合持有对应组件并在渲染时委托调用，按需覆盖平台差异化部分。
- **平台插件**：每个消息平台的数据和渲染实现。IM 平台（飞书、Discord 等）的插件放在 `platforms/` 子目录下。terminal 平台的实现位于 CLI 模块。

模块运行时注册表由 Gateway 维护：

- **Plugin Registry**：platform → IMPlugin 的映射。Gateway 通过 platform 字段选择插件。
- **插件注册机制**：分两阶段。**编译期发现**——构建时自动扫描 `platforms/` 目录，为每个平台生成模块声明，新增平台无需改动 Gateway 等核心代码、重启即自动生效。**运行时注册**——系统启动时遍历已发现的平台插件，读取配置中该平台的启用状态，仅对已启用平台执行注册，写入 Gateway 维护的 Plugin Registry。不在 `platforms/` 下的插件（如 CLI 模块的 terminal）通过显式注册加入 Plugin Registry。各已启用平台的初始化相互独立——单个插件初始化失败仅记录日志并跳过该平台，不影响其他已启用平台的正常加载与运行。新平台默认禁用，需在配置中显式添加并启用方可使用。**平台启用清单（含新增/停用平台）属重启生效类**：变更经配置模块确认后触发网关择机重启，重启后按新启用清单注册（机制见 [config 热重载](../config/hot-reload.md)「重启类变更确认与触发」，需求见 [im_adapter §F1](../../requirements/im_adapter.md)）。

平台插件为自包含模块，内部结构统一：

```
platforms/<平台名>/
├── mod.rs         — 插件注册（实现 IMPlugin trait）
├── adapter.rs     — 入站：平台事件解析 → NormalizedMessage（含媒体落盘）
│                  — 出站：平台 API 发送消息
│                  — token 管理与刷新
├── renderer.rs    — ContentBlock[] + DSL → 平台原生格式

└── tools/         — 平台工具实现（注册职责在模块级，见「对外工具」）
    ├── mod.rs     — 工具实现模块声明
    └── ...        — 各工具分组文件
```

各文件职责单一、无循环依赖。新增平台时按此布局创建目录即可。

### 对外工具

IM Adapter 模块通过 [ToolRegistrar](../tools/tool-registrar.md) trait 向 ToolRegistry 注册平台插件工具。注册入口是**模块级唯一注册入口**——模块以单一 Registrar 加入 Tools 模块的全局编排（[四个标准 Registrar](../tools/tool-registrar.md#四个标准-registrar) 之一），与 tools/session/skills 模块对称。平台插件目录（`platforms/<平台>/tools/`）只承载工具实现，不承担注册职责。

新增平台工具的约定：在对应平台 `tools/` 目录实现工具，并在模块级 Registrar 的注册清单中声明该工具分组。飞书平台注册的工具分组见 [飞书插件](platforms/feishu.md)，各工具分组详细参数见 [tools 模块文档](../tools/README.md)。

```
im_adapter/
├── README.md               ← 本文件（插件架构+通用能力索引）
├── code-render.md           ← 代码块语法高亮（平台无关）
├── streaming-render.md      ← 流式增量渲染（平台无关）
├── media-store.md           ← 媒体落盘与生命周期（平台无关）
└── platforms/
    └── feishu.md            ← 飞书插件
```

### IMPlugin trait 契约

IMPlugin trait 的完整接口契约定义见 [common/core-traits](../common/core-traits.md#implugin)。本文档聚焦 IM Adapter 模块对插件实现和注册编排的具体职责。

每个消息平台插件实现 IMPlugin trait，包含入站解析、渲染、发送、生命周期四组方法。平台凭证（token 等）的管理与刷新内聚在插件的 Adapter 内、随生命周期方法初始化与续期，不跨模块传递。渲染和发送拆为两步——渲染结果是数据，发送是副作用——Gateway 在两步之间可插入审计、频率限制等中间件（详见 [Gateway 出站中间件](../gateway/outbound-flow.md#出站中间件)）。

NormalizedMessage 是插件产出的统一中间结构，屏蔽各平台差异。完整字段定义及身份映射规则见 [common 共享类型](../common/shared-types.md)。

IM Adapter 负责在入站解析时填充 NormalizedMessage 的全部字段——各平台插件将原生格式转为统一结构，Processor Chain 和 Gateway 下游消费时不感知平台差异。归一化以 NormalizedMessage 字段集为完整边界：各平台特有的元数据（API 字段名、租户标识等）不进入归一化结构，在插件内消化。

**引用/回复消息的处理**：若 IM 平台支持消息引用/回复功能，Adapter 按 [common 共享类型](../common/shared-types.md) 的引用/回复消息处理规则解析——被引用内容截断至 500 字符、以 markdown blockquote 拼接在 content 字段之前，不传递独立的引用消息字段（此处仅指被引用内容不单独立字段；出站定向投递使用的 reply_ref 是独立的回复目标标识，见出站路径）。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [代码块渲染](code-render.md) | 代码块语法高亮，按平台选择渲染策略 |
| [流式渲染](streaming-render.md) | 流式增量输出，行缓冲 + 块类型路由 |
| [媒体存储](media-store.md) | 媒体落盘、上下文形态、出站读取约束与生命周期 |
| [飞书插件](platforms/feishu.md) | 飞书平台完整插件实现（基于 lark-cli） |

### 平台渲染选择

各消息平台插件根据内容特征自动选择输出格式：

- 纯文本、无格式标记、无 DSL → text 消息
- 含 markdown 格式（标题/粗体/斜体/代码块/列表/引用/链接/分割线）或换行或 DSL → 富格式消息
- 含 Thinking/ToolUse/ToolResult 块 → 富格式消息
- 含 Image/Audio/File 块 → 富格式消息

例外：terminal 渠道无富格式消息形态，恒输出 text 消息——富内容在 payload 内转为 ANSI 样式文本（见 [cli/Terminal Renderer](../cli/renderer.md)）；流式模式下 DSL 交互指令不产生渲染输出（仅日志记录与出站历史写入），交互指令仅在批量模式渲染（见[流式渲染](streaming-render.md)）。

## 数据流

### 入站路径

```
1. IM 平台事件到达。
2. IMPlugin 入站：平台格式解析 → NormalizedMessage { platform, sender_id, account_id, peer_id, content, ... }（account_id 经 Config 身份映射得到，见「模块关系-上游」）；消息携带媒体时立即下载落盘并填入 media_refs（见 [媒体存储](media-store.md)）。← 日志：入站解析（平台、消息类型、解析耗时）
3. Processor Chain 入站依次执行 RawLog → SessionRouter → ContentNormalizer。
4. 产出 [ProcessedMessage](../common/shared-types.md#processedmessage) → Gateway 路由决策。
```

### 出站路径

```
1. LLM 输出 ContentBlock[]。
2. Processor Chain 出站依次执行（Verbosity 过滤 → DSL 解析 → 出站日志，见 [Processor Chain 出站链路](../processor_chain/README.md)）。
3. 产出 [ProcessedMessage](../common/shared-types.md#processedmessage)。
4. IMPlugin 渲染：渲染接口消费 Step 3 的 ProcessedMessage（内含 ContentBlock[] 与 DSL 解析结果，定义见 [common DslParseResult](../common/shared-types.md#dslparseresult--dslinstruction)），产出 RenderedOutput { msg_type, payload }。← 日志：出站渲染（平台、渲染耗时）
5. 中间件插入点：Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件（流式模式下在增量阶段开始前执行一次 pre-flight 检查，非逐片插入，见[流式渲染](streaming-render.md)）。
6. IMPlugin 发送：发送接口将渲染结果按 peer_id、reply_ref 投递到平台。← 日志：平台 API 发送（平台、目标、耗时）
```

出站路径中，Renderer 不属 Processor Chain——渲染是终结操作，输出后无后续处理器接力。Gateway 根据目标 platform 选择对应插件，调用插件内部的 Renderer 完成渲染，再通过 Adapter 发送。

peer_id 和 reply_ref 来源于入站时 IM Adapter 填入 NormalizedMessage 的对应字段，经 Session 上下文存储后在出站时取出，由 Gateway 传递给 IMPlugin 的发送方法。

## 模块关系

- **上游**：Gateway（出站方向：调用 IM Adapter 完成渲染和发送）、Config（accounts.json：入站解析时查询身份映射表，将 sender_id 转为 account_id）
- **下游**：Processor Chain（入站方向：消费 IM Adapter 产出的 NormalizedMessage）、debug_log（入站解析、出站渲染、平台发送各环节记录调试日志）
- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：ToolRegistrar、IMPlugin、StreamingRenderer；消费：IdentityResolver）
- **无关**：Session（IMPlugin 不直接参与 session 生命周期管理；peer_id/reply_ref 经 Session 上下文存储后由 Gateway 在出站时取出传入）、LLM Provider（IMPlugin 不调用 LLM）、Slash Command（IMPlugin 不参与指令解析）

### 平台接口真实性验证

IM 平台接口行为可能随平台侧变更而漂移（事件结构、字段语义、接口可用性）。每个平台插件提供一套真实性验证手段，用于发现此类漂移：

- **离线对照**：单元测试与解析逻辑以已采集的真实事件样本（fixture）为权威输入，样本存于 `tests/fixtures/<平台>/`；平台接口行为与限制另以平台官方 API 文档为权威来源，开发实现与单元测试须对照两者验证（见各平台插件文档，如 [feishu 插件「平台接口权威来源」](platforms/feishu.md)）
- **真实收发验证**：平台插件附带验证脚本，向平台发起真实收发，核对接口行为与已采集样本仍一致；脚本由维护者不定期手动触发，不进入自动化流程（不进 cargo test）
- **凭证安全**：验证使用真实平台账号，凭证与身份信息不落在代码库内（飞书平台经 lark-cli profile 提供，见 [飞书插件](platforms/feishu.md)）
- 验证产出的新样本用于更新 fixture，接口行为变化时同步修订设计与解析实现
