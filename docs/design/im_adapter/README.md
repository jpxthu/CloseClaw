# IM Adapter

## 概述

- 关联需求文档：[requirements/im_adapter.md](../../requirements/im_adapter.md)
- IM Adapter 模块提供跨消息平台的插件化适配框架。每个消息平台封装为一个独立插件，插件内部由协议适配（Adapter）、格式渲染（Renderer）与平台工具三部分组成；模块层面的组成见「架构」。Gateway 按平台选择对应插件，不关心插件内部实现。
- 多媒体能力按需求暂不设计：媒体落盘、上下文形态与出站媒体发送均为暂缓设计，当前不实现，见 [媒体存储](media-store.md)。

## 架构

### 插件体系

IM Adapter 模块不承载对话、会话、路由等业务逻辑，由以下部分组成：

- **插件接口层**：IMPlugin trait 是统一插件契约，完整接口定义见 [common/core-traits](../common/core-traits.md#implugin)。每个消息平台实现此 trait，提供入站解析、渲染、发送、生命周期四组方法。terminal 平台的实现位于 [CLI 模块](../cli/README.md)，不在此目录。
- **通用渲染能力**：代码块渲染与流式增量渲染的平台无关原语（内容段解析、流式渲染）是跨平台共享的，位于 [common](../common/README.md)——流式渲染原语（trait 定义 + 默认实现 + 行缓冲）见 [common/core-traits](../common/core-traits.md#streamingrenderer)，内容段数据结构与内容段解析见 [common/shared-types](../common/shared-types.md#contentsegment--内容段落解析)。各平台适配器（im_adapter 与 cli）组合持有这些原语并在渲染时委托调用；平台专属 emit（飞书卡片富文本组装、终端 ANSI 渲染）由各适配器自身实现。
- **媒体存储（暂缓）**：需求暂不设计，当前不实现媒体落盘、上下文形态、出站读取与生命周期能力；启用前需重新设计，范围见 [媒体存储](media-store.md)。
- **平台插件**：每个消息平台的数据和渲染实现。IM 平台（飞书、Discord 等）的插件放在 `platforms/` 子目录下。terminal 平台的实现位于 CLI 模块。平台工具只封装平台 API 能力，不实现业务规则。

本文中「插件注册」指写入 Gateway 的 Plugin Registry，「工具注册」指经 ToolRegistrar 登记到 ToolRegistry，两者互不替代。

模块运行时注册表由 Gateway 维护：

- **Plugin Registry**：platform → IMPlugin 的映射。Gateway 通过 platform 字段选择插件。
- **插件注册机制**：分两阶段。**编译期发现**——构建时自动扫描 `platforms/` 目录生成平台清单模块声明，新增平台无需改动 Gateway 等核心代码；平台目录内的 `mod.rs` 仍为手写插件实现。**装配期注册**——Daemon 作为组合根，在启动与网关重建时创建已启用平台的插件实例并注册到 Gateway 维护的 Plugin Registry。启用状态读取自 [config plugins.json](../config/README.md)（平台启用清单），仅已启用平台注册；新平台默认禁用，须在配置中显式添加并启用。不在 `platforms/` 下的 terminal 插件为 RPC 代理实体，由 Daemon 在 chat 连接建立时创建并注册（chat 进程仅为客户端，不持有插件实例）。各已启用平台的初始化相互独立——单个插件初始化失败仅记录日志并跳过该平台，不影响其他已启用平台的正常加载与运行。平台 API 调用失败按插件内降级处理（记录日志、不导致进程崩溃）；平台工具执行失败向 Agent 返回明确错误（需求 §稳定性）。**平台启用清单（含新增/停用平台）属重启生效类**：变更经配置模块确认后触发 Gateway 择机重启；重启时新增平台创建插件并注册、停用平台关闭连接并移除、未变更平台保留连接并在重建后重新注册（机制见 [config 热重载](../config/hot-reload.md)「重启类变更确认与触发」，需求见 [im_adapter §F1](../../requirements/im_adapter.md)）。同一平台在单个实例内接入一个接收方机器人应用，多应用接入不在当前设计范围。

平台插件为自包含模块，内部结构统一：

```
platforms/<平台名>/
├── mod.rs         — 插件注册（实现 IMPlugin trait）
├── adapter.rs     — 入站：平台事件解析 → NormalizedMessage（媒体落盘随媒体能力暂缓）
│                  — 出站：平台 API 发送消息
│                  — token 管理与刷新
├── renderer.rs    — ContentBlock[] → 平台原生格式
└── tools/         — 平台工具实现（注册职责在模块级，见「对外工具」）
    ├── mod.rs     — 工具实现模块声明
    └── ...        — 各工具分组文件
```

各文件职责单一、无循环依赖。新增平台时按此布局创建目录。流式传输实现（StreamingSink）与平台接口真实性验证脚本随平台插件目录组织，不改变上述核心文件职责。

### 对外工具

IM Adapter 模块通过 [ToolRegistrar](../tools/tool-registrar.md) trait 向 ToolRegistry 注册平台插件工具。注册入口是**模块级唯一注册入口**——模块以单一 Registrar 加入 Tools 模块的全局编排（[四个标准 Registrar](../tools/tool-registrar.md#四个标准-registrar) 之一），与 tools/session/skills 模块对称。平台插件目录（`platforms/<平台>/tools/`）只承载工具实现，不承担注册职责。

新增平台工具的约定：在对应平台 `tools/` 目录实现工具，并在模块级 Registrar 的注册清单中声明该工具分组。Registrar 按平台启用状态注册分组——仅注册已启用平台的分组，未启用平台的工具不进入 ToolRegistry；启用清单变更属重启生效类，随重启重新注册。飞书平台注册的工具分组与各工具定义见 [飞书插件 · 对外工具](platforms/feishu.md#对外工具)。

```
im_adapter/
├── README.md               ← 本文件（插件架构+通用能力索引）
├── code-render.md           ← 代码块渲染（common 原语 + 平台策略）
├── streaming-render.md      ← 流式增量渲染（common 原语 + 平台 emit）
├── media-store.md           ← 媒体落盘与生命周期（平台无关；暂缓设计，当前不实现）
└── platforms/
    └── feishu.md            ← 飞书插件
```

### IMPlugin trait 契约

IMPlugin trait 的完整接口契约定义见 [common/core-traits](../common/core-traits.md#implugin)。本文档聚焦 IM Adapter 模块对插件实现和注册编排的具体职责。

平台凭证（token 等）由插件自行管理——可内聚在 Adapter 内随生命周期初始化与续期，也可委托平台 CLI 管理（如飞书由 lark-cli 管理），不跨模块传递。渲染和发送拆为两步——渲染结果是数据，发送是副作用——Gateway 在两步之间可插入审计、频率限制等中间件（详见 [Gateway 出站中间件](../gateway/outbound-flow.md#出站中间件)）。

NormalizedMessage 是插件产出的统一中间结构，屏蔽各平台差异。完整字段定义及身份映射规则见 [common 共享类型](../common/shared-types.md)。

IM Adapter 负责在入站解析时填充 NormalizedMessage 的全部字段——各平台插件将原生格式转为统一结构，Processor Chain 和 Gateway 下游消费时不感知平台差异。归一化以 NormalizedMessage 字段集为完整边界：平台特有的字段名不进入归一化结构，平台特有元数据需要向下游传递时仅以标准字段的取值承载；无需传递的部分在插件内消化。

**接收方机器人应用**：每个平台插件实例对应一个接收方机器人应用（平台凭证/profile 在插件配置中确定）。Gateway 按产出该消息的插件实例识别机器人应用，查 [config accounts.json](../config/README.md) 的机器人→Agent 绑定确定路由 Agent；接收方机器人应用不进入 NormalizedMessage 字段（需求 [im_adapter/feishu §F1](../../requirements/im_adapter/feishu.md)），平台事件中的 app_id 仅用于校验与身份映射键。插件声明接入模式（webhook / 长连接），Gateway 据此决定队列满拒绝时的送达语义（见 [Gateway 消息队列](../gateway/README.md#消息队列与排队语义)）。

**引用/回复消息的处理**：引用关系不在当前范围——Adapter 不识别引用、不回查被引用消息正文，被引用内容不拼接进 content（需求 [im_adapter §F2](../../requirements/im_adapter.md)）。出站定向投递不受影响：reply_ref 是独立的回复目标标识，用于把回复投递回原消息/话题位置。

### 平台接口真实性验证

IM 平台接口行为可能随平台侧变更而漂移（事件结构、字段语义、接口可用性）。每个平台插件提供一套真实性验证手段，用于发现此类漂移：

- **离线对照**：单元测试与解析逻辑以已采集的真实事件样本（fixture）为权威输入，样本存于 `tests/fixtures/<平台>/`（平台内可按采集方式分子目录，如飞书 `cli-poc/`）；平台接口行为与限制另以平台官方 API 文档为权威来源，开发实现与单元测试须对照两者验证（见各平台插件文档，如 [feishu 插件「平台接口权威来源」](platforms/feishu.md)）
- **真实收发验证**：平台插件附带验证脚本，向平台发起真实收发，核对接口行为与已采集样本仍一致；脚本由维护者不定期手动触发，不进入自动化流程（不进 cargo test）
- **凭证安全**：验证使用真实平台账号，凭证与身份信息不落在代码库内（飞书平台经 lark-cli profile 提供，见 [飞书插件](platforms/feishu.md)）
- 验证产出的新样本用于更新 fixture，接口行为变化时同步修订设计与解析实现

### 子功能索引

| 文档 | 内容 |
|------|------|
| [代码块渲染](code-render.md) | 代码块渲染：委托 common 内容段原语，按平台选择渲染策略 |
| [流式渲染](streaming-render.md) | 流式增量输出：委托 common 流式渲染原语，行缓冲 + 块类型路由 |
| [媒体存储](media-store.md) | 媒体落盘、上下文形态、出站读取约束与生命周期（暂缓设计，当前不实现） |
| [飞书插件](platforms/feishu.md) | 飞书平台完整插件实现（基于 lark-cli） |

### 平台渲染选择

各消息平台插件根据内容特征自动选择输出格式，格式选择由系统自动完成，用户与 Agent 无需手动指定：

- 纯文本、无格式标记、无换行、无 Thinking/ToolUse/ToolResult 块 → text 消息（轻量文本）
- 含 markdown 格式（标题/粗体/斜体/删除线/下划线/行内代码/代码块/列表/引用/链接/分割线）、换行、Thinking/ToolUse/ToolResult 块 → 富格式消息
- 含 Image/Audio/File 块 → 富格式消息（随媒体能力暂缓，当前不出现此类出站块）
- **交互控件的边界**：本模块不渲染交互控件（按钮/选择器等）——入站交互事件不在当前范围内，见 [im_adapter 需求](../../requirements/im_adapter.md)

**流式与批量的格式分流**：流式输出一律使用富格式消息——流式内容在生成完成前无法判定是否为纯文本，不适用 text 分支；轻量文本消息仅用于内容在发送前已完整生成、且判定为纯文本的批量消息。消息格式一经发出即锁定。

例外：terminal 渠道无富格式消息形态，恒输出 text 消息——富内容在 payload 内转为 ANSI 样式文本（见 [cli/Terminal Renderer](../cli/renderer.md)）。

## 数据流

### 入站路径

```
1. IM 平台事件经 Gateway 入站队列缓冲、出队后交由对应平台插件（队列、重放与去重见 [Gateway 入站流程](../gateway/inbound-flow.md)）。
2. IMPlugin 入站：插件区分消息事件与交互事件（如卡片按钮点击），仅消息事件转为 NormalizedMessage，交互事件走所在平台的独立通路（飞书当前仅记录调试日志）。消息事件平台格式解析 → NormalizedMessage { platform, sender_id, account_id, peer_id, content, ... }（account_id 经 Config 身份映射得到，见「模块关系-上游」）。过滤规则：text 类型空 content 消息丢弃；post 类型 content 为空且无媒体引用时丢弃（媒体能力暂缓，当前仅按 content 判定）。解析失败时不产出 NormalizedMessage、丢弃该事件并记录告警日志。媒体能力暂缓——当前不做媒体下载落盘、不填充 media_refs，非文本消息的处理不在实现范围（设计见 [媒体存储](media-store.md)）。← 日志：入站解析（平台、消息类型、解析耗时）
3. Processor Chain 入站依次执行 RawLog（仅 `raw_log_dir` 配置时注册）→ SessionRouter → ContentNormalizer。
4. 产出 [ProcessedMessage](../common/shared-types.md#processedmessage) → Gateway 路由决策。
```

### 出站路径

```
1. LLM 输出 ContentBlock[]。
2. Processor Chain 出站依次执行（Verbosity 过滤 → 出站日志（仅 `raw_log_dir` 配置时注册），见 [Processor Chain 出站链路](../processor_chain/outbound-chain.md)）。
3. 产出 [ProcessedMessage](../common/shared-types.md#processedmessage)。
4. IMPlugin 渲染：渲染接口消费 Step 3 的 ProcessedMessage（content_blocks + metadata），产出 RenderedOutput { msg_type, payload }。← 日志：出站渲染（平台、渲染耗时）
5. 中间件插入点：Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件（流式模式下在增量阶段开始前执行一次 pre-flight 检查，非逐片插入，见[流式渲染](streaming-render.md)）。
6. IMPlugin 发送：发送接口将渲染结果按 peer_id、reply_ref 投递到平台。← 日志：平台 API 发送（平台、目标、耗时）
```

**流式出站路径**：LLM 流式事件（StreamEvent）经增量阶段 Processor Chain（VerbosityFilter 过滤）→ 插件流式渲染器逐事件产出 StreamingOutput → Gateway 组装 RenderedOutput 并调用插件发送能力逐批投递（同一回复在平台侧增量更新）；收尾阶段由 Gateway 执行出站日志/历史写入（详见 [流式渲染](streaming-render.md)）。

出站路径中，Renderer 不属 Processor Chain——渲染是终结操作，输出后无后续处理器接力。Gateway 根据目标 platform 选择对应插件，调用插件内部的 Renderer 完成渲染，再通过 Adapter 发送。Gateway 的简化出站路径（队列满繁忙提示、错误/降级提示、系统通知等纯文本提示）同样经插件的渲染与发送能力投递，不经出站 Processor Chain。

peer_id 和 reply_ref 来源于入站时 IM Adapter 填入 NormalizedMessage 的对应字段，经 Session 上下文存储后在出站时取出，由 Gateway 传递给 IMPlugin 的发送方法。

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

- **上游**：Gateway（出站方向：调用 IM Adapter 完成渲染和发送）、Processor Chain（出站方向：消费链产出的 ProcessedMessage 渲染为平台格式）、Config（accounts.json：入站解析时按「平台 + 接收方机器人应用 + sender_id」查询身份映射表，将 sender_id 转为 account_id）
- **下游**：Processor Chain（入站方向：消费 IM Adapter 产出的 NormalizedMessage）、Gateway（出站方向：消费 RenderedOutput / StreamingOutput 并投递）、Session（出站定向字段承载：peer_id / reply_ref 经 Gateway 存入 Session 上下文，出站时取出回传）、debug_log（入站解析、出站渲染、平台发送各环节记录调试日志）
- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：ToolRegistrar、IMPlugin、StreamingSink（平台侧增量投递）；消费：IdentityResolver、StreamingRenderer；媒体能力暂缓，MediaStoreAccess 当前不实现）、[common/shared-types](../common/shared-types.md)（产出：NormalizedMessage、RenderedOutput、StreamingOutput；消费：ProcessedMessage、ContentBlock、StreamEvent、ContentSegment）
- **无关**：LLM Provider（IMPlugin 不调用 LLM）、Slash Command（IMPlugin 不参与指令解析）
