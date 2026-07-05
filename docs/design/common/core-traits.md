# 核心 trait

## 概述

核心 trait 是跨模块依赖注入的接口契约。每个 trait 在本文档中唯一定义其完整接口，各业务模块文档通过引用指向此处，不在自身文档中重复定义 trait 签名。

> **本文档是 common crate 中 DI trait 的权威清单。** 若代码中 common crate 存在本文档未收录的 pub trait，该 trait 不属于 common 的跨模块 DI 接口，应移至对应领域模块的 crate。反之，本文档定义的所有 trait 和接口，代码中均位于 common crate（或其子 crate）。各业务模块文档通过引用指向此处，不在自身文档中重复定义本文档已收录的 trait。

## 架构

### PromptFragmentProvider

**用途**：统一抽象 system prompt 静态层各数据来源（bootstrap 文件、ToolRegistry、DiskSkillRegistry、MEMORY.md），System Prompt Builder 通过收集已注册的 Provider 并依次调用组装静态层内容。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | Provider 的唯一名称，用于注册和日志 |
| 优先级 | 数值越小越靠前，决定片段在静态层中的排列顺序 |
| 片段生成 | 根据 [FragmentContext](shared-types/prompt-fragment.md) 产出 [PromptFragment](shared-types/prompt-fragment.md)。无内容时返回空（文件缺失、agent 无可见 skill 等），Builder 自动跳过 |
| 缓存键 | Section 级缓存的标识。不可缓存时返回空。文件型 Provider 基于文件修改时间生成键，注册表型 Provider 由各自注册表管理失效 |

四个标准 Provider（BootstrapFragmentProvider / ToolsFragmentProvider / SkillsFragmentProvider / MemoryFragmentProvider）的定义和 Provider 注册编排流程详见 [fragment-provider](../system_prompt/fragment-provider.md)。

兜底规则：所有 Provider 均返回空时，使用默认 prompt。无 workspace 目录时 BootstrapFragmentProvider 返回空，静态层仅含工具和 skill 片段。

### ToolRegistrar

