# CLI Chat

## 概述

CLI Chat 是 terminal 消息渠道的对话交互功能，通过 `closeclaw` 二进制的 `chat` 子命令启动（与 CLI Admin 同属一个二进制，按子命令分派）。它实现 IMPlugin trait，以 platform="terminal" 显式注册到 Gateway 的 Plugin Registry（非 platforms/ 插件的显式注册机制，见 [IM Adapter 插件体系](../im_adapter/README.md)），将终端输入输出接入完整的出入站消息链路。CLI Chat 是独立客户端进程——终端 I/O 与渲染在 chat 进程内完成，经 daemon 管理接口与 daemon 侧的 Gateway 及会话链路联通；terminal 渠道在该连接上与 Gateway 的 Plugin Registry 完成注册。此功能依赖 daemon 已运行：启动时连接 daemon 管理接口，不可达则报错退出，提示用户先执行 `closeclaw run`，不自动拉起 daemon。

## 架构

CLI Chat 的实现实体是 TerminalPlugin（IMPlugin trait 的 terminal 渠道实现），包含 TerminalAdapter（入站解析）和 TerminalRenderer（出站渲染）两个组件；TerminalPlugin 无特殊生命周期需求（IMPlugin 的 init/shutdown 为空实现）。启动时先连接 daemon 管理接口做存活检测（不可达则报错退出，见 §数据流）。

入站链路：

1. stdin 输入
2. TerminalAdapter 解析为 NormalizedMessage
3. Processor Chain 入站依次处理（RawLog → SessionRouter → ContentNormalizer，见 [processor_chain 入站链路](../processor_chain/inbound-chain.md)）
4. Gateway 路由，按内容分流：
   - 审批指令（`/approve-once`、`/approve-whitelist`、`/deny`）→ Gateway 硬拦截，走权限审批流，不进 SlashDispatcher
   - 其他以 `/` 开头 → SlashDispatcher → SlashHandler 返回 SlashResult → Gateway 经 SideEffectContext 触发执行 → ContentBlock[]
   - 普通文本 → Session → LlmCaller/UnifiedResponse → ContentBlock[]

两条分支的产出相同（ContentBlock[]），汇合后进入出站链路：

1. Processor Chain 出站依次处理（VerbosityFilter 过滤 → DslParser 解析并剥离 DSL → OutboundRawLog 记录日志）
2. TerminalPlugin 调用 TerminalRenderer 渲染 → RenderedOutput（按终端能力为 ANSI 或纯文本）
3. Gateway 在渲染与发送之间执行出站中间件链（审计、频率限制等，通过后才发送）
4. TerminalPlugin 发送到 stdout

### 入站：TerminalAdapter

TerminalAdapter 从 stdin 读取用户输入，封装为 NormalizedMessage（字段定义见 [common 共享类型](../common/shared-types.md)）。terminal 渠道的字段取值：

terminal 渠道 NormalizedMessage 取值：

- platform = "terminal"
- sender_id = 运行 chat 的 OS 用户标识（terminal 无平台内用户概念，以 OS 用户作为发送者标识）
- peer_id = 目标 Agent 标识（`--agent-id` 指定，默认取默认 Agent）——作为会话对端锚点，使不同 Agent 的对话落到不同会话路由键、相互隔离；同一 Agent 的多个 chat 实例共享同一路由键、落到同一会话
- account_id = "owner"（terminal 单用户恒为 Owner，不经账户绑定表映射）
- content = 原始输入文本
- message_type = text

其余字段按默认值：reply_ref 为空，media_refs 为空列表，unavailable_media 为空列表；timestamp 取毫秒级 Unix 系统时间。

消息过滤规则与其他渠道一致：空内容不产出 NormalizedMessage。

### 出站：TerminalRenderer

