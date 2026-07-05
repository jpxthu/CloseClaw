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
| 片段生成 | 根据 [FragmentContext](shared-types.md#fragmentcontext) 产出 [PromptFragment](shared-types.md#promptfragment)。无内容时返回空（文件缺失、agent 无可见 skill 等），Builder 自动跳过 |
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
| 入站 | 解析平台原生 webhook/事件 payload 为 [NormalizedMessage](shared-types.md#normalizedmessage)。空内容消息在解析阶段丢弃，非文本消息正常产出 NormalizedMessage（message_type 标记类型，media_refs 存储引用） |
| 渲染 | 接收 [ContentBlock](shared-types.md#contentblock)[] 和 [DslParseResult](shared-types.md#dslparseresult-和-dslinstruction)，按平台能力选择输出格式（纯文本或富格式），产出 [RenderedOutput](shared-types.md#renderedoutput)。渲染是纯数据转换，无副作用 |
| 发送 | 接收 [RenderedOutput](shared-types.md#renderedoutput)，以指定目标（peer_id + thread_id）调用平台发送 API |
| 生命周期 | `init()`：启动时初始化（连接池、token 等），不需要的插件空实现；`shutdown()`：关闭时清理资源，不需要的插件空实现 |

**渲染与发送的分离**：渲染产出数据（RenderedOutput），发送执行副作用。Gateway 在两步之间可插入审计、频率限制等中间件。

**平台插件实现**和注册机制详见 [IM Adapter 模块](../im_adapter/README.md)。

**入站身份映射**：IMPlugin 在入站解析时负责填充 [NormalizedMessage](shared-types.md#normalizedmessage) 的全部字段，包括通过 sender_id 查询账户绑定表获取 account_id。映射规则和账户配置详见 [config 模块](../config/README.md)。

### AgentSkillsQuery

**用途**：Agent skill 可见性查询接口。Skills Registry 通过此接口查询指定 agent 的 skills 白名单，确定哪些 skills 对该 agent 可用。由 Agent Registry 实现。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 查询 | 按 `agent_id` 查询该 agent 的 skills 白名单 |
| 白名单语义 | 白名单为 `["*"]` 或空时 → 不限制，所有 skills 对该 agent 可见 |

**关键规则**：
- Agent Registry 在系统启动时从 Config 加载 agent 配置并填充，运行时只读查询
- Skills Registry 构建 agent system prompt 的 skill 段时，通过此接口过滤可见 skills

### AgentToolsConfigQuery

**用途**：Agent 工具配置查询接口。Tools Registry 通过此接口查询指定 agent 的 tools 白名单和 disallowedTools 黑名单，确定哪些 tools 对该 agent 允许或禁止使用。由 Agent Registry 实现。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 白名单查询 | 按 `agent_id` 查询该 agent 的 tools 白名单。白名单为 `["*"]` 或空时 → 不限制 |
| 黑名单查询 | 按 `agent_id` 查询该 agent 的 disallowedTools 黑名单。黑名单为空时 → 不限制 |

**关键规则**：
- Agent Registry 在系统启动时从 Config 加载 agent 配置并填充，运行时只读查询
- Tools Registry 在注册工具时，通过此接口过滤各 agent 的工具可见性

## 数据流

core-traits 本身不参与运行时数据流。trait 接口在依赖注入时绑定实现，各业务模块通过 trait 接口交互而非直接依赖实现模块。

### PromptFragmentProvider 注册与调用

1. 系统启动 → System Prompt Builder 收集所有 Provider 实现者 → 按优先级排序
2. 构建触发（session 创建/恢复/compaction）
3. Builder 构建 [FragmentContext](shared-types.md#fragmentcontext)（agent 标识 + bootstrap 模式 + 工作目录）
4. 按优先级遍历 Provider → 检查缓存（命中则复用，未命中则调用片段生成）→ 跳过返回空的 → 按序拼接产出 [PromptFragment](shared-types.md#promptfragment)
5. 写入 ConversationSession 的 system prompt 字段

缓存由 Builder 内部管理，详细缓存策略和失效规则见 [fragment-provider](../system_prompt/fragment-provider.md)。

### ToolRegistrar 注册与编排

1. 系统启动 → Tools 模块收集所有 ToolRegistrar 实现者 → 按优先级排序
2. 依次调用各 Registrar → 向 [ToolRegistry](#toolregistry) 注册工具 → 注册完成 → ToolRegistry 冻结
3. 后续流程（索引构建、工具发现、system prompt 注入）不变

### AgentSkillsQuery 查询

1. 系统启动 → Agent Registry 从 Config 加载 agent 配置 → 填充各 agent 的 skills 白名单
2. Skills Registry 在构建 system prompt 的 skill 段时 → 调用 AgentSkillsQuery 按 agent_id 查询白名单
3. 白名单为通配或空 → 不限制，返回全部 skills

### AgentToolsConfigQuery 查询

1. 系统启动 → Agent Registry 从 Config 加载 agent 配置 → 填充各 agent 的 tools 白名单和黑名单
2. Tools Registry 注册和过滤工具时 → 调用 AgentToolsConfigQuery 按 agent_id 查询黑白名单
3. 白名单为通配或空 → 不限制；黑名单为空 → 不限制

## 模块关系

- **上游**：无（common 不依赖任何其他模块，是纯定义基底层）
- **下游**：
  - **system_prompt**（实现 BootstrapFragmentProvider，System Prompt Builder 收集所有 Provider 并触发生成）
  - **tools**（实现 ToolsFragmentProvider 和 CoreToolsRegistrar，提供 ToolRegistry 具体实现，收集 ToolRegistrar 实现者并编排调用；消费 AgentToolsConfigQuery 过滤工具可见性）
  - **session**（实现 SessionToolsRegistrar）
  - **skills**（实现 SkillsFragmentProvider 和 SkillsToolsRegistrar；消费 AgentSkillsQuery 过滤 skill 可见性）
  - **memory**（实现 MemoryFragmentProvider）
  - **im_adapter**（实现 ImAdapterToolsRegistrar；各平台插件实现 IMPlugin trait，Gateway 通过 Plugin Registry 消费）
  - **gateway**（消费 IMPlugin trait，维护平台到插件的 Plugin Registry 映射）
  - **cli**（TerminalAdapter 实现 IMPlugin trait，提供 terminal 渠道的插件实现）
  - **agent**（实现 AgentSkillsQuery 和 AgentToolsConfigQuery）
- **无关**：Processor Chain（不参与 trait 接口定义或 DI 绑定）
