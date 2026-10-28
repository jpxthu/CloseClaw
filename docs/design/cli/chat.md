# CLI Chat

## 概述

CLI Chat 是 terminal 消息渠道的对话交互功能，通过 `closeclaw` 二进制的 `chat` 子命令启动（与 CLI Admin 同属一个二进制，按子命令分派）。Chat 进程是纯客户端：只负责 stdin/stdout、与 daemon 建立的 chat 连接（独立于 Admin 管理接口）；terminal 渠道的 IMPlugin 由 daemon 侧以 RPC 代理方式实现（连接建立时由 Daemon 注册，非 platforms/ 插件的显式注册机制，见 [IM Adapter 插件体系](../im_adapter/README.md)），消息解析、Processor Chain、渲染与发送均在 daemon 侧。此功能依赖 daemon 已运行：启动时连接 chat 连接，不可达则以提示行告知 daemon 未运行、需先执行 `closeclaw run`，随后结束（不自动拉起）；连接建立后被断开（daemon 关闭或配置变更引发网关重建）时，在途输出直接截断、进程退出不自动重连（与 CLI Admin 口径一致）。

## 架构

CLI Chat 的实现实体是 daemon 侧的 terminal 渠道插件（IMPlugin 的 RPC 代理实现），内部包含 TerminalAdapter（入站解析）和 TerminalRenderer（出站渲染）两个组件，并持有 common 流式渲染原语（[StreamingRenderer](../common/core-traits.md#streamingrenderer)）用于流式路径（见 §数据流「流式路径」）；终端能力检测经 platform 模块在 daemon 侧获取，渲染结果为终端文本、经 chat 连接回传。插件无特殊生命周期需求（IMPlugin 的 init/shutdown 为空实现）。启动时先建立 chat 连接做存活检测（不可达则提示并结束，见 §数据流）。

入站链路：

1. stdin 输入
2. TerminalAdapter 解析为 NormalizedMessage
3. Processor Chain 入站依次处理（RawLog → SessionRouter → ContentNormalizer，见 [processor_chain 入站链路](../processor_chain/inbound-chain.md)）
4. Gateway 路由，按内容分流：
   - 审批指令（`/approve-once`、`/approve-whitelist`、`/deny`）→ Gateway 硬拦截，走权限审批流，不进 SlashDispatcher
   - 其他以 `/` 开头 → SlashDispatcher → SlashHandler 返回 SlashResult → Gateway 经 SideEffectContext 触发执行 → ContentBlock[]
   - 普通文本 → Session → LlmCaller/UnifiedResponse → ContentBlock[]

两条分支的产出相同（ContentBlock[]），汇合后进入出站链路：

1. Processor Chain 出站依次处理（VerbosityFilter 过滤 → OutboundRawLog 记录日志）
2. TerminalPlugin 调用 TerminalRenderer 渲染 → RenderedOutput（按终端能力为 ANSI 或纯文本）
3. Gateway 在渲染与发送之间执行出站中间件链（审计、频率限制等，通过后才发送）
4. TerminalPlugin 发送到 stdout

### 入站：TerminalAdapter

terminal 渠道的字段由 daemon 侧从 chat RPC 收到的输入构造（chat 进程只负责 stdin/stdout）。字段取值：

terminal 渠道 NormalizedMessage 取值：

- platform = "terminal"
- sender_id = "owner"（CLI 渠道的 User 上下文为 Owner，需求 [cli §F1](../../requirements/cli.md)）；路由键因此与运行 chat 的 OS 用户无关（同一 Agent 的终端对话归属同一 Session）
- peer_id = 目标 Agent 标识（`--agent-id` 指定，未指定时取初始 Agent）——作为会话对端锚点，使不同 Agent 的对话落到不同会话路由键、相互隔离；同一 Agent 的多个 chat 实例共享同一路由键、落到同一会话
- account_id = "owner"（terminal 单用户恒为 Owner，不经账户绑定表映射）
- content = 原始输入文本
- message_type = text

其余字段按默认值：reply_ref 为空，media_refs 为空列表，unavailable_media 为空列表；timestamp 取毫秒级 Unix 系统时间。

消息过滤规则与其他渠道一致：空内容不产出 NormalizedMessage。

### 出站：TerminalRenderer

TerminalRenderer 接收 ContentBlock[]（定义见 [common ContentBlock](../common/shared-types.md#contentblock)），转换为 RenderedOutput（按终端能力为 ANSI 或纯文本格式）。TerminalPlugin 通过 send 方法将 RenderedOutput 写入 stdout。渲染与发送分离，遵循 IM Adapter 框架的设计原则。详细渲染策略见 [Terminal Renderer](renderer.md)。

### Session 与 Agent 指定

`--agent-id` 为可选参数：显式指定时以该标识作为 terminal 渠道的 peer_id；未指定时取系统初始 Agent（`initial: true` 的 Agent，标记与加载规则见 [Agent 配置档案](../agent/agent-config.md)）。解析在 daemon 侧完成（以 AgentRegistry 的运行时注册表为准，与 Session 创建使用同一配置源），CLI 仅负责传参。该标识作为 terminal 渠道的 peer_id 进入会话路由键（platform + sender_id + peer_id + account_id），使不同 Agent 的对话形成不同会话路由键、相互隔离（会话组织与路由机制见 [session 模块](../session/README.md)；SessionManager 对每个 agent_id 串行处理请求，避免同一会话路由键下的并发竞态）。

目标 Agent 解析失败即提示并结束，不回退到其它 Agent：

- 显式指定的 `--agent-id` 未注册或配置无法加载：提示该 Agent ID 不可用，并以 `closeclaw agent list` 列出可用 Agent 作为下一步
- 未指定且系统中不存在初始 Agent（无 `initial: true` 的 Agent）：提示先执行 `closeclaw config setup` 完成初始 Agent 创建

两种情形均不建立对话会话。

会话组织规则：同一 Agent 的终端对话归属同一个 Session（`/new` 强制新建时除外）；终端对话与 IM 渠道的 Session 相互隔离；退出终端不影响 Session 生命周期（退出后由 session 模块按 idle 判据归档清理）。

通过 `/stop` 斜杠指令强制终止当前运行（固定 Forceful，级联终止子 session；停运行不停会话，详见 [slash/session-management.md](../slash/session-management.md)）；不活跃的 session 由 session 模块的后台归档任务自动归档。

## 数据流

0. 启动检查：chat 进程连接 daemon 的 chat 连接（独立于 Admin 管理接口），不可达则以提示行告知 daemon 未运行、需先执行 `closeclaw run`，随后结束（不自动拉起）；启动检查结果与耗时记入调试日志。目标 Agent 解析在 daemon 侧完成（AgentRegistry + 注册清单）：未指定 `--agent-id` 时取初始 Agent；解析失败则提示并结束，不建立对话
1. stdin 读取用户输入（按行切分，stdin 结束时 EOF 终止），TerminalAdapter 解析并封装 NormalizedMessage（内部含空内容过滤，空内容不产出 NormalizedMessage）。终端字段取值见上文架构节；stdin 结束（EOF / Ctrl+D）时 chat 正常退出
2. Processor Chain 入站依次执行：RawLog 记录原始输入 → SessionRouter 计算 session_key（消息级标识，写入 metadata）→ ContentNormalizer 文本标准化（规则见 [processor_chain 入站链路](../processor_chain/inbound-chain.md)）
3. 处理后消息进入 Gateway（daemon 侧）：先做 Session 解析（terminal 渠道无机器人↔Agent 绑定，目标 Agent 取 `--agent-id` 或初始 Agent，写入 metadata.agent_id，peer_id 取同一标识；会话路由键定位 Session，含忙碌入队、归档恢复等分支，见 [gateway 入站路径](../gateway/README.md)），再按内容分流：
   - 审批指令（`/approve-once`、`/approve-whitelist`、`/deny`）→ Gateway 硬拦截走权限审批流，不进 SlashDispatcher
   - 有等待中的审批请求时，审批推送通道（Gateway 经 CLI 渠道）向该终端连接推送审批提示（内容与审批卡片一致，覆盖全部等待中的请求）；Owner 可在终端直接发出审批指令，按审批单号命中，与 Session 无关（见 [permission 审批工作流](../permission/approval-workflow.md#审批信息与投递)）
   - 其他 `/` 开头 → SlashDispatcher（与飞书等渠道共享同一套）→ ContentBlock[]
   - 普通文本 → Session → LLM → ContentBlock[]
4. ContentBlock[] 经 Processor Chain 出站依次处理（VerbosityFilter 按 Session Verbosity 等级过滤 → OutboundRawLog 写出站日志）
5. TerminalPlugin 先经 platform 模块获取终端能力信息，调用 TerminalRenderer 执行渲染（入参 ContentBlock[] + 终端能力信息）：确定渲染模式与终端宽度 + 逐块渲染（渲染策略见 [Terminal Renderer](renderer.md)）
6. TerminalRenderer 返回单个 RenderedOutput；Gateway 在渲染与发送之间执行出站中间件链（审计、频率限制等，通过后才发送），随后 TerminalPlugin 的 send 方法写入 stdout

**进程归属**：chat 进程只负责 stdin/stdout 与 chat RPC 客户端；NormalizedMessage 构造、入站/出站 Processor Chain、渲染（daemon 侧 RPC terminal 插件调 TerminalRenderer）、出站中间件、系统提示与流式增量均在 daemon 侧，渲染结果经 chat RPC 回传后由 chat 进程写 stdout。daemon 侧主动产生的内容——排队/恢复提示、后台任务与子 Session 完成通知、流式增量——可随时插入输出，不等待当前输入行结束；回复按请求-响应配对回投发起该次请求的连接，主动通知投递给该 Agent 当前活跃连接。

> **流式路径**：LLM 流式输出时，不走 TerminalRenderer 批量渲染路径。增量阶段开始前，Gateway 执行一次 pre-flight 出站中间件检查（审计、频率限制等，被拒则终止流式并发送拒绝通知）；随后 [StreamEvent](../common/shared-types.md#streamevent) 事件流经统一预处理（VerbosityFilter 按块边界过滤）后，TerminalPlugin 委托 common 流式渲染原语（[common StreamingRenderer](../common/core-traits.md#streamingrenderer)）逐事件产出增量输出（[StreamingOutput](../common/shared-types.md#streamingoutput)），逐片写入 stdout。流式结束后，由 Gateway 在收尾阶段调度 Processor Chain 完成 OutboundRawLog 出站日志。详见 [IM Adapter 流式渲染](../im_adapter/streaming-render.md)。

## 模块关系

- **上游**：操作系统 stdin（用户输入）、Gateway（通过 IMPlugin trait 调用 TerminalPlugin 出站）、daemon（启动时经管理接口做存活检测；断开即报错退出）、platform 模块（提供终端能力检测结果，供 TerminalPlugin 渲染时获取）、Config 模块（读取注册清单与 Agent 配置，解析缺省目标 Agent）
- **下游**：Gateway（消费经 Processor Chain 产出的 ProcessedMessage 做入站路由，并调度出站链路）、stdout（TerminalPlugin.send() 输出渲染结果）
- **与模块内其他子功能**：使用 TerminalRenderer 完成出站渲染，renderer 文档定义详细的块类型渲染规则
- **无关**：CLI Admin（Admin 命令不走消息链路，不经过 TerminalPlugin）、IM Adapter 的具体平台实现（terminal 渠道与其平级）
