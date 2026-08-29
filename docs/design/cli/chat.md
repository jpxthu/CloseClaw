# CLI Chat

## 概述

CLI Chat 是 terminal 消息渠道的对话交互功能，通过 `closeclaw chat` 命令启动。它实现 IMPlugin trait，以 platform="terminal" 注册到 Gateway 的 Plugin Registry，将终端输入输出接入完整的出入站消息链路。此功能依赖 daemon 已运行：启动时检测 daemon 不可达则报错退出，提示用户先执行 `closeclaw run`，不自动拉起 daemon。

## 架构

CLI Chat 的实现实体是 TerminalPlugin（IMPlugin trait 的 terminal 渠道实现），包含 TerminalAdapter（入站解析）和 TerminalRenderer（出站渲染）两个组件。

入站链路：

1. stdin 输入
2. TerminalAdapter 解析为 NormalizedMessage
3. Processor Chain 入站依次处理（RawLog → SessionRouter → ContentNormalizer）
4. Gateway 路由，按内容分流：
   - 以 `/` 开头 → SlashDispatcher → 产出 ContentBlock[]
   - 普通文本 → Session → LLM → 产出 ContentBlock[]

两条分支的产出相同（ContentBlock[]），汇合后进入出站链路：

1. Processor Chain 出站依次过滤（VerbosityFilter → DslParser → OutboundRawLog）
2. TerminalPlugin 调用 TerminalRenderer 渲染 → RenderedOutput（ANSI 文本数据）
3. TerminalPlugin 发送到 stdout

### 入站：TerminalAdapter

TerminalAdapter 从 stdin 读取用户输入，封装为 NormalizedMessage（字段定义见 [common 共享类型](../common/shared-types.md)）。terminal 渠道的字段取值：

terminal 渠道 NormalizedMessage 取值：

- platform = "terminal"
- sender_id = 当前用户系统 UID
- peer_id = "cli"
- account_id = "owner"
- content = 原始输入文本
- message_type = text

其余字段（reply_ref、media_refs、unavailable_media、timestamp）按默认值：reply_ref 为空，media_refs 为空列表，unavailable_media 为空列表，timestamp 取系统时间。

消息过滤规则与其他渠道一致：空内容不产出 NormalizedMessage。

### 出站：TerminalRenderer

TerminalRenderer 接收 ContentBlock[]（定义见 [common ContentBlock](../common/shared-types.md#contentblock)）和 DSL 解析结果，转换为 ANSI 格式的 RenderedOutput。TerminalPlugin 通过 send 方法将 RenderedOutput 写入 stdout。渲染与发送分离，遵循 IM Adapter 框架的设计原则。详细渲染策略见 [Terminal Renderer](renderer.md)。

### Session 与 Agent 指定

用户通过 `--agent-id` 指定目标 agent。同一用户对不同 agent 的对话相互隔离（会话组织与路由机制见 [session 模块](../session/README.md)——SessionManager 按 agent_id 串行处理会话解析请求，避免并发竞态）。

通过 `/stop` 斜杠指令强制终止当前运行（固定 Forceful，级联终止子 session；停运行不停会话，详见 [slash/session-management.md](../slash/session-management.md)）；不活跃的 session 由 session 模块的后台归档任务自动归档。

## 数据流

0. 启动检查：连接 daemon 管理接口，不可达则报错退出并提示先执行 `closeclaw run`（不自动拉起）
1. stdin 逐行/逐段读取用户输入，TerminalAdapter 解析并封装 NormalizedMessage（内部含空内容过滤，空内容不产出 NormalizedMessage）。终端字段取值见上文架构节
2. Processor Chain 入站依次执行：RawLog 记录原始输入 → SessionRouter 计算会话路由键 → ContentNormalizer 文本标准化（去除控制字符和 ANSI 转义序列、压缩空行、去尾空格）
3. 处理后消息进入 Gateway 路由，按内容分流：
   - 以 `/` 开头 → SlashDispatcher（与飞书等渠道共享同一套）→ ContentBlock[]
   - 普通文本 → Session → LLM → ContentBlock[]
4. ContentBlock[] 经 Processor Chain 出站依次过滤（VerbosityFilter 按 Session Verbosity 等级过滤 → DslParser 扫描并剥离 DSL 指令行到 metadata，与通用 DslParser 行为一致不按平台过滤 → OutboundRawLog 写出站日志）
5. TerminalPlugin 调用 TerminalRenderer 执行渲染：获取终端能力信息（经 platform 模块，确定渲染模式与终端宽度）+ DSL 交互元素预处理（按钮/选择器 → 纯文本提示行，其他 DSL → 忽略）+ 逐块渲染（各块类型渲染策略见 [Terminal Renderer](renderer.md)，块间空行分隔，超宽截断）
6. TerminalRenderer 返回单个 RenderedOutput，TerminalPlugin 的 send 方法写入 stdout

> **流式路径**：LLM 流式输出时，不走 TerminalRenderer 批量渲染路径。ContentBlock[] 经统一预处理（VerbosityFilter → DslParser 零开销透传）后，IM Adapter 流式渲染组件驱动，TerminalPlugin 逐行产生增量 RenderedOutput 后立即写入 stdout。流式结束后，DslParser 完整解析 → OutboundRawLog 写入出站日志。详见 [IM Adapter 流式渲染](../im_adapter/streaming-render.md)。

## 模块关系

- **上游**：操作系统 stdin（用户输入）、Gateway（通过 IMPlugin trait 调用 TerminalPlugin 出站）
- **下游**：Gateway（接收 NormalizedMessage 入站路由）、stdout（TerminalPlugin.send() 输出渲染结果）
- **与模块内其他子功能**：使用 TerminalRenderer 完成出站渲染，renderer 文档定义详细的块类型渲染规则
- **无关**：CLI Admin（Admin 命令不走消息链路，不经过 TerminalPlugin）、IM Adapter 的具体平台实现（terminal 渠道与其平级）
