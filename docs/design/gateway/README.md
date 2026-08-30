# Gateway

## 概述

关联需求文档：[requirements/gateway.md](../../requirements/gateway.md)

Gateway 是消息路由中枢。它管理所有 IM 平台的插件，调度 Processor Chain 完成消息的出入站处理，做出路由决策（斜杠指令 vs 普通对话），并选择对应平台的 IM 插件完成出站消息的格式转换与发送。

Gateway 自身不含业务逻辑，通过编排下游模块完成消息流转。入站方向维护有界持久化消息队列缓冲高并发请求，出站方向根据交付模式协调 Processor Chain 执行时机。LLM 回复和斜杠指令回复统一经出站 Processor Chain 处理后发送。含媒体的消息经分型路由后进入对话流程（媒体上下文形态见 [im_adapter media-store](../im_adapter/media-store.md)）；媒体不可得时经简化出站路径提示用户；系统通知（如"⏳ 正在排队..."、"正在恢复会话..."）由各业务模块（Session 等）经 Gateway 的通用系统通知接口发送，同样走简化出站路径。出站完整链路（批量/流式时序、简化路径、出站历史、中间件）详见 [出站流程](outbound-flow.md)。

## 架构

Gateway 由以下职责组成：

- **IM Adapter 管理**：注册和维护各平台插件（含 IM 平台插件和 terminal 通道的 CLI）。入站方向将平台原始格式归一化为统一结构。
- **Processor Chain 调度**：按 priority 顺序调度入站和出站处理器链。入站链完成消息日志记录、session_key 计算和文本标准化。出站链按交付模式决定执行时机——批量模式一次性执行完整链；流式模式分四个阶段——pre-flight（增量开始前执行一次出站中间件链做检查，被拒则终止流式并发送拒绝通知）、增量阶段（消费 [StreamEvent](../common/shared-types.md#streamevent) 流式事件，按事件过滤渲染，DslParser 零开销透传、跳过出站调试日志）、收尾阶段（执行 DSL 解析和出站调试日志，不重跑 VerbosityFilter。流式模式下 DSL 指令仅用于日志记录和出站历史写入，不产生渲染输出）和出错降级（流式进行中出错时终止流式会话，经简化出站路径追加错误提示，详见 [出站流程](outbound-flow.md)）。
- **路由决策**：根据消息前缀决定走向——以 `/` 开头则拦截分派给斜杠指令处理（其中 `/approve-once`、`/approve-whitelist`、`/deny` 在 Gateway 层硬拦截不进 SlashDispatcher），否则路由到 Session 进入 LLM 对话流程。普通消息路由前，Gateway 先根据配置定义的机器人→Agent 绑定确定对应的 Agent，将 agent_id 一并传给 SessionManager。绑定关系由 [config accounts.json](../config/README.md)（账户映射）承载，属重启生效类：变更确认后由配置模块触发网关择机重启，重启后新绑定生效（已投递消息不受影响，详见 [daemon §F6](../../requirements/daemon.md)）。
- **出站中间件**：渲染完成后、发送前，Gateway 按注册顺序链式执行中间件（流式模式前置为 pre-flight）。详见 [出站流程](outbound-flow.md) 出站中间件节。
- **IM Adapter 选择与渲染**：出站方向根据目标平台选择对应 IM Adapter，调用其渲染接口产出平台格式内容。渲染完成后、发送前，Gateway 执行中间件链，通过后调用 IM Adapter 的发送接口完成消息投递。渲染和发送为分离接口。
- **出站历史记录**：出站消息发送后，Gateway 将消息写入 session checkpoint 持久化存储，作为用户可见内容的交付记录。详见 [出站流程](outbound-flow.md) 出站历史记录节。
- **系统通知接口**：Gateway 提供通用系统通知发送接口，供 Session 等模块发送纯文本系统通知（如"正在恢复会话..."、"⏳ 正在排队..."）。系统通知走简化出站路径。通知内容与触发时机由调用方模块负责。
- **系统生命周期管理**：Gateway 参与优雅关闭——响应 ShutdownHandle 的 drain 计数（消息处理开始时递增、响应发送完成后递减），排空入站队列，等待进行中的流式会话完成后退出。关闭流程由 Daemon 的 ShutdownHandle 统一协调，Gateway 是被管理组件之一。

Gateway 维护以下运行时注册表：

- **Plugin Registry**：platform → IMPlugin 的映射
- **Processor Registry**：入站/出站处理器链，按 priority 排序
- **入站消息队列**：有界持久化缓冲队列，默认容量 256，可通过配置调整。满则拒收并回复用户，不支持动态调整。消息入队即持久化，处理完成后删除，重启时重放未完成消息（详见「消息队列与排队语义」）

**明确不做的职责**（详见下方无关表）：Bootstrap 加载与 System Prompt 构建、LLM 调用、工具注册与工具调用的直接执行。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [入站流程](inbound-flow.md) | 入站完整链路：IM Adapter 解析 → Processor Chain → Gateway 路由决策 |
| [出站流程](outbound-flow.md) | 出站完整链路：批量/流式交付模式、简化出站路径、系统通知、出站历史记录、出站中间件 |

### 模块分层和数据流

消息流经的模块分层：IM Adapter（平台适配，入站解析/出站渲染发送）→ Processor Chain（消息变换：入站标准化与 session_key 计算、出站过滤/DSL 解析/日志）→ Gateway（编排与路由决策）→ 下游（SessionManager/Session、SlashDispatcher、Permission）。

- **入站**：平台事件到达 → 入站消息队列缓冲 → IM Adapter 解析为 NormalizedMessage（媒体已落盘为引用）→ Processor Chain 入站变换 → Gateway 分型路由（含媒体的分型进入对话流程）、机器人→Agent 绑定、Session 解析 → 路由决策（斜杠指令 / LLM 对话）。完整步骤详见 [数据流 · 入站路径](#入站路径)与[入站流程](inbound-flow.md)。
- **出站**：ContentBlock[] → Processor Chain 出站（过滤/DSL 解析/日志）→ IM Adapter 渲染 → 中间件链 → 发送 → 出站历史写入。批量/流式时序与简化路径详见 [数据流 · 出站路径](#出站路径)与[出站流程](outbound-flow.md)。

跨模块交接的共享数据结构见 [模块关系 · 共享类型](#共享类型)。

## 数据流

### 入站路径

Gateway 收到入站平台事件后，消息先进入入站消息队列（有界持久化缓冲，详见下方「消息队列与排队语义」），再由 IM Adapter 解析后进入 Processor Chain。Processor Chain 入站产出 [ProcessedMessage](../common/shared-types.md#processedmessage) 后，Gateway 按以下路径处理：

- **消息分型路由**：Gateway 读取 message_type 判断消息形态。含媒体的进入媒体可得性校验：媒体可得（media_refs 非空且 unavailable_media 为空）时按类型构造上下文形态（图片进对话内容、文件音频以媒体引用），进入正常对话链路（形态规则见 [im_adapter media-store](../im_adapter/media-store.md)）；媒体不可得（unavailable_media 非空，即下载失败或超出大小上限）时向用户提示「该消息内容无法获取」，经简化出站路径发送，流程结束（简化路径机制详见 [出站流程](outbound-flow.md) 简化出站路径节）。

- **Session 解析**：Gateway 从 metadata 取出 session_key，并根据配置定义的机器人→Agent 绑定确定对应的 Agent，得到 agent_id。若 session_key 为空（SessionRouter 计算失败），Gateway 记录 warning 日志，仍通过消息路由字段（platform, sender_id, peer_id, account_id）传给 SessionManager 正常完成 session 查找/创建（详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md)）。session_key 是消息级追踪标识，不参与 session 路由——SessionManager 从路由字段中提取稳定路由键做查找。Gateway 将 agent_id 连同 session_key、路由字段一并传给 SessionManager，session 在 Agent 范围内隔离（详见 [Session 模块](../session/README.md)）。若 SessionManager 的 session 查找或创建操作失败（如存储异常），Gateway 向 User 回复错误提示，不进入后续 LLM 对话流程。

- **路由决策**：获得 session_id 后按消息内容路由：
  - **`/` 开头 → 斜杠指令**：先拦截 `/approve-once`、`/approve-whitelist`、`/deny`。
    - 非 Owner 调用 `/approve-once`、`/approve-whitelist`、`/deny` → Gateway 直接回复"权限不足：该指令仅限 Owner 使用"，不进入 Permission 模块和 SlashDispatcher。流程到此结束。
    - `/approve-once`、`/approve-whitelist`、`/deny`（Owner 调用）：经 Permission 模块审批工作流管理审批流转（详见 [Permission 模块审批工作流](../permission/approval-workflow.md)）。审批通过后执行对应操作，结果经出站链路发送。
    - 其余斜杠指令分派给 SlashDispatcher。Gateway 将 session_id 传给 SlashDispatcher 作为执行上下文（权限校验依赖）。消息不进入 LLM，不追加到对话历史。
    - Immediate 指令（如 `/stop`、`/status`、`/help` 等）→ 绕过 Session 忙碌队列立即执行。完整 Immediate 标记见 [Slash 模块 Handler 清单](../slash/README.md#handler-清单)。
    - 非 Immediate 指令 → 若 Session 正忙则进入 Session 忙碌队列（FIFO），Session 空闲后取出执行。入队时由 Session 生成"⏳ 正在排队..."提示语，经 Gateway 的通用系统通知接口发送。
  - **普通消息**：若 Session 正忙则进入 Session 忙碌队列；如果 Session 空闲，则直接进入 LLM 对话流程。若 Session 处于 archived 状态，由 SessionManager 触发 restore 流程，Session 生成"正在恢复会话..."提示语，经 Gateway 的通用系统通知接口发送。Session 就绪后进入 LLM 对话流程，返回 ContentBlock[] 进入出站链路。

> 斜杠指令的解析和 SlashResult 处理详见 [slash 模块](../slash/README.md)。Session 的创建、查找、归档、恢复详见 [Session 模块](../session/README.md)。审批流程详见 [Permission 模块审批工作流](../permission/approval-workflow.md)。

**Gateway 调试日志**：Gateway 在以下环节记录调试日志（不含原始消息 Payload——原始内容日志由 Processor Chain 的 RawLog 组件负责）：

- **入站消息到达与队列操作**：记录平台事件到达、消息出队和队列满拒绝日志
- **路由决策结果**：记录识别结果（普通对话 / 斜杠指令 / 排队等待），含 session_id 和决策依据
- **中间件拦截**：中间件返回拒绝时记录告警日志，含拒绝原因和 session 标识

> 日志格式、级别、追踪标识、存储轮转和隐私脱敏遵循 [debug_log 框架](../debug_log/README.md)。Session 查找与生命周期事件日志由 [Session 模块](../session/README.md) 负责。出站渲染与平台 API 发送结果日志由 [IM Adapter 模块](../im_adapter/README.md) 负责。

### 出站路径

出站路径按交付模式分两种执行时序（批量/流式），共用同一组 Processor Chain 处理器和同一条 IM Adapter 渲染管线。完整执行时序、简化出站路径（媒体不可得提示、系统通知、流式降级提示）、出站历史记录与出站中间件契约详见 [出站流程](outbound-flow.md)。

**关键概要**：

- **批量模式**：ContentBlock[] 完整到齐后一次性执行出站链，渲染 → 中间件 → 发送 → 写出站历史；渲染/发送失败经简化路径发"回复发送失败"提示。斜杠指令回复统一走批量模式。
- **流式模式**：pre-flight 中间件 → 增量阶段（逐 [StreamEvent](../common/shared-types.md#streamevent) 事件过滤渲染）→ 收尾阶段（DSL 完整解析、写调试日志、写出站历史）→ 出错降级（简化路径追加错误提示）。
- **简化出站路径**：媒体不可得提示、系统通知、降级提示（含流式中断与批量发送失败）——跳过完整链，仅经调试日志（配置时）→ 渲染 → 发送，不写出站历史。
- **出站日志的两种形态**：出站调试日志（OutboundRawLog，链内 processor，仅 `raw_log_dir` 配置时注册）与出站历史记录（发送成功后写入 session checkpoint 的交付记录）。字段、定位与完整时序详见 [出站流程](outbound-flow.md)。

### 消息队列与排队语义

Gateway 涉及两层排队：

**第 1 层：入站消息队列**

- 位置：IM 平台事件到达后、进入 Processor Chain 之前
- 性质：有界持久化缓冲队列，默认容量 256。消息入队即持久化（WAL 追加），完整处理链结束后标记完成并删除
- 满行为：拒绝新消息，Gateway 根据事件来源平台选择对应 IM Adapter 回复"服务繁忙，请稍后重试"（经系统通知通道发送）
- 重启行为：重启时重放未标记完成的消息，未完成处理的消息不丢失（at-least-once）。消息重放时按消息身份去重，避免重复处理。优雅关闭时 Gateway 先停收新消息、排空已有队列后再退出
- **配置触发重建期暂存补投**（区别于上行 WAL 重启重放）：配置触发的 Gateway 重建执行期间，入站新消息不经队列满判定，照常走持久化入站队列暂存，不拒绝、不丢失；重建完成后按原到达顺序补投进处理链。触发时机与执行边界由 Daemon 协调（详见 [daemon README](../daemon/README.md)）
- 用户感知：正常负载下，文本消息到达 Gateway 后应在 1 秒内收到系统响应或排队提示。队列满时的"服务繁忙"拒绝本身是快速系统响应（另有 2 秒兜底约束），不视为例外；仅审批等待等真正异步等待的场景不适用此 1 秒响应约束
- 消费：IM Adapter 按 FIFO 从队列取消息解析，送入 Processor Chain 串行处理
- 送达语义（按平台接入模式分型）：webhook 类平台——队列满拒绝的消息未确认（ack），由对应平台自动重发；推送/长连接类平台（如飞书 CLI 事件订阅）——平台无自动重发，消息即终，用户需重新发送；系统仅回复一次繁忙提示。平台接入模式由各平台插件声明（见 [飞书插件](../im_adapter/platforms/feishu.md)）

**第 2 层：Session 忙碌队列**

- 位置：Gateway 路由决策后、进入 LLM 对话或非 Immediate 斜杠指令执行前
- 触发：Session 正忙（LLM 调用中或前台工具执行中）时新消息入队
- 性质：FIFO 待处理队列，Session 空闲后自动取出队首消息
- 通知：非 Immediate 消息（普通消息和非 Immediate 斜杠指令）入队时，由 Session 生成"⏳ 正在排队..."提示语，经 Gateway 的通用系统通知接口发送；Immediate 斜杠指令绕过此队列
- 详见 [Session 模块执行状态](../session/README.md)

1. 入站消息（高并发）进入入站消息队列（第 1 层；配置触发重建期间见上「暂存补投」条目）。
   - 有空闲槽位 → 进入 Processor Chain → 路由。
   - 队列满 → 拒绝 + 回复"服务繁忙，请稍后重试"。
2. 路由决策：
   - Immediate 指令 → 绕过 Session 队列，直接执行。
   - 其他 → 判断 Session 是否空闲：
     - 空闲 → 直接处理。
     - 正忙 → 进入 Session 忙碌队列（第 2 层），通知"⏳ 正在排队..."。Session 空闲后 FIFO 取出，再按 Session 状态分派：
       - Session archived → restore → 通知"正在恢复会话..." → Session 就绪后按原路由分派（LLM / SlashDispatcher）。
       - Session active → 按原路由分派（LLM / SlashDispatcher）。

### 斜杠指令副作用执行

SlashDispatcher 返回 [SlashResult](../common/shared-types.md#slashresult) 后，Gateway 构造 SideEffectContext（封装 Session 引用和回复通道）并触发 SlashResult 执行。各 SlashResult 变体在其执行逻辑中通过上下文完成对应的 session 操作。Gateway 不穷举变体，副作用逻辑内聚在 slash 模块。

SlashResult 的执行通过上下文的回复通道产出回复内容，Gateway 将回复送入出站 Processor Chain（VerbosityFilter → DslParser → OutboundRawLog）处理后由 IM Adapter 渲染发送。详见 [Slash 模块](../slash/README.md)。

### 权限调用时机

Gateway 在以下场景调用 Permission 模块：

1. **`/approve-once`、`/approve-whitelist`、`/deny`**：消息路由阶段硬拦截——不进 SlashDispatcher，直接在 Gateway 层调用 Permission 模块的审批工作流（owner 专用）。审批通过后执行操作，结果经出站链路发送。
2. **其他斜杠指令高危操作**（`/exec`、`/git` 写操作）：在 SlashDispatcher 分派到 Handler、Handler 返回 SlashResult 后、执行前校验。SlashResult 变体自带「需权限校验」标记，Gateway 读取该标记决定是否调用 Permission——不穷举高危变体清单，新增高危指令只需在新变体上声明标记。校验在构造 SideEffectContext 之前完成：校验未通过不构造上下文，直接按 Deny 分流处理；通过后才构造上下文触发执行。Handler 仅做指令解析（无副作用），权限引擎拿到完整操作信息后评估——非 Owner 默认 Deny，但可通过白名单规则授予特定 Agent-User 组合的执行权（详见 [Permission 模块](../permission/README.md)）。

Gateway 自身的消息路由、Processor Chain 调度、IM Adapter 选择均不经过权限检查。工具调用的权限检查由 tools 模块触发，Gateway 不参与。

### 模块关系

### 上游（输入来源）

| 模块 | 关系 |
|------|------|
| IM Adapter | 入站消息通过插件进入 Gateway 入站处理 |
| Session | LLM 响应以 ContentBlock[] 形式传入 Gateway 出站发送；系统通知经 Gateway 通用系统通知接口发送 |
| Config | Gateway 读取机器人与 Agent 的绑定关系（机器人→Agent 绑定由 [config accounts.json](../config/README.md) 承载），据此确定普通消息路由到的 Agent；绑定属重启生效类，变更经配置模块确认后触发网关重启生效 |

### 下游（Gateway 调用谁）

| 模块 | 关系 |
|------|------|
| Processor Chain | 调度入站和出站处理器链 |
| SlashDispatcher | 斜杠指令拦截后分派给 SlashDispatcher |
| SessionManager | 调用 SessionManager（传入 agent_id + session_key 和消息路由字段），由 SessionManager 内部提取稳定路由键进行 session 查找/创建。生命周期实现由 SessionManager 负责 |
| IM Adapter | 选择对应平台插件完成出站渲染与发送 |
| Permission | 斜杠指令高危操作执行前校验 |
| 审批工作流（Permission 模块） | `/approve-once`、`/approve-whitelist`、`/deny` 指令的审批流转管理（审批请求入队、Owner 通知、回调处理），详见 [审批工作流](../permission/approval-workflow.md) |

### 共享类型

跨模块传递的共享数据结构定义在 [common 模块](../common/README.md)：

- [NormalizedMessage](../common/shared-types.md#normalizedmessage)：IM Adapter 产出，Processor Chain 消费
- [ProcessedMessage](../common/shared-types.md#processedmessage)：Processor Chain 产出，Gateway 消费
- [ContentBlock](../common/shared-types.md#contentblock)：LLM 响应 / SlashResult 变体产出，Processor Chain 出站消费
- [SlashResult](../common/shared-types.md#slashresult)：SlashDispatcher 产出，Gateway 消费
- [DslParseResult](../common/shared-types.md#dslparseresult--dslinstruction)：DslParser 产出，IM Adapter 消费（渲染）和 Gateway 消费（出站历史记录写入）
- SideEffectContext：Gateway 构造的执行上下文，封装 Session 引用和回复通道。定义见 [common SlashResult](../common/shared-types.md#slashresult)。Gateway 构造后触发 [SlashResult](../common/shared-types.md#slashresult) 执行，各变体通过上下文完成副作用
- RenderedOutput：IM Adapter 渲染产出，Gateway 中间件消费（只读不修改），最终由 IM Adapter 发送接口投递

- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：LlmCaller、MetricsEmitter、OutboundMiddleware、SlashEffectExecutor、SlashSessionQuery、SessionLookup、PermissionChecker、ShutdownSignal；消费：IMPlugin、SlashRouter、ProcessorChain、OutboundMiddleware、ToolRegistryQuery、SkillRegistryQuery、SlashResultExecutor）

### 无关

- **Bootstrap**（无调用关系）：Gateway 不参与 Bootstrap 加载
- **System Prompt**（仅注入不构建）：Gateway 不构建 system prompt 内容，但 SessionManager 持有 SystemPromptBuilder / DynamicPromptBuilder 并注入 session（DI 接线，见 [common core-traits](../common/core-traits.md)）
- **LLM Provider**（无调用关系）：Gateway 不直接调用 LLM。Gateway 提供 LlmCaller 抽象的具体实现（[FallbackLlmCaller](../common/core-traits.md#llmcaller)，桥接统一客户端），但真实 Provider 请求由 LLM 模块完成。
- **Tools**（无调用关系）：Gateway 不注册工具、不执行工具调用
