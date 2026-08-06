# 核心 trait

## 概述

核心 trait 是跨模块依赖注入的接口契约。本文档唯一定义 common crate 中每个核心 DI trait 的完整接口。各业务模块文档通过引用指向此处，不在自身文档中重复定义本文档已收录的 trait。各业务模块文档通过引用指向此处，不在自身文档中重复定义本文档已收录的 trait。

未被收录的 trait 不属于 common 的跨模块 DI 接口，属于对应领域模块的接口，应放在领域模块文档中。代码映射规则见 [common README](README.md) 边界规则——common crate 中 pub trait 必须已在本文档中唯一定义；本文档定义的所有 trait，代码中均位于 common crate。

## 架构

### PromptFragmentProvider

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

### ToolRegistrar

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

### IMPlugin

**用途**：统一抽象各消息平台的插件契约。Gateway 通过收集已注册的 IMPlugin 管理跨平台的消息入站解析、出站格式渲染和消息发送。每个消息平台（飞书、Discord、Telegram、Terminal）封装为一个独立插件，实现此 trait 的四个方法分组。

**接口契约**：

| 要素 | 说明 |
|------|------|
| 标识 | Plugin 的唯一平台名（如 `"feishu"`、`"terminal"`），用于 Gateway 的 Plugin Registry 路由 |
| 入站 | 解析平台原生 webhook/事件 payload 为 [NormalizedMessage](shared-types.md#normalizedmessage)。text 类型空 content 消息在解析阶段丢弃，非文本消息（image/file/audio）正常产出 NormalizedMessage（message_type 标记类型，media_refs 存储引用，content 可为空） |
| 渲染 | 接收 [ContentBlock](shared-types.md#contentblock)[] 和 [DslParseResult](shared-types.md#dslparseresult-和-dslinstruction)，按平台能力选择输出格式（纯文本或富格式），产出 [RenderedOutput](shared-types.md#renderedoutput)。渲染是纯数据转换，无副作用 |
| 发送 | 接收 [RenderedOutput](shared-types.md#renderedoutput)，以指定目标（peer_id + thread_id）调用平台发送 API |
| 生命周期 | `init()`：启动时初始化（连接池、token 等），不需要的插件空实现；`shutdown()`：关闭时清理资源，不需要的插件空实现 |

**渲染与发送的分离**：渲染产出数据（RenderedOutput），发送执行副作用。Gateway 在两步之间可插入审计、频率限制等中间件。

**平台插件实现**和注册机制详见 [IM Adapter 模块](../im_adapter/README.md)。

**入站身份映射**：IMPlugin 在入站解析时负责填充 [NormalizedMessage](shared-types.md#normalizedmessage) 的全部字段，包括通过 sender_id 查询账户绑定表获取 account_id。映射规则和账户配置详见 [config 模块](../config/README.md)。

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

## 模块关系

- **上游**：无（common 不依赖任何其他模块，是纯定义基底层）
- **下游**：
  - **system_prompt**（实现 BootstrapFragmentProvider，System Prompt Builder 收集所有 Provider 并触发生成）
  - **tools**（实现 ToolsFragmentProvider 和 CoreToolsRegistrar，提供 ToolRegistry 具体实现，收集 ToolRegistrar 实现者并编排调用）
  - **session**（实现 SessionToolsRegistrar）
  - **skills**（实现 SkillsToolsRegistrar 和 SkillsFragmentProvider）
  - **memory**（实现 MemoryFragmentProvider）
  - **im_adapter**（实现 ImAdapterToolsRegistrar；各平台插件实现 IMPlugin trait，Gateway 通过 Plugin Registry 消费）
  - **gateway**（消费 IMPlugin trait，维护平台到插件的 Plugin Registry 映射）
  - **cli**（TerminalPlugin 实现 IMPlugin trait，提供 terminal 渠道的插件实现，TerminalAdapter 为其入站解析子组件）
- **无关**：Processor Chain（不参与 trait 接口定义或 DI 绑定）