**用途**：抽象各模块"我能注册工具"的接口契约。Tools 模块通过收集已注册的 Registrar 并依次调用其注册方法完成全局工具编排。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | Registrar 的唯一名称，用于日志和冲突报告 |
| 优先级 | 数值越小越靠前，决定各模块工具的注册顺序。同等优先级下注册顺序不保证 |
| 注册 | 接收 [ToolRegistry](#toolregistry) 引用，将本模块所有工具一次性注册。工具名冲突时中断启动 |

注册阶段的错误策略：
- **工具名冲突**：报告冲突工具名和双方 Registrar，中断启动
- **单个 Registrar 内部错误**：由 Registrar 自行处理（跳过无效工具并记录警告，不中断其他工具注册）。Registrar 整体注册失败则报告错误

四个标准 Registrar（CoreToolsRegistrar / SessionToolsRegistrar / SkillsToolsRegistrar / ImAdapterToolsRegistrar）的定义和编排流程详见 [tool-registrar](../tools/tool-registrar.md)。

### ToolRegistry

**用途**：全局工具注册中心接口。Tools 模块提供此接口的具体实现。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 注册工具 | 以工具名为键注册工具定义（名称、分组、摘要、行为描述、输入模式、运行时标记）。工具名冲突时拒绝注册 |
| 索引构建 | 按分组聚合已注册工具，生成一级索引字符串。常用工具展示名称和行为描述，延迟加载工具仅展示名称和危险度标记 |
| 工具查询 | 按工具名返回完整详情；按分组名返回该组下所有工具名 |
| 冻结 | 标记注册完成，拒绝后续注册调用。冻结后仅允许查询操作 |

具体实现和工具注册编排流程详见 [tools 模块](../tools/README.md)。

### Tool trait

**用途**：所有工具的统一切入点接口。每个工具实现此 trait，ToolRegistry 通过此接口统一管理工具的标识、描述和输入模式。Tools 模块提供此 trait 的具体定义。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | `name`：工具名，用于索引和发现；`group`：所属分组，用于索引聚合 |
| 摘要 | `summary`：一句话描述，用于工具列表场景 |
| 行为描述 | `detail`：完整的功能说明。常用工具的行为描述进入一级索引供 LLM 理解工具用途 |
| 动态 prompt 生成 | `generate_prompt`：根据运行时上下文（权限、可用工具、工作目录等）动态调整工具描述，默认实现回退到 `detail` |
| 参数模式 | `input_schema`：JSON Schema 格式，直接暴露为 API schema |
| 运行时标记 | `flags`：标识工具是否只读、是否破坏性、是否昂贵、是否默认延迟加载、是否并发安全 |

工具注册编排和 Tool trait 的实现规范详见 [tools 模块](../tools/README.md)。

### IMPlugin

**用途**：统一抽象各消息平台的插件契约。Gateway 通过收集已注册的 IMPlugin 管理跨平台的消息入站解析、出站格式渲染和消息发送。每个消息平台（飞书、Discord、Telegram、Terminal）封装为一个独立插件，实现此 trait 的四个方法分组。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | Plugin 的唯一平台名（如 `"feishu"`、`"terminal"`），用于 Gateway 的 Plugin Registry 路由 |
| 入站 | 解析平台原生 webhook/事件 payload 为 [NormalizedMessage](shared-types/inbound-message.md)。空内容消息在解析阶段丢弃，非文本消息正常产出 NormalizedMessage（message_type 标记类型，media_refs 存储引用） |
| 渲染 | 接收 [ContentBlock](shared-types/content-block.md)[] 和 [DslParseResult](shared-types/dsl-parse-result.md)，按平台能力选择输出格式（纯文本或富格式），产出 [RenderedOutput](shared-types/rendered-output.md)。渲染是纯数据转换，无副作用 |
| 发送 | 接收 [RenderedOutput](shared-types/rendered-output.md)，以指定目标（peer_id + thread_id）调用平台发送 API |
| 生命周期 | `init()`：启动时初始化（连接池、token 等），不需要的插件空实现；`shutdown()`：关闭时清理资源，不需要的插件空实现 |

**渲染与发送的分离**：渲染产出数据（RenderedOutput），发送执行副作用。Gateway 在两步之间可插入审计、频率限制等中间件。

**平台插件实现**和注册机制详见 [IM Adapter 模块](../im_adapter/README.md)。

**入站身份映射**：IMPlugin 在入站解析时负责填充 [NormalizedMessage](shared-types/inbound-message.md) 的全部字段，包括通过 sender_id 查询账户绑定表获取 account_id。映射规则和账户配置详见 [config 模块](../config/README.md)。

### AgentConfigLookup

**用途**：Agent 配置查询接口。按 agent_id 查询对应 agent 的完整配置信息。由 Agent 模块实现，Daemon、System Prompt Builder、Tools 等模块通过 DI 消费此接口获取 agent 级配置。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 配置查询 | 按 `agent_id` 查询 AgentConfig，返回 `Option<AgentConfig>`。agent_id 不存在时返回 None |
| 默认配置 | 提供当前生效的默认配置快照，用于未匹配特定 agent 的 fallback 场景 |

**关键规则**：
- 配置查询对 agent_id 大小写敏感
- 配置内容在系统运行期间缓存在 Agent 模块内部，查询不涉及 I/O
- Daemon 启动时由 Agent 模块完成配置加载并注册到 DI 容器

### AgentLookup

**用途**：Agent 查找接口。列出所有可用 agent 及其基本信息，供 Gateway 路由入站消息到目标 agent、CLI 交互等场景消费。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 列表查询 | 返回所有可用 agent 的元信息列表，每个元素含 `agent_id`、`name`、`version`、`status` |
| 单例查询 | 按 `agent_id` 查询单个 agent 的完整元信息，返回 `Option<AgentMeta>` |
| 可用性 | 检查指定 agent 是否可用（已加载且状态正常） |

**关键规则**：
- 列表查询返回当前运行中可用的 agent，不含已禁用或加载失败的 agent
- 单例查询返回 `None` 表示 agent_id 不存在或当前不可用
- Agent 模块在 Daemon 启动时完成加载和注册，运行时列表不变（除非热加载）

### AgentSkillsQuery

**用途**：Agent 可见 skill 列表查询接口。按 agent_id 查询该 agent 可用的 skill 列表，供 SkillsFragmentProvider 在构建 system prompt 静态层时使用。

**接口契约**：

| 要素 | 说明 |
|------|------|
| skill 列表查询 | 按 `agent_id` 查询该 agent 可见的 skill 名称和路径列表 |
| skill 可见性检查 | 按 `agent_id` + `skill_name` 检查指定 skill 是否对该 agent 可见 |
| 变更通知 | 注册回调接收 skill 列表变更通知（热加载场景） |

**关键规则**：
- 若 agent 未配置 skill 可见性规则，返回全局 skill 列表（所有 skill 可见）
- 可见性判断由 Agent 配置或默认策略决定，Skills 模块不感知 agent 区分逻辑
- 变更通知用于 system prompt 缓存失效，由 SkillsFragmentProvider 消费

### AgentToolsConfigQuery

**用途**：Agent 工具配置和权限查询接口。按 agent_id 查询该 agent 的工具启用状态、权限限制等配置信息。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 启用工具列表 | 按 `agent_id` 返回该 agent 已启用的工具名列表。未配置的工具按默认策略处理 |
| 工具权限 | 按 `agent_id` + `tool_name` 查询指定工具的权限级别（允许 / 拒绝 / 需确认） |
| 全局配置 | 返回当前生效的全局工具配置（未按 agent 细化时的 fallback） |

**关键规则**：
- 工具权限优先级：agent 级配置 > 全局配置 > 默认允许
- 需确认的工具有权限校验时由 Permission 模块介入
- 配置内容由 Config 模块在系统启动时加载并缓存。Agent 模块可按需覆盖特定 agent 的工具配置

### IdentityResolver

**用途**：身份解析接口。将 IM 平台的 sender_id 映射为 CloseClaw 本地的 account_id。由 Config 模块实现，IM Adapter 在入站消息解析时消费。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 身份解析 | 按 `platform` + `sender_id` 查询对应的 `account_id`，返回 `Option<string>` |
| 绑定检查 | 按 `account_id` 返回其绑定的所有 `(platform, sender_id)` 对 |
| 平台默认 | 按 `platform` 返回该平台的默认账户（如 terminal 平台恒为 `"owner"`） |

**关键规则**：
- 一个 account_id 可绑定多个平台的 sender_id（跨平台身份聚合）
- 一个 sender_id 在同一平台内仅映射到一个 account_id
- 映射数据来源：配置文件（accounts.json）中的绑定规则
- 未找到映射时返回 None，由上游 Gateway 决定处理策略（拒绝消息或使用默认账户）
- terminal 平台的映射恒为 `"owner"`，无需查表

### OutboundMiddleware

**用途**：出站消息中间件拦截链接口。Gateway 在 IMPlugin 渲染完成后、发送前遍历已注册的中间件链，各中间件对 [RenderedOutput](shared-types.md#renderedoutput) 进行检查、记录或阻断。用于审计日志、频率限制、内容安全过滤等场景。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | 中间件的唯一名称，用于日志和错误报告 |
| 优先级 | 数值越小越早执行，决定中间件在链中的执行顺序 |
| 拦截 | 接收 [RenderedOutput](shared-types.md#renderedoutput) + 消息上下文（platform、目标、agent 等），返回 `Result<Action>`。`Action` 枚举：`Pass`（放行）、`Block`（阻断发送并记录原因）、`Modify(RenderedOutput)`（修改后放行） |
| 异步支持 | 拦截方法为异步调用，支持 I/O 操作（持久化审计日志等） |

**关键规则**：
- 中间件链按优先级排序，依次执行
- 任一中间件返回 `Block` 时立即中断后续中间件执行，消息不发送
- `Modify` 返回的修改后 RenderedOutput 传递给下一个中间件
- 中间件不应修改消息上下文（platform、目标等），仅处理 RenderedOutput 本身

### SessionLookup

**用途**：Session 查找接口。按 session_id 或 session_key 查询会话。由 Daemon 模块实现，Gateway 和 Permission 模块通过 DI 消费。

**接口契约**：

| 要素 | 说明 |
|------|------|
| ID 查询 | 按 `session_id`（UUID）查询 session，返回 `Option<ConversationSession>` |
| Key 查询 | 按 `session_key`（平台无关的消息级标识）查询 session，返回 `Option<ConversationSession>` |
| 活跃列表 | 返回当前所有活跃 session 的元信息列表（session_id、agent_id、状态、创建时间） |
| 存在检查 | 按 `session_id` 或 `session_key` 检查 session 是否存在，返回 bool |

**关键规则**：
- session_id 和 session_key 的映射关系由 Daemon 管理
- 查询不包括已归档或已删除的 session
- 活跃列表用于 Gateway 做消息路由和权限校验的场景

### SlashHandler

**用途**：斜杠指令 Handler 接口。每个斜杠指令实现此 trait，提供指令名称、描述和执行逻辑。SlashRouter 收集所有已注册的 Handler，按指令名路由。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 指令名 | Handler 处理的具体指令名（不含 `/` 前缀，如 `"help"`、`"status"`） |
| 描述 | 指令的简短帮助说明，用于 `/help` 列表展示 |
| 参数定义 | 指令的可选参数列表和格式说明，用于参数校验和帮助展示 |
| 执行 | 接收参数字符串和 SlashContext，返回 [SlashResult](shared-types.md#slashresult) |

**关键规则**：
- 参数解析由各 Handler 自行负责，Router 仅传递原始参数字符串
- Handler 不直接产生 I/O 副作用——所有副作用通过返回的 SlashResult 变体表达，由 Gateway 统一执行
- 指令名全局唯一，同名注册冲突在系统启动时报告并拒绝注册
- 标准 Handler 列表见 [slash 模块](../slash/README.md)

### TaskManager

**用途**：后台任务管理接口。支持后台任务的创建、取消、状态查询和结果收集。由 Tasks 模块实现，Daemon、System Prompt Builder（后台缓存预热）、Tools 模块（长耗时工具）等消费。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 任务创建 | 接收 `TaskSpec`（任务类型、参数、超时时间、优先级），返回 `task_id`（UUID） |
| 任务取消 | 按 `task_id` 取消任务。任务未开始则移除，运行中则发送取消信号 |
| 状态查询 | 按 `task_id` 查询任务状态（Pending / Running / Completed / Failed / Cancelled）和进度信息 |
| 结果获取 | 按 `task_id` 阻塞等待任务完成，返回 `TaskResult<T>` |
| 列表查询 | 返回当前所有任务的摘要列表（含 Agent 标识的筛选） |

**关键规则**：
- 任务创建不阻塞调用方，立即返回 task_id。调用方通过结果获取接口获取执行结果
- 取消操作为"尽力而为"——任务可能收到取消信号后继续执行至完成
- 任务超时由 Tasks 模块内部管理，超时后标记为 Failed
- 任务的并发度由 Tasks 模块的配置控制，不影响提交方的行为

### ToolRegistryQuery

**用途**：工具注册表只读查询接口。不暴露写操作（注册工具、冻结注册表等），供 Daemon 和 Tools 模块以外的代码查询已注册的工具信息。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 工具查询 | 按工具名返回 [Tool trait](#tool-trait) 的完整引用（名称、分组、摘要、行为描述、输入模式、运行时标记），返回 `Option<&dyn Tool>` |
| 分组列表 | 返回所有已注册的分组名列表 |
| 分组查询 | 按分组名返回该分组下的所有工具名 |
| 索引字符串 | 返回构建完成的一级索引字符串（用于 system prompt 注入） |
| 存在检查 | 按工具名检查工具是否已注册 |

**关键规则**：
- 查询操作绕过 ToolRegistry 的冻结检查——冻结前和冻结后均可查询
- 不提供按类型或标签过滤的接口，仅按工具名和分组查询
- 不暴露注册表大小或元信息（如注册时间）

### IMAdapter

**用途**：IM 适配器接口。封装单个消息平台的入站消息解析、出站格式渲染和消息发送。与 IMPlugin 同领域但职责更轻量——IMAdapter 是单平台适配器，不管理插件生命周期（init/shutdown），专注数据转换和发送；IMPlugin 是完整的平台插件契约，包含生命周期管理和 Gateway 的 Plugin Registry 集成。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 平台标识 | Adapter 所属平台名（如 `"feishu"`、`"telegram"`），用于路由 |
| 入站解析 | 接收平台原生 webhook/事件 payload，解析为 [NormalizedMessage](shared-types.md#normalizedmessage)。签名校验、事件去重等由 Adapter 内部处理 |
| 渲染 | 接收 [ContentBlock](shared-types.md#contentblock)[] 和 [DslParseResult](shared-types.md#dslparseresult-和-dslinstruction)，按平台能力选择输出格式，产出 [RenderedOutput](shared-types.md#renderedoutput)。渲染是纯数据转换，无副作用 |
| 发送 | 接收 [RenderedOutput](shared-types.md#renderedoutput)，以指定目标调用平台发送 API |

**关键规则**：
- IMAdapter 不持有生命周期方法——连接池、token 刷新等由调用方或上层 IMPlugin 管理
- 渲染逻辑与 IMPlugin 共享——同一个 Renderer 实现可供 IMAdapter 和 IMPlugin 复用
- IMAdapter 被 IMPlugin 内部使用或供 Gateway 直接使用（无需完整插件生命周期的场景）

### SkillRegistryQuery

**用途**：Skill 注册表只读查询接口。不暴露写操作（注册 skill、安装/卸载等），供 Daemon 查询已加载的 skill 信息。

**接口契约**：

| 要素 | 说明 |
|------|------|
| skill 查询 | 按 `skill_name` 查询已注册 skill 的完整元信息（名称、版本、路径、描述），返回 `Option<SkillMeta>` |
| 全部列表 | 返回所有已注册 skill 的摘要列表（名称、版本、状态） |
| 状态查询 | 按 `skill_name` 查询 skill 的运行状态（Loaded / Error / Disabled） |
| 存在检查 | 按 `skill_name` 检查 skill 是否已注册且可用 |

**关键规则**：
- 查询的是"已加载到当前进程"的 skill 状态，非文件系统的 skill 清单
- 不提供按目录或标签的过滤方法
- 安装和卸载操作不在本接口范围内

### StorageProvider

**用途**：Session 持久化存储接口。提供了一个统一的 checkpoint 存储抽象，覆盖 session 的持久化保存、恢复、删除和列表管理。基础设施层接口，被 11 篇设计文档引用。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 保存 checkpoint | 按 `session_id` + `session_key` 保存 [SessionCheckpoint](shared-types.md#sessioncheckpoint)，返回 [PersistResult](shared-types.md#persistresult) |
| 加载 checkpoint | 按 `session_id` 加载最新的 checkpoint，返回 `Option<SessionCheckpoint>` |
| 删除 checkpoint | 按 `session_id` 删除该 session 的所有 checkpoint，返回 [PersistResult](shared-types.md#persistresult) |
| 列出 session | 返回所有已持久化 session 的 [SessionCheckpoint](shared-types.md#sessioncheckpoint) 列表（含 session_id、更新时间、消息数概览） |

**关键规则**：
- Checkpoint 的存储格式和位置由具体实现决定（文件系统 / 嵌入式数据库 / 远程存储）
- 保存操作为全量写入——每次保存产出完整的 checkpoint，不支持增量写入
- 加载操作返回该 session 的最新 checkpoint，不提供按版本加载
- 列出 session 不返回完整的 checkpoint 数据，仅返回状态摘要
- PersistResult 含成功/失败状态和可选的错误描述

### SystemPromptBuilder

**用途**：System prompt 构建器接口。收集所有已注册的 [PromptFragmentProvider](#promptfragmentprovider)，构建 [FragmentContext](shared-types.md#fragmentcontext)，触发片段生成，按优先级排序并拼接为完整的 system prompt。

**接口契约**：

| 要素 | 说明 |
|------|------|
| Provider 注册 | 注册一个 [PromptFragmentProvider](#promptfragmentprovider) 实现者，包含其名称和优先级 |
| 上下文构建 | 根据 `agent_id`、`bootstrap_mode`、`workdir` 构建 [FragmentContext](shared-types.md#fragmentcontext) |
| 触发生成 | 收集所有已注册 Provider，按优先级排序后依次调用片段生成，返回拼接后的完整 prompt 字符串 |
| 缓存管理 | 管理 Section 级缓存：按 Provider 的缓存键判定是否命中，未命中时调用对应 Provider 生成并缓存 |

**关键规则**：
- 一次构建调用 = 一个完整的 system prompt 字符串，包含所有非空 Provider 的片段
- Provider 返回空时 Builder 跳过其片段（不插入空 section 标题）
- 缓存由 Builder 内部管理，Provider 不感知缓存状态
- 触发行为详见 [数据流](#system-prompt-构建) 节

### SlashRouter

**用途**：斜杠指令路由器接口。管理所有 [SlashHandler](#slashhandler) 的注册，将入站指令字符串解析为指令名和参数，路由到对应的 Handler 执行。

**接口契约**：

| 要素 | 说明 |
|------|------|
| Handler 注册 | 注册一个 [SlashHandler](#slashhandler) 实现者。指令名冲突时拒绝注册 |
| 指令解析 | 将 `/cmd arg1 arg2` 格式的字符串解析为 `(command_name, args_string)` |
| 路由执行 | 根据指令名查找对应的 Handler，传递参数字符串和 SlashContext，返回 [SlashResult](shared-types.md#slashresult) |
| 帮助生成 | 聚合所有已注册 Handler 的描述和参数定义，生成 `/help` 的输出文本 |

**关键规则**：
- 路由查找失败时（未知指令名），返回 [SlashResult](shared-types.md#slashresult) 的 Unknown 变体
- 指令名解析大小写不敏感——"/Help" 和 "/help" 路由到同一 Handler
- Handler 注册在系统启动阶段完成，运行时注册需通过 DI 容器

## 数据流

core-traits 本身不参与运行时数据流。trait 接口在依赖注入时绑定实现，各业务模块通过 trait 接口交互而非直接依赖实现模块。

### PromptFragmentProvider 注册与调用

1. 系统启动 → System Prompt Builder 收集所有 Provider 实现者 → 按优先级排序
2. 构建触发（session 创建/恢复/compaction）
3. Builder 构建 [FragmentContext](shared-types/prompt-fragment.md)（agent 标识 + bootstrap 模式 + 工作目录）
4. 按优先级遍历 Provider → 检查缓存（命中则复用，未命中则调用片段生成）→ 跳过返回空的 → 按序拼接产出 [PromptFragment](shared-types/prompt-fragment.md)
5. 写入 ConversationSession 的 system prompt 字段

缓存由 Builder 内部管理，详细缓存策略和失效规则见 [fragment-provider](../system_prompt/fragment-provider.md)。

### ToolRegistrar 注册与编排

1. 系统启动 → Tools 模块收集所有 ToolRegistrar 实现者 → 按优先级排序
2. 依次调用各 Registrar → 向 [ToolRegistry](#toolregistry) 注册工具 → 注册完成 → ToolRegistry 冻结
3. 后续流程（索引构建、工具发现、system prompt 注入）不变

### Agent 配置类 trait（AgentConfigLookup / AgentLookup / AgentSkillsQuery / AgentToolsConfigQuery）

1. 系统启动 → Agent 模块加载 agent 配置 → 实现 AgentConfigLookup 和 AgentLookup → 注册到 DI 容器
2. Config 模块加载工具权限配置 → 实现 AgentToolsConfigQuery → 注册到 DI 容器
3. Agent 模块或 Skills 模块加载 skill 可见性规则 → 实现 AgentSkillsQuery → 注册到 DI 容器
4. 运行时：各模块通过 DI 容器获取对应 trait 引用，按需查询
5. 配置变更需重启或触发热加载回调

### 消息出站中间件（OutboundMiddleware）

1. 系统启动 → 各中间件实现者注册到 Gateway 的出站中间件链 → 按优先级排序
2. 运行时：Gateway 在 IMPlugin 渲染完成后、发送前遍历中间件链
3. 每个中间件对 RenderedOutput 检查 → 返回 Pass / Block / Modify
4. Pass：继续传递至下一个中间件；Block：停止发送，记录阻断原因；Modify：修改后继续传递
5. 所有中间件均返回 Pass 后，消息通过 IMPlugin 发送

### Session 查找（SessionLookup）

1. 系统启动 → Daemon 模块实现 SessionLookup → 维护 session_id ↔ session_key 映射和活跃 session 列表 → 注册到 DI 容器
2. Gateway 收到入站消息 → Processor Chain 计算出 session_key → Gateway 调用 SessionLookup 查询目标 session
3. Permission 模块进行权限校验 → 调用 SessionLookup 获取 session 上下文信息
4. session 不存在时 Gateway 创建新 session，Daemon 将其加入活跃列表

### 查询类 trait（ToolRegistryQuery / SkillRegistryQuery / IdentityResolver）

这些 trait 为纯查询接口，不参与独立数据流，由消费方按需调用。无运行时状态变更或多步骤触发流程。

- ToolRegistryQuery：Daemon 启动后按需查询已注册工具（索引生成、schema 获取等）。Tools 模块实现
- SkillRegistryQuery：Daemon 和 SkillsFragmentProvider 按需查询 skill 状态和列表。Skills 模块实现
- IdentityResolver：IM Adapter 在每条入站消息解析时调用，将 sender_id 映射为 account_id。Config 模块实现
4. session 不存在时 Gateway 创建新 session，Daemon 将其加入活跃列表

### 斜杠指令路由（SlashHandler / SlashRouter）

1. 系统启动 → Slash 模块收集所有 SlashHandler 实现者 → 注册到 SlashRouter → 指令名冲突时拒绝注册
2. Gateway 收到入站消息 → Processor Chain 检测到指令前缀 `/` → 调用 SlashRouter 解析指令名和参数
3. SlashRouter 按指令名路由到对应 Handler → Handler 执行并返回 SlashResult
4. 额外场景：
   - 帮助生成（/help）：遍历所有 Handler 的指令描述 → 生成帮助文本，由 SlashRouter 或专门的 HelpHandler 管理
   - 未知指令（路由查找失败）→ 返回 [SlashResult](shared-types.md#slashresult) 的 Unknown 变体

### 后台任务（TaskManager）

1. 系统启动 → Tasks 模块实现 TaskManager → 初始化任务队列和线程池 → 注册到 DI 容器
2. 各模块通过 DI 获取 TaskManager 引用 → 提交任务（指定类型、参数、超时）
3. Tasks 模块内部的任务调度器按优先级和并发度驱动任务执行
4. 任务完成或失败后，结果通过 TaskManager 的结果获取接口提供给提交方
5. 提交方可通过取消接口主动取消任务，Tasks 模块向任务发送取消信号

### System prompt 构建（SystemPromptBuilder）

1. 注册和调用流程与 PromptFragmentProvider 相同——SystemPromptBuilder 由 System Prompt 模块实现，统一管理 Provider 注册、缓存和触发生成
2. 构建时：SystemPromptBuilder 按需构建 FragmentContext → 触发各 Provider 生成片段 → 拼接返回完整 system prompt
3. 缓存策略由 Builder 内部管理，详细见 [fragment-provider](../system_prompt/fragment-provider.md)

### IMAdapter 出站发送

1. IMAdapter 为 IMPlugin 的轻量子集——仅封装发送能力，入站解析和渲染由完整的 IMPlugin（或调用方）管理
2. Gateway 完成渲染和中间件处理后 → 调用 IMPlugin.send() 或 IMAdapter.send() 发送消息到平台

### 存储（StorageProvider）

1. 系统启动 → 基础设施模块实现 StorageProvider → 初始化存储后端（文件目录 / 数据库连接）→ 注册到 DI 容器
2. Session 生命周期中的持久化：
   - Session 创建时：无 checkpoint，首次保存时创建
   - Session 运行中：状态变更（消息追加、mode 切换等）触发 checkpoint 保存
   - Compaction 后：压缩后的 session 状态保存为新 checkpoint
   - Session 关闭：保存最终 checkpoint
3. Session 恢复：Gateway 收到匹配已有 session_key 的消息 → 调用 StorageProvider 加载 checkpoint → 重建 Session
4. 会话列表查询：Daemon 调用 StorageProvider 列出所有持久化 session → 用于 session 管理（恢复、清理等）

## 模块关系

- **上游**：无（common 不依赖任何其他模块，是纯定义基底层）
- **下游**：
  - **system_prompt**（实现 BootstrapFragmentProvider，System Prompt Builder 收集所有 Provider 并触发生成）
  - **tools**（实现 ToolsFragmentProvider 和 CoreToolsRegistrar，提供 ToolRegistry 具体实现，收集 ToolRegistrar 实现者并编排调用）
  - **session**（实现 SessionToolsRegistrar）
  - **skills**（实现 SkillsFragmentProvider 和 SkillsToolsRegistrar）
  - **memory**（实现 MemoryFragmentProvider）
  - **im_adapter**（实现 ImAdapterToolsRegistrar；各平台插件实现 IMPlugin trait，Gateway 通过 Plugin Registry 消费）
  - **gateway**（消费 IMPlugin trait，维护平台到插件的 Plugin Registry 映射；消费 SessionLookup、OutboundMiddleware、SystemPromptBuilder）
  - **cli**（TerminalAdapter 实现 IMPlugin trait，提供 terminal 渠道的插件实现）
  - **agent**（实现 AgentConfigLookup、AgentLookup）
  - **config**（实现 IdentityResolver）
  - **permission**（消费 SessionLookup）
  - **slash**（实现 SlashHandler、SlashRouter）
  - **tasks**（实现 TaskManager）
  - **daemon**（消费 StorageProvider、SessionLookup、ToolRegistryQuery、SkillRegistryQuery）
- **无关**：Processor Chain（不参与 trait 接口定义或 DI 绑定）
