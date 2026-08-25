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
| 索引构建 | 按分组聚合已注册工具，生成一级索引字符串。常用工具展示名称和行为描述，延迟加载工具仅展示名称和危险度标记 |
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

### 消息平台插件

#### IMPlugin

**用途**：统一抽象各消息平台的插件契约。Gateway 通过收集已注册的 IMPlugin 管理跨平台的消息入站解析、出站格式渲染和消息发送。每个消息平台（飞书、Discord、Telegram、Terminal）封装为一个独立插件，实现此 trait 的四个方法分组。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | Plugin 的唯一平台名（如 `"feishu"`、`"terminal"`），用于 Gateway 的 Plugin Registry 路由 |
| 入站 | 解析平台原生 webhook/事件 payload 为 [NormalizedMessage](shared-types.md#normalizedmessage)。text 类型空 content 消息在解析阶段丢弃，非文本消息（image/file/audio）正常产出 NormalizedMessage（message_type 标记类型，media_refs 存储引用，content 可为空） |
| 渲染 | 接收 [ContentBlock](shared-types.md#contentblock)[] 和 [DslParseResult](shared-types.md#dslparseresult--dslinstruction)，按平台能力选择输出格式（纯文本或富格式），产出 [RenderedOutput](shared-types.md#renderedoutput)。渲染是纯数据转换，无副作用 |
| 发送 | 接收 [RenderedOutput](shared-types.md#renderedoutput)，以指定目标（peer_id + thread_id）调用平台发送 API |
| 生命周期 | `init()`：启动时初始化（连接池、token 等），不需要的插件空实现；`shutdown()`：关闭时清理资源，不需要的插件空实现 |

**渲染与发送的分离**：渲染产出数据（RenderedOutput），发送执行副作用。Gateway 在两步之间可插入审计、频率限制等中间件。

**与流式渲染的关系**：上表渲染方法描述批量路径（接收完整 ContentBlock[] + DslParseResult）。流式渲染是独立路径——各平台插件组合持有流式渲染器组件，消费 [StreamEvent](shared-types.md#streamevent) 事件流增量渲染后经本 trait 的发送能力投递，不经过本 trait 的渲染方法（详见 [im_adapter/streaming-render](../im_adapter/streaming-render.md)）。

**平台插件实现**和注册机制详见 [IM Adapter 模块](../im_adapter/README.md)。

**入站身份映射**：IMPlugin 在入站解析时负责填充 [NormalizedMessage](shared-types.md#normalizedmessage) 的全部字段，包括通过 sender_id 查询账户绑定表获取 account_id。映射规则和账户配置详见 [config 模块](../config/README.md)。

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
| 待处理消息 | 向会话队列推送 PendingMessage |
| 后台触发 | 触发会话的手动后台执行 |
| workflow 状态 | 设置并持久化 workflow run（类型擦除，避免依赖 workflow crate） |
| 系统提示词 | 失效静态层缓存、重建会话 system prompt、追加 system append |
| 会话状态查询 | model、reasoning、verbosity、mode、workdir、LLM busy、token 统计、缓存断裂通知、子会话句柄数 |

#### SlashEffectExecutor

**用途**：斜杠指令副作用执行接口。Gateway 实现（拥有完整 SessionManager 与 SessionMessageHandler），SlashResult 执行流程消费——停止、建新会话、压缩、系统提示词操作、设置模式/推理/详细度、执行 shell 命令。common 定义接口、gateway 提供实现，打破循环依赖。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 停止 | 停止当前 LLM turn（支持级联与强制） |
| 新会话 | 为指定渠道创建新会话，返回新 session_id |
| 压缩 | 触发上下文压缩（可携带自定义指令） |
| 系统提示词 | 应用 append/clear 动作，返回相关计数 |
| 模式/推理/详细度 | 设置会话模式、推理深度、输出详细度 |
| shell 执行 | 以指定 agent 执行命令，权限由 Gateway 层先行校验 |

#### SlashResultExecutor

**用途**：SlashResult 的扩展执行 trait。为 SlashResult 实现，Gateway 构造 SideEffectContext 后调用 `execute()` 触发副作用分发与回复。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 执行 | 接收 SideEffectContext，按 SlashResult 变体分发到对应副作用并回发 ReplyAction |

### 权限评估与审批

#### PermissionEvaluator

**用途**：跨 agent 权限请求评估接口。daemon 的 adapter 包装 permission 的 PermissionEngine 实现，session tools 消费——评估 agent 间消息权限，避免直接依赖 permission 模块。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 评估 | 评估 from→to 的 agent 间消息，返回 Allowed 或 Denied（原因 + 风险级别） |

#### PermissionChecker

**用途**：子 agent 生成权限校验接口。Gateway 实现（包装 PermissionEngine），session 消费——校验子 agent 是否可在父会话下 spawn，避免 session → permission 循环依赖。

**接口契约**：

| 要素 | 说明 |
|------|------|
| spawn 校验 | 校验 child_agent_id 是否允许在 parent_session_id 下 spawn，返回 Ok 或 Denied（原因） |

#### ApprovalSubmission

**用途**：权限拒绝提交审批流接口。daemon 的 adapter 包装 permission 的 ApprovalFlow 实现，session tools 消费——将拒绝的 agent 间请求提交 owner 审批。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 提交审批 | 提交拒绝的 agent 间请求，返回 request_id；被拒（子 agent 或重复）返回 None |

### 会话查询与生命周期

#### SessionLookup

**用途**：会话关系与待处理消息查询接口。Gateway 的 SessionManager 实现，permission 与 slash 消费——查询父/子会话关系、聊天 ID、计划状态，避免直接依赖 gateway。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 父会话查询 | 给定子会话 ID 返回父会话 ID |
| 聊天 ID 查询 | 给定会话 ID 返回关联聊天 ID |
| 待处理消息 | 向会话队列推送 PendingMessage |
| 计划状态 | 读取/更新会话的 PlanState |
| 会话模式切换 | 设置会话模式（如 plan → auto） |

#### SessionModeQuery

**用途**：会话模式查询接口。session 模块桥接实现，permission 消费——按 agent 查询当前 SessionMode，避免硬依赖 session 模块。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 模式查询 | 给定 agent_id 返回当前 SessionMode，未知返回 None（同步，内存级查询） |

#### KillHandle

**用途**：工具进程终止适配器。tools 的前后台进程适配器实现，LLM/session 消费——终止在途工具进程，避免循环依赖。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 终止 | 请求终止底层进程/任务，幂等（重复调用也返回成功）；调用方不等待实际退出，由 stop 路径经 wall-clock 预算兜底 |

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

**用途**：LLM 调用抽象接口。gateway 实现（FallbackLlmCaller 桥接 UnifiedFallbackClient / UnifiedChatClient，因 session 不能依赖 llm 的循环依赖），gateway、daemon、memory 消费——发起流式/非流式 LLM 请求。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 非流式调用 | 接收 InternalRequest 返回 UnifiedResponse |
| 流式调用 | 返回 StreamEvent 流（逐项携带成功事件或 LLMError） |
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

**用途**：LLM StreamEvent 流的增量渲染接口。各平台流式渲染组件实现，逐事件产出增量 RenderedOutput。

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

**用途**：平台身份解析接口。config 支持的 ConfigIdentityResolver 实现，im_adapter/gateway 消费——将 `(platform, sender_id)` 解析为本地 account_id。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 解析 | 给定 platform + sender_id 返回 account_id，无映射返回 None（启动时构造、运行期只读） |

#### ShutdownSignal

**用途**：关停信号抽象接口。gateway 的 ShutdownHandle 实现，llm 模块消费——查询关停状态、忙计数、graceful→forceful 升级、drain 快照，避免 llm 依赖 gateway。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 关停查询 | 是否已发起关停、是否已升级 forceful |
| 忙计数 | 忙计数增减与查询（可携带描述跟踪） |
| 升级 | graceful 原子升级为 forceful |
| drain 快照 | 返回结构化 drain 状态（状态 + 忙计数 + 待处理项描述） |

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
  - **system_prompt**（实现 PromptFragmentProvider、SystemPromptBuilder、DynamicPromptBuilder；System Prompt Builder 收集所有 Provider 并触发生成）
  - **tools**（实现 PromptFragmentProvider、ToolRegistrar、ToolRegistry、ToolRegistryQuery、Tool trait、KillHandle；消费 ToolSession、AgentToolsConfigQuery）
  - **session**（实现 ToolRegistrar、SessionModeQuery；消费 PermissionChecker、ToolSession、KillHandle、SkillListingProvider、StreamingSink）
  - **skills**（实现 PromptFragmentProvider、ToolRegistrar；消费 AgentSkillsQuery）
  - **agent**（实现 AgentSkillsQuery、AgentToolsConfigQuery）
  - **memory**（实现 PromptFragmentProvider；消费 LlmCaller）
  - **im_adapter**（实现 ToolRegistrar、IMPlugin、StreamingRenderer；消费 IdentityResolver）
  - **gateway**（实现 LlmCaller、MetricsEmitter、OutboundMiddleware、SlashEffectExecutor、SlashSessionQuery、SessionLookup、PermissionChecker、ShutdownSignal；消费 IMPlugin、SlashRouter、ProcessorChain、OutboundMiddleware、ToolRegistryQuery、SkillRegistryQuery、SlashResultExecutor）
  - **cli**（实现 IMPlugin）
  - **slash**（实现 SlashRouter、SlashHandler；消费 SlashSessionQuery、SessionLookup）
  - **permission**（消费 SessionLookup、SessionModeQuery）
  - **processor_chain**（实现 ProcessorChain）
  - **daemon**（实现 SkillRegistryQuery、SkillListingProvider、PermissionEvaluator、ApprovalSubmission；消费 LlmCaller、MetricsEmitter）
  - **config**（实现 IdentityResolver）
  - **llm**（消费 ShutdownSignal）
- **无关**：无（common 的 trait 与各业务模块均存在实现或消费关系，不存在无关模块）
