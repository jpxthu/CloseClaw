# 核心 trait

## 概述

核心 trait 是跨模块依赖注入的接口契约。本文档唯一定义 common crate 中每个核心 DI trait 的完整接口。各业务模块文档通过引用指向此处，不在自身文档中重复定义本文档已收录的 trait。

trait 归属按 [STANDARDS](../STANDARDS.md)「common 文档内容准入标准」判定：被 2+ 模块实现或消费的 DI trait 收录进本文档，代码位于 common crate；仅被单一模块定义和消费的 trait 属于对应领域模块，代码移出 common crate。代码映射规则见 [common README](README.md) 边界规则。

## 架构

### 工具注册与查询

#### ToolRegistrar

**用途**：抽象各模块"我能注册工具"的接口契约。Tools 模块通过收集已注册的 Registrar 并依次调用其注册方法完成全局工具编排。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | Registrar 的唯一名称，用于日志和冲突报告 |
| 优先级 | 数值越小越靠前，决定各模块工具的注册顺序。同等优先级下注册顺序不保证 |
| 注册 | 接收 [ToolRegistry](#toolregistry) 引用，将本模块所有工具一次性注册。工具名冲突时中断启动 |

注册阶段的错误策略：
- **工具名冲突**：ToolRegistry 拒绝注册并报告冲突工具名和双方 Registrar，启动编排层据此中断启动
- **单个 Registrar 内部错误**：由 Registrar 自行处理（跳过无效工具并记录警告，不中断其他工具注册）。Registrar 整体注册失败则报告错误并中断启动

各业务模块通过实现此 trait 注册自身工具。具体 Registrar 实现和编排流程详见 [tool-registrar](../tools/tool-registrar.md)。

#### ToolRegistry

**用途**：全局工具注册中心接口。Tools 模块提供此接口的具体实现。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 注册工具 | 以工具名为键注册工具定义（名称、分组、摘要、行为描述、输入模式、运行时标记）。工具名冲突时拒绝注册 |
| 索引构建 | 按分组聚合已注册工具，生成一级索引字符串。常用工具展示名称、危险度标记和行为描述，延迟加载工具仅展示名称和危险度标记 |
| 工具查询 | 按工具名返回完整详情；按分组名返回该组下所有工具名 |
| 冻结 | 标记注册完成，拒绝后续注册调用。冻结后仅允许查询操作 |

具体实现和工具注册编排流程详见 [tools 模块](../tools/README.md)。

#### ToolRegistryQuery

**用途**：工具注册中心的只读查询接口。Tools 模块的 ToolRegistry 实现，Gateway 的 SessionManager 与 system_prompt 的 System Prompt Builder 消费——按 agent 工具白名单/黑名单查询可用工具清单与描述。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 工具名列表 | 返回所有已注册工具名 |
| 工具描述查询 | 按 agent 白名单/黑名单过滤，返回工具描述（供 system prompt 生成） |
| 工具存在性 | 按名查询工具是否存在 |
| 工具 schema | 按名返回工具的 JSON Schema |
| 工具详情 | 按名返回完整 ToolDescriptor（含摘要） |
| 按分组查询 | 返回某分组下所有工具名 |

#### Tool trait

**用途**：所有工具的统一切入点接口。每个工具实现此 trait，ToolRegistry 通过此接口统一管理工具的标识、描述、输入模式和运行时标记。Tools 模块提供该 trait 的实现说明和工具注册流程。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | `name`：工具名，用于索引和发现；`group`：所属分组，用于索引聚合 |
| 摘要 | `summary`：一句话描述，用于工具列表场景 |
| 行为描述 | `detail`：完整的功能说明。常用工具的行为描述进入一级索引供 LLM 理解工具用途 |
| 动态 prompt 生成 | `generate_prompt`：根据运行时上下文（权限、可用工具、工作目录等）动态调整工具描述，默认实现回退到 `detail`。生成的描述由 System Prompt Builder 注入工具 prompt，由 ToolRegistry 索引构建消费 |
| 参数模式 | `input_schema`：JSON Schema 格式，直接暴露为 API schema |
| 运行时标记 | `flags`：标识工具是否只读、是否破坏性、是否昂贵、是否默认延迟加载、是否并发安全 |

工具注册编排和 Tool trait 的实现规范详见 [tools 模块](../tools/README.md)。

### 工具执行

#### ToolExecutor

**用途**：工具执行抽象接口。tools、gateway 各提供实现，多工具调度器（ToolCallDispatcher）消费——执行单个待调度的工具调用，使调度器可脱离真实 I/O 单测。多工具调度与分组规则详见 [tools multi-tool-calls](../tools/multi-tool-calls.md)。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 执行 | 执行单个 [PendingToolCall](shared-types.md#工具契约载荷族)，返回 [ToolResult](shared-types.md#工具契约载荷族) 或错误 |

### 后台任务管理

#### TaskManager

**用途**：后台任务管理接口。tasks crate 的 BackgroundTaskManager 实现，tools、gateway、daemon 消费——生成与接管后台命令进程、终止任务、查询在途任务与任务通知，使消费方无需直接依赖 tasks crate。后台任务生命周期、通知注入与输出文件回收规则详见 [tools/background-tasks](../tools/background-tasks.md)。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 生成 | 以后台方式启动 shell 命令（携带命令、工作目录、是否经自动/手动后台化、所属 session_id），立即返回任务句柄 [BackgroundTask](shared-types.md#backgroundtask--taskstate--runningtaskinfo--completionnotification--notificationpriority--backgroundtaskerror)，失败返回 [BackgroundTaskError](shared-types.md#backgroundtask--taskstate--runningtaskinfo--completionnotification--notificationpriority--backgroundtaskerror) |
| 接管 | 接管一个已在运行的子进程并纳入后台管理，返回任务句柄 |
| 终止 | 按任务 ID 终止指定任务；任务不存在或非运行态时返回 [BackgroundTaskError](shared-types.md#backgroundtask--taskstate--runningtaskinfo--completionnotification--notificationpriority--backgroundtaskerror) |
| 任务查询 | 按 ID 返回任务句柄；列出全部运行中任务（[RunningTaskInfo](shared-types.md#backgroundtask--taskstate--runningtaskinfo--completionnotification--notificationpriority--backgroundtaskerror) 快照） |
| 通知取出 | 清空并返回待处理的任务通知（[CompletionNotification](shared-types.md#backgroundtask--taskstate--runningtaskinfo--completionnotification--notificationpriority--backgroundtaskerror)，含 [NotificationPriority](shared-types.md#backgroundtask--taskstate--runningtaskinfo--completionnotification--notificationpriority--backgroundtaskerror)） |
| 单命令总时长 | 返回单条命令的最大执行时长上限 |
| 清理 | 回收指定 session 全部终态任务（含被终止任务）的输出文件与句柄 |

### 系统提示词构建

#### PromptFragmentProvider

**用途**：统一抽象 system prompt 静态层各数据来源（bootstrap 文件、ToolRegistry、SkillRegistry、MEMORY.md），System Prompt Builder 通过收集已注册的 Provider 并依次调用组装静态层内容。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | Provider 的唯一名称，用于注册和日志 |
| 优先级 | 数值越小越靠前，决定片段在静态层中的排列顺序 |
| 片段生成 | 根据 [FragmentContext](shared-types.md#fragmentcontext) 产出 [PromptFragment](shared-types.md#promptfragment)。无内容时返回空（文件缺失、agent 无可见 skill 等），Builder 自动跳过 |
| 缓存键 | 片段级缓存的标识。不可缓存时返回空。文件型 Provider 基于文件修改时间生成键，注册表型 Provider 由各自注册表管理失效 |

各业务模块通过实现此 trait 提供系统提示词片段。具体 Provider 实现和注册编排流程详见 [fragment-provider](../system_prompt/fragment-provider.md)。

兜底规则：所有 Provider 均返回空时，系统使用默认 prompt。

无 workspace 目录时，BootstrapFragmentProvider 返回空（该行为由 fragment-provider.md 定义，本 trait 接口仅约定 Provider 返回空的处理规则，不感知具体 Provider 实现）。

#### SystemPromptBuilder

**用途**：系统提示词构建接口。具体 builder 实现负责按会话、agent、覆盖项构建完整系统提示词，session handler 通过 common 的 trait 消费，避免直接依赖 system_prompt 模块。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 构建 | 给定 session_id、agent_id、优先级覆盖项（override/agent/custom）与 bootstrap 模式覆盖，返回渲染后的 system prompt 字符串 |
| 缓存失效 | workspace 文件、工具或技能变化时失效已缓存的 section |

#### DynamicPromptBuilder

**用途**：动态提示词构建接口。system_prompt crate 实现，由 Gateway 注入 session——在请求时生成 `system_static` / `system_dynamic` 两部分，避免对 session crate 的反向依赖。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 构建 | 给定 DynamicPromptContext（会话状态、请求元数据、模式、覆盖项等），返回 `(system_static, system_dynamic)`，任一可为 None |

### Agent 能力查询

#### AgentSkillsQuery

**用途**：按 agent 查询可用技能范围的接口契约。Agent Registry 实现此 trait，Skills 模块消费——根据 agent 的 skills 白名单过滤技能列表。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 输入 | agent_id（或无 agent 上下文时返回全局技能） |
| 查询结果 | 该 agent 可用的技能名列表；白名单为 `["*"]` 或空时表示不限制 |

具体实现和调用链详见 [agent-registry](../agent/agent-registry.md)、[skills 模块](../skills/README.md)。

#### AgentToolsConfigQuery

**用途**：按 agent 查询可用工具范围的接口契约。Agent Registry 实现此 trait，Tools 模块消费——根据 agent 的 tools 白名单和 disallowedTools 黑名单过滤工具列表。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 输入 | agent_id（或无 agent 上下文时返回全局工具） |
| 查询结果 | 可用工具白名单和禁用黑名单；白名单为 `["*"]` 或空时表示不限制；白名单与黑名单交集时黑名单优先 |

具体实现和调用链详见 [agent-registry](../agent/agent-registry.md)、[tools 模块](../tools/README.md)。

#### AgentRegistryQuery

**用途**：agent 注册中心的合并查询接口。agent 的 AgentRegistry 实现，gateway、daemon 消费——以单一 trait 对象同时满足 agent 配置与模型规格（[ModelSpec](shared-types.md#modelspec)）、workspace、bootstrap 模式查询与既有 [AgentSkillsQuery](#agentskillsquery)、[AgentToolsConfigQuery](#agenttoolsconfigquery) 的技能/工具白名单查询，避免消费方直接依赖 agent crate。为 [AgentLookup](#agentlookup)、[AgentSkillsQuery](#agentskillsquery)、[AgentToolsConfigQuery](#agenttoolsconfigquery) 的 supertrait。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 组合 | 自身不新增方法；合并 [AgentLookup](#agentlookup)、[AgentSkillsQuery](#agentskillsquery)、[AgentToolsConfigQuery](#agenttoolsconfigquery) 的方法集，供消费方以单一 `Arc<dyn AgentRegistryQuery>` 满足全部查询需求 |

#### AgentLookup

**用途**：agent 配置查询接口。agent 的 AgentRegistry 实现，system_prompt、gateway 消费——按 agent_id 查询模型规格、agent 是否存在、bootstrap 模式与 per-agent workspace，避免消费方直接依赖 agent crate。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 模型规格 | 按 agent_id 返回该 agent 配置的 [ModelSpec](shared-types.md#modelspec)，未配置返回 None |
| 存在性 | 按 agent_id 判断 agent 是否在注册中心 |
| bootstrap 模式 | 按 agent_id 返回其 [BootstrapMode](shared-types.md#会话注入辅助类型)，未配置返回 None |
| workspace | 按 agent_id 返回其 per-agent workspace 路径，未配置返回 None |

#### AgentConfigLookup

**用途**：agent 最小配置查询接口。agent 的 AgentRegistry 实现，session 的子会话工具与 daemon 消费——按 agent_id 查询子 Agent 生成所需的最小配置子集，避免消费方直接依赖 agent crate 的具体注册中心类型。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 配置查询 | 按 agent_id 返回 [AgentConfigInfo](shared-types.md#agentconfiginfo)（子 Agent 模型规格（[ModelSpec](shared-types.md#modelspec)）、超时告警时长、告警间隔比例），agent 不存在返回 None |

### 消息平台插件

#### IMPlugin

**用途**：统一抽象各消息平台的插件契约。Gateway 通过收集已注册的 IMPlugin 管理跨平台的消息入站解析、出站格式渲染和消息发送。每个消息平台（飞书、Discord、Telegram、Terminal）封装为一个独立插件，实现此 trait 的四个方法分组。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | Plugin 的唯一平台名（如 `"feishu"`、`"terminal"`），用于 Gateway 的 Plugin Registry 路由 |
| 入站 | 解析平台原生事件 payload 为 [NormalizedMessage](shared-types.md#normalizedmessage)。text 类型空 content 消息在解析阶段丢弃；其余消息正常产出 NormalizedMessage（message_type 标记类型，media_refs 承载已落盘的媒体引用，纯媒体消息 content 可为空），由 Gateway 分型处理（消息过滤完整规则见 [shared-types](shared-types.md#normalizedmessage)） |
| 渲染 | 接收 [ContentBlock](shared-types.md#contentblock)[] 和 [DslParseResult](shared-types.md#dslparseresult--dslinstruction)，按平台能力选择输出格式（纯文本或富格式），产出 [RenderedOutput](shared-types.md#renderedoutput)。渲染是纯数据转换，无副作用 |
| 发送 | 接收 [RenderedOutput](shared-types.md#renderedoutput)，以指定目标（peer_id + reply_ref）调用平台发送 API |
| 生命周期 | `init()`：启动时初始化（连接池、token 等），不需要的插件空实现；`shutdown()`：关闭时清理资源，不需要的插件空实现 |

**渲染与发送的分离**：渲染产出数据（RenderedOutput），发送执行副作用。Gateway 在两步之间可插入审计、频率限制等中间件。

**与流式渲染的关系**：上表渲染方法描述批量路径（接收完整 ContentBlock[] + DslParseResult）。流式渲染是独立路径——各平台插件组合持有流式渲染器组件，消费 [StreamEvent](shared-types.md#streamevent) 事件流增量渲染后经本 trait 的发送能力投递，不经过本 trait 的渲染方法（详见 [im_adapter/streaming-render](../im_adapter/streaming-render.md)）。

**平台插件实现**和注册机制详见 [IM Adapter 模块](../im_adapter/README.md)。

**入站身份映射**：IMPlugin 在入站解析时负责填充 [NormalizedMessage](shared-types.md#normalizedmessage) 的全部字段，包括经身份映射（platform + 接收方机器人应用 + sender_id → account_id，见 [IdentityResolver](#identityresolver)）获取 account_id。映射规则和账户配置详见 [config 模块](../config/README.md)。

### 媒体访问

#### MediaStoreAccess

**用途**：媒体存储访问接口。im_adapter 的 MediaStore 实现，gateway、tools 等消费——按 [MediaRef](shared-types.md#normalizedmessage) 解析到本地绝对路径，避免直接依赖 im_adapter 的具体 MediaStore 类型。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 解析 | 给定 MediaRef 返回其本地绝对路径，失败返回 [MediaStoreError](shared-types.md#mediastoreerror) |

媒体落盘与消费机制见 [im_adapter media-store](../im_adapter/media-store.md)。

### 斜杠指令分派与执行

#### SlashRouter

**用途**：斜杠指令路由接口。slash 模块的 SlashDispatcher 实现，Gateway 消费——按内容分派指令、判断立即响应、获取 handler，避免 Gateway 直接依赖 slash 模块。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 分派 | 解析以 `/` 开头的内容，识别指令后返回 SlashResult；内容非斜杠指令时返回 None |
| 立即响应 | 判断某指令是否为 immediate（LLM 忙碌时仍立即响应） |
| handler 查询 | 按指令名返回对应 SlashHandler |

#### SlashHandler

**用途**：斜杠指令处理器接口。各指令 handler 实现，Gateway 通过 SlashRouter 调用——声明处理哪些指令、帮助文本、是否立即响应、是否需权限校验，以及异步执行逻辑。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 指令名 | 声明处理哪些指令（不含前导 `/`） |
| 描述 | 一句话说明，用于 /help 列表 |
| immediate | 是否立即响应（默认否） |
| 权限 | 执行前是否需权限校验（默认否） |
| 执行 | 接收参数与 SlashContext，返回 SlashResult |

#### SlashSessionQuery

**用途**：供斜杠指令 handler 查询会话状态的接口。Gateway 的 SessionManager 实现，slash handler 消费——查计划状态、推送待处理消息、重建系统提示词、读写会话状态，打破 slash → gateway 的依赖。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 计划状态 | 读取/更新会话的 PlanState |
| 待处理消息 | 向统一消息队列推送一条待处理消息（排队规则见 [session 统一消息队列](../session/session-execution.md#统一消息队列)） |
| 后台触发 | 触发会话的手动后台执行 |
| workflow 状态 | 设置并持久化 workflow run（类型擦除，避免依赖 workflow crate） |
| 系统提示词 | 失效静态层缓存、重建会话 system prompt、追加 system append |
| 会话状态查询 | model、reasoning、verbosity、mode、workdir、LLM busy、token 统计、缓存断裂通知、子会话句柄数 |

#### SlashEffectExecutor

**用途**：斜杠指令副作用执行接口。Gateway 实现（拥有完整 SessionManager 与 SessionMessageHandler），SlashResult 执行流程消费——停止、建新会话、压缩、系统提示词操作、设置模式/推理深度/信息展示等级、执行 shell 命令。common 定义接口、gateway 提供实现，打破循环依赖。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 停止 | 停止当前 LLM turn（支持级联与强制） |
| 新会话 | 为指定渠道创建新会话，返回新 session_id |
| 压缩 | 触发上下文压缩（可携带自定义指令） |
| 系统提示词 | 应用 append/clear 动作，返回相关计数 |
| 模式/推理深度/信息展示等级 | 设置会话模式、推理深度档位（[ReasoningLevel](shared-types.md#reasoninglevel--agentrole--sessionmode)）、信息展示等级（[VerbosityLevel](shared-types.md#verbositylevel)） |
| shell 执行 | 以指定 agent 执行命令，权限由 Gateway 层先行校验 |

#### SlashResultExecutor

**用途**：SlashResult 的扩展执行 trait。为 SlashResult 实现，Gateway 构造 SideEffectContext 后调用 `execute()` 触发副作用分发与回复。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 执行 | 接收 SideEffectContext，按 SlashResult 变体分发到对应副作用并回发 ReplyAction |

### 计划执行确认

#### PlanConfirmationHandler

**用途**：计划执行确认/取消接口。tools 的 PlanExecConfirmFlow 实现，Gateway 消费——驱动计划执行启动确认的确认或取消分支，避免 Gateway 直接依赖 tools crate。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 确认 | 确认待处理的计划执行，返回是否处理成功 |
| 取消 | 取消待处理的计划执行，返回是否取消成功 |

### 权限评估与审批

#### PermissionEvaluator

**用途**：跨 agent 权限请求评估接口。daemon 的 adapter 包装 permission 的 PermissionEngine 实现，session tools 消费——评估 agent 间消息权限，避免直接依赖 permission 模块。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 评估 | 评估 from→to 的 agent 间消息，返回 [PermissionEvalResponse](shared-types.md#risklevel--permissionevalresponse--callerinfo--permissiondenied--spawnpermissionerror)（Allowed 或 Denied，含原因 + [RiskLevel](shared-types.md#risklevel--permissionevalresponse--callerinfo--permissiondenied--spawnpermissionerror)） |

#### PermissionChecker

**用途**：子 agent 生成权限校验接口。Gateway 实现（包装 PermissionEngine），session 消费——校验子 agent 是否可在父会话下 spawn，避免 session → permission 循环依赖。

**接口契约**：

| 要素 | 说明 |
|------|------|
| spawn 校验 | 校验 child_agent_id 是否允许在 parent_session_id 下 spawn，返回 Ok 或 [SpawnPermissionError::Denied](shared-types.md#risklevel--permissionevalresponse--callerinfo--permissiondenied--spawnpermissionerror)（含原因） |

#### ApprovalSubmission

**用途**：权限拒绝提交审批流接口。daemon 的 adapter 包装 permission 的 ApprovalFlow 实现，session tools 消费——将拒绝的 agent 间请求提交 owner 审批。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 提交审批 | 提交拒绝的 agent 间请求（携带 [CallerInfo](shared-types.md#risklevel--permissionevalresponse--callerinfo--permissiondenied--spawnpermissionerror) + [RiskLevel](shared-types.md#risklevel--permissionevalresponse--callerinfo--permissiondenied--spawnpermissionerror)），返回 request_id；被拒（子 agent 或重复）返回 None |

> **共享句柄别名**：上述 trait 以 `Arc<dyn Trait>` 形式跨模块传递时以类型别名暴露——SharedPermissionEvaluator、SharedApprovalSubmission（带互斥包装）。别名与对应 trait 同属 common。

#### AuditLogger

**用途**：审计日志记录接口。permission 的文件审计日志实现，permission 权限引擎与 daemon 消费——记录危险操作审批/拒绝的结构化审计条目（操作内容、风险级别与最终处置），使审计写入与权限判定解耦。审计的生成时机与查看需求见 [mode 需求](../requirements/mode.md)；审计条目的查看由具体文件日志实现承担，不在本 trait 契约内。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 记录 | 写入一条 [AuditLogEntry](shared-types.md#auditlogentry--auditdisposition)（含处置 [AuditDisposition](shared-types.md#auditlogentry--auditdisposition)） |

### 会话查询与生命周期

#### SessionLookup

**用途**：会话关系与待处理消息接口。Gateway 的 SessionManager 实现，permission 与 slash 消费——查询父/子会话关系、聊天 ID、计划状态，并向统一消息队列推送待处理消息，避免直接依赖 gateway。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 父会话查询 | 给定子会话 ID 返回父会话 ID |
| 聊天 ID 查询 | 给定会话 ID 返回关联聊天 ID |
| 待处理消息 | 向统一消息队列推送一条待处理消息（排队规则见 [session 统一消息队列](../session/session-execution.md#统一消息队列)） |
| 计划状态 | 读取/更新会话的 PlanState |
| 会话模式切换 | 设置会话模式（如 plan → auto） |

#### SessionModeQuery

**用途**：会话模式查询接口。session 模块桥接实现，permission 消费——按 agent 查询当前 SessionMode，避免硬依赖 session 模块。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 模式查询 | 给定 agent_id 返回当前 SessionMode，未知返回 None（同步，内存级查询） |

#### SpawnValidator

**用途**：子会话生成校验接口。session 的 SpawnController 实现，tools（子会话管理工具）、daemon（组合根装配注入）消费——校验子会话生成的前置条件与权限，使工具侧无需直接依赖 SpawnController 具体类型。前置校验与权限校验两步分离，各自独立。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 前置校验 | 给定父 session_id 与目标 agent_id（可空），校验深度、并发、目标 agent 解析与 allowlist，返回 [SpawnValidationResult](shared-types.md#spawnvalidationresult--spawnerror)（目标 agent 标识 + 子会话可用的最大生成深度、执行超时、超时告警、告警间隔比例等派生参数）；失败返回 [SpawnError](shared-types.md#spawnvalidationresult--spawnerror)（不含权限——权限为独立一步） |
| 权限校验 | 前置校验通过后执行，校验子 agent 是否可在父会话下生成（权限判定语义见 [permission 模块](../permission/README.md)），返回 Ok 或 [SpawnError](shared-types.md#spawnvalidationresult--spawnerror) 的权限变体（权限被拒） |

两步统一返回 [SpawnError](shared-types.md#spawnvalidationresult--spawnerror)：前置校验失败为其各前置变体，权限被拒为 `Permission` 变体。权限校验步经 [PermissionChecker](#permissionchecker) 的权限引擎边界完成（载荷复用既有的 [SpawnPermissionError](shared-types.md#risklevel--permissionevalresponse--callerinfo--permissiondenied--spawnpermissionerror)，不重复定义拒绝载荷）：SpawnValidator 是子会话生成的高层门面，PermissionChecker 是权限引擎边界的窄接口。

#### KillHandle

**用途**：工具进程终止适配器。tools 的前后台进程适配器实现，session 消费——终止在途工具进程，避免循环依赖。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 终止 | 请求终止底层进程/任务，幂等（重复调用也返回成功）。调用方不等待其实际退出——一次停止对本次涉及的全部工具终止施加单一 wall-clock 预算；预算内未完成的终止视为失败，其后不论正常返回、返回错误还是 panic，均不影响停止流程的完成 |

#### ToolSession

**用途**：工具会话注册接口。ConversationSession 包装实现（session 模块），tools 的 ToolContext 消费——注册/注销工具 kill handle 与 pending 状态、持久化 checkpoint、文件读取去重、进度上报、waiting 状态，使 Tool trait 无需依赖 ConversationSession。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 句柄注册 | 注册工具 kill handle，及工具调用/子会话的 pending 状态 |
| checkpoint | 持久化当前 pending 操作（崩溃恢复用） |
| waiting 状态 | 进入/退出 active waiting、查询是否 waiting |
| 文件读取 | 记录/查询文件 mtime 与 per-turn 读取去重缓存 |
| 进度上报 | 上报工具实时执行进度（默认空） |
| 子会话 | 注册/注销子会话状态、查询是否有子会话运行 |
| 手动后台 | 返回手动后台化通知信号（不支持时返回 None） |

### LLM 调用与流式渲染

#### LlmCaller

**用途**：LLM 调用抽象接口。具体实现由 gateway 提供，daemon 启动时经 SessionManager 注入会话，session、daemon、memory 消费——发起流式/非流式 LLM 请求。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 非流式调用 | 接收 [InternalRequest](shared-types.md#internalrequest--internalmessage--systemblock--tooldefinition) 返回 [UnifiedResponse](shared-types.md#unifiedresponse--unifiedusage) |
| 流式调用 | 返回 [StreamEvent](shared-types.md#streamevent) 流（逐项携带成功事件或 LLMError） |
| 默认请求头 | 返回 provider 默认请求头，用于 prompt 指纹检测缓存断裂；敏感头（Authorization、api-key 等）值替换为占位符 |

#### StreamingSink

**用途**：平台无关的流式输出 sink。各传输实现（飞书卡片更新、CLI stdout 等），session 持有 handle 并推送增量文本、完成通知（携带 model + usage）、错误通知。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 文本增量 | 逐 delta 推送增量文本（实现须非阻塞） |
| 完成通知 | 流成功结束时调用一次，此后无更多文本增量 |
| 错误通知 | 流失败时最多调用一次，此后无更多通知 |

#### StreamingRenderer

**用途**：LLM [StreamEvent](shared-types.md#streamevent) 流的增量渲染接口。trait 定义与默认实现（DefaultStreamingRenderer / LineBuffer）位于 common，实现方为 common 自身；im_adapter、cli 各平台适配器作为消费方持有并委托调用、按需覆盖差异化渲染行为（见 [im_adapter/streaming-render](../im_adapter/streaming-render.md)），逐事件处理事件流、产出增量 [StreamingOutput](shared-types.md#streamingoutput)。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 事件处理 | 处理单个 StreamEvent 返回增量输出 |
| 刷新 | MessageEnd 时清空残留缓冲内容 |
| 超时检查 | 超时则强制输出缓冲内容（默认返回空） |

### 消息处理链与出站中间件

#### ProcessorChain

**用途**：入站/出站消息处理链接口。processor_chain 的 ProcessorRegistry 实现，Gateway 消费——运行入站/出站处理链，避免直接依赖 processor_chain 内部。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 入站处理 | NormalizedMessage → ProcessedMessage |
| 出站处理 | ProcessedMessage → ProcessedMessage |
| DSL 行解析 | 解析单行文本的 DSL 指令，返回清洗文本 + 解析结果（默认零开销透传） |

#### OutboundMiddleware

**用途**：出站消息中间件接口。各中间件实现，Gateway 在 IMPlugin 渲染与发送之间调用——检查出站消息，允许或拒绝（审计、频率限制等）。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 名称 | 中间件名称，用于日志与错误报告 |
| 处理 | 检查渲染后消息，Ok 放行、Rejected 拒绝；不得修改消息内容 |
| 流式预检 | 流式出站前一次性预检（默认放行，避免逐 chunk 开销） |

### 技能查询

#### SkillListingProvider

**用途**：技能清单生成接口。daemon 层包装 DiskSkillRegistry 实现，session 消费——按 agent 生成技能清单文本、条件技能匹配，避免 session 依赖 skills。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 清单生成 | 按 agent/白名单生成格式化技能清单（无匹配返回空） |
| 排除条件技能 | 生成不含条件技能的清单（初始 turn / 增量 diff 基准） |
| 条件匹配 | 按文件路径 glob 匹配条件技能，返回带 ⚡ 注解的清单行 |

#### SkillRegistryQuery

**用途**：技能注册表查询接口。daemon 的 SkillRegistryWrapper 包装 skills 的 DiskSkillRegistry 实现，Gateway 的 SessionManager 消费——查询可用技能、按 agent 白名单过滤、生成 SP 注入清单。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 技能存在 | 按名查询技能是否存在 |
| 技能列表 | 列出全部技能名 / 按 agent 白名单过滤 |
| 清单生成 | 生成格式化技能清单用于 system prompt 注入（无匹配返回空） |

### 观测与协调

#### MetricsEmitter

**用途**：运营指标上报接口。DI trait（归属 gateway 领域），默认 NoopMetricsEmitter 为零成本空操作，gateway、daemon 消费。指标后端只需实现此 trait，无需改动调用点。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 缓存断裂上报 | 记录 KV cache break 事件 |

#### IdentityResolver

**用途**：平台身份解析接口。config 支持的 ConfigIdentityResolver 实现，im_adapter 消费——将 `(platform, bot_app_id, sender_id)` 解析为本地 account_id。接收方机器人应用（bot_app_id）参与映射键：IM 平台的发送者标识按「应用 × 发送者」隔离（同一用户在不同应用语境下标识不同），跨应用 ID 不可直接互换。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 解析 | 给定 platform + bot_app_id + sender_id 返回 account_id，无映射返回 None（启动时构造、运行期只读） |

#### ShutdownSignal

**用途**：关停信号抽象接口。daemon 的 ShutdownHandle 实现（daemon 启动时创建；gateway 侧为转发包装），llm、session 等消费——查询关停状态（[ShutdownState](shared-types.md#shutdownstate--shutdownmode--drainstatus)）、忙计数、graceful→forceful 升级（[ShutdownMode](shared-types.md#shutdownstate--shutdownmode--drainstatus)）、drain 快照（[DrainStatus](shared-types.md#shutdownstate--shutdownmode--drainstatus)），使消费方无需直接依赖 daemon 模块。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 关停查询 | 是否已发起关停、是否已升级 forceful |
| 忙计数 | 忙计数增减与查询（可携带描述跟踪） |
| 升级 | graceful 原子升级为 forceful |
| drain 快照 | 返回结构化 drain 状态（状态 + 忙计数 + 是否正在 drain + 待处理项描述） |

## 数据流

core-traits 不定义具体的业务数据流。以下描述各 trait 实现方在依赖注入后的典型调用路径，供模块开发者理解接口在系统中的运转方式。详细数据流见各业务模块文档。

### PromptFragmentProvider 注册与调用

System Prompt Builder 收集已注册 Provider → 按优先级排序 → 依次请求片段（传入 FragmentContext）→ 跳过空返回 → 拼接产出静态层文本 → 写入 ConversationSession 的 system prompt 字段。

完整数据流（含字段级详解、缓存策略）见 [fragment-provider](../system_prompt/fragment-provider.md)，核心数据结构的产出链路见 [shared-types 数据流](shared-types.md#数据流)。

### ToolRegistrar 注册与编排

1. 系统启动 → Tools 模块收集所有 ToolRegistrar 实现者 → 按优先级排序
2. 依次调用各 Registrar → 向 [ToolRegistry](#toolregistry) 注册工具 → 注册完成 → ToolRegistry 冻结
3. 后续流程（索引构建、工具发现、system prompt 注入）照常进行

### IMPlugin 入站与出站

Gateway 通过 Plugin Registry 按平台名路由 → IMPlugin 解析入站 payload → 产出 NormalizedMessage → 进入 Processor Chain → 出站时 Gateway 将 ContentBlock[] 和 DslParseResult 传给同平台 IMPlugin 渲染为 RenderedOutput → 中间件插入点（审计、频率限制）→ IMPlugin 发送到平台。

完整数据流（入站链路含 account_id 映射、出站链路含平台渲染差异）见 [shared-types 数据流](shared-types.md#数据流)。

其余 trait 遵循同一通用模式——「实现方在启动/依赖注入时注册，消费方在运行时调用」，具体调用路径见各业务模块文档，不在本文档展开。

## 模块关系

- **上游**：无（common 不依赖任何其他模块，是纯定义基底层）
- **下游**：
  - **system_prompt**（实现 PromptFragmentProvider、SystemPromptBuilder、DynamicPromptBuilder；消费 ToolRegistryQuery、SkillListingProvider、AgentLookup；System Prompt Builder 收集所有 Provider 并触发生成）
  - **tools**（实现 PromptFragmentProvider、ToolRegistrar、ToolRegistry、ToolRegistryQuery、Tool trait、KillHandle、PlanConfirmationHandler、ToolExecutor；消费 ToolSession、AgentToolsConfigQuery、MediaStoreAccess、TaskManager、SpawnValidator）
  - **session**（实现 ToolRegistrar、SessionModeQuery、ToolSession、SpawnValidator；消费 PermissionChecker、PermissionEvaluator、ApprovalSubmission、KillHandle、SkillListingProvider、StreamingSink、LlmCaller、SystemPromptBuilder、DynamicPromptBuilder、ShutdownSignal、AgentConfigLookup）
  - **skills**（实现 PromptFragmentProvider、ToolRegistrar；消费 AgentSkillsQuery）
  - **agent**（实现 AgentSkillsQuery、AgentToolsConfigQuery、AgentRegistryQuery、AgentLookup、AgentConfigLookup）
  - **tasks**（实现 TaskManager；后台任务执行见 [tools/background-tasks](../tools/background-tasks.md)）
  - **memory**（实现 PromptFragmentProvider；消费 LlmCaller）
  - **im_adapter**（实现 ToolRegistrar、IMPlugin、MediaStoreAccess；消费 IdentityResolver、StreamingRenderer）
  - **gateway**（实现 LlmCaller、MetricsEmitter、OutboundMiddleware、SlashEffectExecutor、SlashSessionQuery、SessionLookup、PermissionChecker、ToolExecutor；消费 IMPlugin、SlashRouter、ProcessorChain、OutboundMiddleware、ToolRegistryQuery、SkillRegistryQuery、SlashResultExecutor、DynamicPromptBuilder、SystemPromptBuilder、MediaStoreAccess、PlanConfirmationHandler、TaskManager、AgentRegistryQuery）
  - **cli**（实现 IMPlugin；消费 StreamingRenderer）
  - **slash**（实现 SlashRouter、SlashHandler；消费 SlashSessionQuery、SessionLookup）
  - **permission**（实现 AuditLogger；消费 SessionLookup、SessionModeQuery）
  - **processor_chain**（实现 ProcessorChain）
  - **daemon**（实现 SkillRegistryQuery、SkillListingProvider、PermissionEvaluator、ApprovalSubmission、ShutdownSignal；消费 LlmCaller、MetricsEmitter、TaskManager、SpawnValidator、AgentConfigLookup、AgentRegistryQuery、AuditLogger）
  - **config**（实现 IdentityResolver）
  - **llm**（消费 ShutdownSignal）
- **无关**：无。core-traits 的每个 trait 均至少被一个业务模块实现或消费；workflow、mode 不实现也不消费本文档收录的 core-trait，但经 shared-types 中的共享类型（PlanState 等）与 common 建立数据流关联，故不计为「无关联」的无关模块。
