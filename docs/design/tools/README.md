# Tools 模块

## 概述

- 关联需求文档：[requirements/tools.md](../../requirements/tools.md)
- Tools 模块是 CloseClaw 的工具注册基础设施——提供统一接口、并发安全的注册中心、索引构建和工具发现能力。各模块通过 [ToolRegistrar](../common/core-traits.md#toolregistrar) 注册自身工具、由 Tools 模块编排调用（Mode 执行触发工具与 Workflow 工具属注册编排的例外，见「架构 · 各模块注册的工具一览」）；Tools 模块不硬编码任何工具列表。

核心设计：

- 所有工具通过统一接口（[Tool trait](../common/core-traits.md)，定义见 common）实现
- ToolRegistry 是全局注册中心；各模块实现 ToolRegistrar，由 Tools 模块按优先级编排调用完成注册
- 一级索引注入 system prompt，按分组展示工具——常用工具（始终加载）展示名称、危险度标记与行为描述，延迟工具（延迟加载）仅展示名称与危险度标记
- 二级详情通过工具发现机制按需注入，不占用初始上下文

## 架构

Tools 模块由三层组成：接口层、注册中心层、工具提供者层。

- **接口层**：统一工具接口（[Tool trait](../common/core-traits.md)，完整定义见 common）
- **注册中心层**：ToolRegistry —— 并发安全的注册、查询、索引构建
- **工具提供者层**：各模块实现的工具，经 ToolRegistrar 注册进 ToolRegistry（清单见「自身工具」「各模块注册的工具一览」）

### 接口层

每个工具都实现统一的 Tool trait（完整定义见 [common/core-traits.md](../common/core-traits.md)），包含一组描述方法和一个执行方法：

- **标识**：工具名和所属分组，用于索引和发现
- **摘要**：一句话描述，用于工具发现结果的简要展示
- **行为描述**：完整的功能说明，常用工具的行为描述进入一级索引供 LLM 理解工具用途
- **动态 prompt 生成**：根据运行时上下文（权限、可用工具、工作目录等）动态调整工具描述，默认回退到静态的行为描述（机制详见 [dynamic-prompt-generation.md](dynamic-prompt-generation.md)）
- **参数模式**：JSON Schema 格式，直接暴露为 API schema，不转自然语言
- **运行时标记**：标识工具是否只读、是否破坏性、是否昂贵、是否默认延迟加载、是否并发安全
- **执行方法**：接收调用参数、产出工具结果

### 注册中心层

ToolRegistry 是线程安全的全局工具注册与查询入口，运行期以工具名为键持有所有已注册工具。提供以下能力：

- **注册**：Tools 模块按优先级编排各 ToolRegistrar，依次调用其注册方法写入其工具，工具名冲突时报错
- **冻结**：全部注册完成后标记注册结束，此后拒绝新注册
- **索引构建**：按分组聚合工具，生成一级索引字符串（展示规则见「索引结构」）
- **查询**：按工具名获取完整详情、按分组名获取该组全部工具名、按关键词或工具名匹配工具（供工具发现使用）

各模块通过 ToolRegistrar trait 向 ToolRegistry 注册工具。Tools 模块收集所有 ToolRegistrar 实现者、按优先级排序依次调用，提供 ToolRegistry 的具体实现，不硬编码各模块的具体接口。各模块工具清单的权威描述位于其所属模块的设计文档，ToolRegistry 不硬编码清单；工具的定义和参数归所属模块，Tools 模块负责编排注册顺序。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [read-tool.md](read-tool.md) | Read 工具：分段读取、截断策略、续读指引、文件去重 |
| [write-edit-tool.md](write-edit-tool.md) | Write/Edit 工具：精确文本替换、non-incremental 匹配、per-file 互斥、冲突检测 |
| [bash-tool.md](bash-tool.md) | Bash 工具：命令执行、超时控制、输出截断、后台触发 |
| [bash-security.md](bash-security.md) | Bash 安全解析：AST 分析、信任分级、攻击检测 |
| [background-tasks.md](background-tasks.md) | 后台任务：异步执行、自动后台化、卡死检测、完成通知、输出生命周期与停止/销毁清理 |
| [multi-tool-calls.md](multi-tool-calls.md) | 多工具并行调用：并发安全声明、per-file 互斥队列、配置开关 |
| [tools-prompt-injection.md](tools-prompt-injection.md) | 工具提示词注入：两级注入机制、加载策略、长度控制 |
| [dynamic-prompt-generation.md](dynamic-prompt-generation.md) | 提示词动态生成：Schema/Prompt 双轨制、上下文感知 |
| [tools-keywords.md](tools-keywords.md) | 工具关键词索引：嵌入格式、匹配机制、维护原则 |
| [tool-registrar.md](tool-registrar.md) | ToolRegistrar trait（定义见 [common](../common/core-traits.md#toolregistrar)）：各模块向 ToolRegistry 注册工具的统一接口契约 |

### 索引结构

一级索引按分组输出，加载策略分两类——**常用工具**（始终加载，展示名称、危险度标记和行为描述）与**延迟工具**（延迟加载，仅展示名称和危险度标记）。分组格式、危险度标记取值、加载策略标注与长度截断规则详见 [tools-prompt-injection.md](tools-prompt-injection.md)。

### 工具发现

二级详情通过工具发现机制按需注入：LLM 传入关键词或工具名，注册中心匹配并返回完整详情。发现流程详见 [tools-prompt-injection.md](tools-prompt-injection.md)，关键词匹配机制详见 [tools-keywords.md](tools-keywords.md)。

### 安全边界

工具的权限检查不在单个工具实现中处理——工具执行前由 Tools 模块统一调用权限引擎校验。工具本身通过运行时标记声明自身的安全属性（只读/破坏性/昂贵），供权限引擎和索引渲染使用——一级索引的危险度标记即由只读/破坏性标记派生（只读标 `(read-only)`、破坏性标 `(destructive)`，取值定义见 [tools-prompt-injection.md](tools-prompt-injection.md)）。例外：Workflow 工具为系统级工具，不受 Agent 权限配置限制、不经权限引擎校验。

### 自身工具

Tools 模块自身向 ToolRegistry 注册以下核心工具：

| 分组 | 工具 | 加载策略 | 所属模块 |
|------|------|---------|---------|
| bash | Bash | 始终加载 | tools |
| file_ops | Read、Write、Edit、Grep、Ls | 始终加载 | tools |
| git_ops | GitStatus、GitLog、GitCommit、GitPush、GitPull | 延迟加载 | tools |
| meta | ToolSearch、PermissionQuery | 始终加载 | tools |

Bash、Read、Write/Edit 的详细设计见 [bash-tool.md](bash-tool.md)、[read-tool.md](read-tool.md)、[write-edit-tool.md](write-edit-tool.md)。Git 操作组中状态和日志为只读，提交、推送、拉取为破坏性操作。meta 分组的 ToolSearch 承载工具发现，PermissionQuery 向 Agent 暴露当前权限状态、使其了解自己可执行的操作范围（权限维度与语义见 [permission 模块](../permission/README.md)）。

### 各模块注册的工具一览

以下工具由各模块注册到 ToolRegistry，定义和参数以所属模块的设计文档为准。

| 注册模块 | 分组 | 工具 | 加载策略 |
|---------|------|------|---------|
| [Mode](../mode/README.md) | mode | 执行触发工具 | 始终加载 |
| [Session](../session/README.md) | sessions | sessions_spawn、sessions_steer、sessions_kill、sessions_yield | 始终加载 |
| [Skills](../skills/README.md) | skills | SkillTool | 始终加载 |
| [Workflow](../workflow/README.md) | workflow | workflow_start、workflow_verify、workflow_jump、workflow_blocked | 始终加载 |
| [IM Adapter](../im_adapter/README.md) | feishu_im | feishu_im_user_message、feishu_im_user_get_messages、feishu_im_user_get_thread_messages、feishu_search_user | 延迟加载 |
| [IM Adapter](../im_adapter/README.md) | feishu_calendar | — | — |
| [IM Adapter](../im_adapter/README.md) | feishu_task | — | — |
| [IM Adapter](../im_adapter/README.md) | feishu_bitable | — | — |
| [IM Adapter](../im_adapter/README.md) | feishu_doc | — | — |
| [IM Adapter](../im_adapter/README.md) | feishu_drive | — | — |
| [IM Adapter](../im_adapter/README.md) | feishu_sheet | — | — |

补充说明：

- **注册编排**：Mode 的执行触发工具、Workflow 工具在 ToolRegistry 初始化阶段、冻结之前注册，不经 [tool-registrar.md](tool-registrar.md) 的四个标准 Registrar（tools、session、skills、im_adapter）编排；其中仅 Workflow 工具为系统级工具、跳过权限校验（见 [mode/execution.md](../mode/execution.md)、[workflow-tools.md](../workflow/workflow-tools.md)）。
- **SkillCreator** 是 Skills 模块的 Bundled 技能，经 SkillTool 分发执行，不作为独立工具注册（见 [skills/skill-execution.md](../skills/skill-execution.md)）。
- **飞书扩展能力域**：IM 消息操作（feishu_im）是 CloseClaw 固有能力，工具已定义；日历、任务、多维表格、文档、云盘、电子表格由需求声明为飞书扩展能力域（见 [飞书需求 §F6](../../requirements/im_adapter/feishu.md)），但接口与采集数据尚未齐备、设计无法落地，其工具暂不定义（表中以 — 标注）。

## 数据流

### 注册与注入

1. 系统启动，Tools 模块收集各模块的 ToolRegistrar 实现者
2. 按优先级数值升序排序，依次调用其注册方法，各模块向 ToolRegistry 注册自身工具
3. Mode 执行触发工具、Workflow 工具在冻结之前另行注册
4. 全部注册完成，ToolRegistry 冻结，进入运行态
5. system prompt 构建时经 ToolsFragmentProvider 触发索引构建，生成分组索引字符串
6. 分组索引注入 system prompt 的工具区，LLM 在对话初始化时看到一级索引（常用工具为名称、危险度标记与行为描述，延迟工具仅名称与危险度标记）

### 工具调用

1. LLM 选择工具并生成调用参数，agent 运行时解析工具调用
2. Tools 模块在执行前调用权限引擎校验（日志：权限检查结果）——通过则执行工具调用并返回结果（日志：工具调用——工具名、参数摘要、返回结果、耗时）；拒绝则返回权限错误
3. 例外：Workflow 工具为系统级工具，跳过权限校验直接执行（见「架构 · 安全边界」）

### 工具发现

1. LLM 需了解延迟工具详情，调用 ToolSearch（关键词或工具名）
2. 注册中心匹配并返回工具完整详情
3. 详情注入当前上下文，LLM 在后续对话中使用该工具

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| 各工具提供模块 | 实现 ToolRegistrar 注册自身工具，Tools 模块采集其注册结果 |
| system prompt 构建器 | 经 [ToolsFragmentProvider](../system_prompt/fragment-provider.md)（Tools 模块实现的 [PromptFragmentProvider](../common/core-traits.md#promptfragmentprovider)）获取分组索引，注入 system prompt 静态层 ToolsSection |
| Session 模块 | 管理工具调用的执行状态（阻塞、后台、进程句柄注册） |
| agent 模块 | 提供 AgentToolsConfigQuery，供 ToolRegistry 查询 agent 工具范围与配置 |
| Skill 系统 | 提供 skill 注册表，SkillTool 桥接调用 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| 权限引擎 | 工具执行前调用，校验工具调用的安全属性 |
| debug_log | 记录工具调用与权限检查结果的调试日志（见 [debug_log](../debug_log/README.md)） |

### 无关

| 模块 | 说明 |
|------|------|
| Processor Chain | 工具不参与消息出站处理 |
| Renderer | 工具不参与平台渲染 |

### 共享类型 / 核心 trait

- [common/core-traits](../common/core-traits.md)（实现：PromptFragmentProvider、ToolRegistrar、ToolRegistry、ToolRegistryQuery、Tool trait、KillHandle；消费：ToolSession、AgentToolsConfigQuery）