TerminalRenderer 接收 ContentBlock[]（定义见 [common ContentBlock](../common/shared-types.md#contentblock)）和 DSL 解析结果，转换为 RenderedOutput（按终端能力为 ANSI 或纯文本格式）。TerminalPlugin 通过 send 方法将 RenderedOutput 写入 stdout。渲染与发送分离，遵循 IM Adapter 框架的设计原则。详细渲染策略见 [Terminal Renderer](renderer.md)。

### Session 与 Agent 指定

用户通过 `--agent-id` 指定目标 agent，该标识作为 terminal 渠道的 peer_id 进入会话路由键（platform + sender_id + peer_id + account_id），使不同 Agent 的对话形成不同会话路由键、相互隔离（会话组织与路由机制见 [session 模块](../session/README.md)；SessionManager 对每个 agent_id 串行处理请求，避免同一会话路由键下的并发竞态）。

通过 `/stop` 斜杠指令强制终止当前运行（固定 Forceful，级联终止子 session；停运行不停会话，详见 [slash/session-management.md](../slash/session-management.md)）；不活跃的 session 由 session 模块的后台归档任务自动归档。

## 数据流

0. 启动检查：连接 daemon 管理接口，不可达则报错退出并提示先执行 `closeclaw run`（不自动拉起）
1. stdin 读取用户输入（按行切分，stdin 结束时 EOF 终止），TerminalAdapter 解析并封装 NormalizedMessage（内部含空内容过滤，空内容不产出 NormalizedMessage）。终端字段取值见上文架构节；stdin 结束（EOF / Ctrl+D）时 chat 正常退出
2. Processor Chain 入站依次执行：RawLog 记录原始输入 → SessionRouter 计算会话路由键 → ContentNormalizer 文本标准化（规则见 [processor_chain 入站链路](../processor_chain/inbound-chain.md)）
3. 处理后消息进入 Gateway：先做 Session 解析（渠道/绑定确定 Agent、会话路由键定位 Session，含忙碌入队、归档恢复等分支，见 [gateway 入站路径](../gateway/README.md)），再按内容分流：
   - 审批指令（`/approve-once`、`/approve-whitelist`、`/deny`）→ Gateway 硬拦截走权限审批流，不进 SlashDispatcher
   - 其他 `/` 开头 → SlashDispatcher（与飞书等渠道共享同一套）→ ContentBlock[]
   - 普通文本 → Session → LLM → ContentBlock[]
4. ContentBlock[] 经 Processor Chain 出站依次处理（VerbosityFilter 按 Session Verbosity 等级过滤 → DslParser 从 Text 块中解析并剥离 DSL 指令行、将 DslParseResult 写入 ProcessedMessage.metadata → OutboundRawLog 写出站日志）
5. TerminalPlugin 先经 platform 模块获取终端能力信息，调用 TerminalRenderer 执行渲染（入参 ContentBlock[] + DslParseResult + 终端能力信息）：确定渲染模式与终端宽度 + DSL 交互元素预处理（按钮/选择器 → 纯文本提示行，其他 DSL 类型在渲染层不输出提示行）+ 逐块渲染（渲染策略见 [Terminal Renderer](renderer.md)）
6. TerminalRenderer 返回单个 RenderedOutput；Gateway 在渲染与发送之间执行出站中间件链（审计、频率限制等，通过后才发送），随后 TerminalPlugin 的 send 方法写入 stdout

> **流式路径**：LLM 流式输出时，不走 TerminalRenderer 批量渲染路径。增量阶段开始前，Gateway 执行一次 pre-flight 出站中间件检查（审计、频率限制等，被拒则终止流式并发送拒绝通知）；随后 [StreamEvent](../common/shared-types.md#streamevent) 事件流经统一预处理（VerbosityFilter 按块边界过滤 → DslParser 零开销透传）后，TerminalPlugin 委托 common 流式渲染原语（[common StreamingRenderer](../common/core-traits.md#streamingrenderer)）逐事件产出增量输出（[StreamingOutput](../common/shared-types.md#streamingoutput)），逐片写入 stdout。流式结束后，由 Gateway 在收尾阶段调度 Processor Chain 完成 DslParser 完整解析与 OutboundRawLog 出站日志。详见 [IM Adapter 流式渲染](../im_adapter/streaming-render.md)。

## 模块关系

- **上游**：操作系统 stdin（用户输入）、Gateway（通过 IMPlugin trait 调用 TerminalPlugin 出站）、daemon（启动时经管理接口做存活检测）
- **下游**：Gateway（消费经 Processor Chain 产出的 ProcessedMessage 做入站路由，并调度出站链路）、stdout（TerminalPlugin.send() 输出渲染结果）
- **与模块内其他子功能**：使用 TerminalRenderer 完成出站渲染，renderer 文档定义详细的块类型渲染规则
- **无关**：CLI Admin（Admin 命令不走消息链路，不经过 TerminalPlugin）、IM Adapter 的具体平台实现（terminal 渠道与其平级）
