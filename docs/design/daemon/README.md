# Daemon

## 概述

- 关联需求文档：[requirements/daemon.md](../../requirements/daemon.md)
- 一句话：Daemon 是进程入口和组件胶水层，负责系统启动时的组件初始化与依赖注入、系统级后台任务启动、配置触发的网关重启，以及优雅关闭。Daemon 自身不含业务逻辑。

## 架构

### 依赖驱动的启动顺序

启动采用依赖声明模型：每个组件声明自身依赖，启动时拓扑排序确定执行顺序。同层组件并行初始化；同一层内各组件按组件名称字母序依次发起初始化（发起后并行等待完成），以保证启动行为的确定性。存在循环依赖时拒绝启动并报错。同层并行使启动耗时收敛于各层关键路径；后台任务类组件以 spawn 方式非阻塞启动，不进入消息接收关键路径。外部入口（IM Adapters、CLI terminal 渠道）在其注册进 Gateway 接收路径前不接收消息，故外部入口就绪前系统不接收消息。

各组件依赖关系及所属层由声明自动推导，完整分层依赖表如下：

| 层 | 组件 | 依赖 |
|----|------|------|
| 1 | ConfigManager | 无 |
| 1 | SqliteStorage | 无 |
| 1 | ShutdownHandle | 无 |
| 2 | SessionConfigProvider | ConfigManager |
| 2 | MemoryConfigProvider | ConfigManager |
| 2 | AgentRegistry | ConfigManager |
| 2 | Config Hot Reload | ConfigManager（出站通知通道为运行时引用，IM Adapters 就绪后接线，不构成启动依赖） |
| 2 | LLM Registry | ConfigManager |
| 2 | SkillRegistry | ConfigManager |
| 2 | Renderers / Plugins | ConfigManager |
| 2 | Permission Engine | ConfigManager |
| 3 | IM Adapters | Renderers, ConfigManager |
| 3 | ToolRegistry | SkillRegistry |
| 3 | ArchiveSweeper | SqliteStorage, SessionConfigProvider |
| 3 | AnnounceSweeper | SqliteStorage, SessionConfigProvider |
| 3 | DreamingScheduler | SqliteStorage, MemoryConfigProvider |
| 3 | ApprovalFlow | Permission Engine, AgentRegistry |
| 4 | SessionManager | LlmCaller, SqliteStorage, AgentRegistry, SkillRegistry, ToolRegistry, SessionConfigProvider（接线持有 SpawnController 与 System Prompt 构建器） |
| 4 | SpawnController | AgentRegistry, ToolRegistry（ConfigManager、Permission Engine 与 Session 的 spawn 上下文为接线运行时引用） |
| 4 | System Prompt 构建器（含 DynamicPromptBuilder） | AgentRegistry, SkillRegistry, ToolRegistry |
| 5 | Gateway | SessionManager, IM Adapters, Permission Engine, ApprovalFlow, Renderers / Plugins |
| 6 | Admin RPC Server | Gateway |

上表中的后台任务类组件（Config Hot Reload、ArchiveSweeper、AnnounceSweeper、DreamingScheduler）构成系统级后台任务的权威清单；新增系统级后台任务时必须同步更新本表与 [shutdown.md](shutdown.md) 的停止设计。

初始化完成后进入消息循环，由 Gateway 接管所有消息处理。

Daemon 作为组合根，创建并持有上述依赖表中的全部组件（含 ConfigManager、SqliteStorage、ShutdownHandle）的引用，管理其生命周期；各组件的所有权与装配关系见「模块关系」。运行时，Daemon 亦负责择机执行配置触发的网关重启——重建承载重启类配置的组件（Gateway、LLM Registry 等）并重新接线、会话层不动，完整机制见 [restart.md](restart.md)。

### 子功能

| 文档 | 简述 |
|------|------|
| [restart.md](restart.md) | 配置触发的网关重启：待重启状态、变更合并、择机窗口、重建执行与兜底、完成通知 |
| [shutdown.md](shutdown.md) | 关闭全流程：ShutdownHandle 协调器、graceful/forceful 双模、阶段化执行、recovery 衔接、用户可见进度通知 |

## 数据流

### 启动路径

Daemon 启动时先经 [platform 模块](../platform/process.md) 的进程管理接口自注册 PID，再按层序执行初始化：层 1 → 层 2 → 层 3 → 层 4 → 层 5 → 层 6，上一层全部完成后进入下一层；同层内组件并行初始化。

1. **层 1**（无依赖）：
   - ConfigManager（多文件合并、凭据分离、环境变量加载、主配置文件迁移）
   - SqliteStorage（初始化持久化存储）
   - ShutdownHandle（关闭门控标志、活跃操作计数与 drain 状态查询，供各组件经 ShutdownSignal 消费，见 [shutdown.md](shutdown.md)）
2. **层 2**（依赖层 1）：
   - SessionConfigProvider（ConfigManager 加载后作为独立组件暴露，提供 per-agent 的 idle/purge 阈值与扫描间隔等会话配置，供 ArchiveSweeper、AnnounceSweeper、SessionManager 消费）
   - MemoryConfigProvider（ConfigManager 加载后作为独立组件暴露，提供记忆子系统配置（含 `dreaming.schedule`，字段语义与默认值见 [memory/config](../memory/config.md)），供 DreamingScheduler 消费）
   - AgentRegistry（创建空注册表 → ConfigManager 加载 agent 配置 → populate 填充）
   - Config Hot Reload（spawn 后台任务，监听配置文件变更，触发增量重载；重载校验失败时保留旧配置运行并经 IM 通知 Owner——出站通道为运行时引用，IM Adapters 就绪后接线，不构成启动依赖，详见 [config/hot-reload.md](../config/hot-reload.md)）
   - SkillRegistry（创建注册表骨架，加载 bundled skills）
   - LLM Registry（Daemon 侧的模型装配组件：从 ConfigManager 取得 models.json 供应商定义与凭据，构造统一 LLM Client（UnifiedChatClient）实例，见 [llm/README.md](../llm/README.md)；无可用模型/供应商时构造为空、不阻塞启动）
   - Renderers / Plugins（各平台 Renderer 封装为 Plugin 并注册）
   - Permission Engine（加载全局默认策略，Agent 维度规则延迟加载）
3. **层 3**（依赖层 2）：
   - IM Adapters（各平台 Adapter 创建，注入对应 Renderer）
   - ToolRegistry（各模块注册工具定义）
   - ArchiveSweeper（spawn 后台任务，定时扫描 idle session 归档 + 过期 archive 清理；归档前查询 SessionManager 四维活跃维度——该运行时引用在 SessionManager 就绪后接线，不构成启动依赖，详见 [session/session-lifecycle.md](../session/session-lifecycle.md)）
   - AnnounceSweeper（spawn 后台任务，定时扫描 spawn_tree 补推完成通知与僵死检测——扫描经 SessionManager 进行，该运行时引用在 SessionManager 就绪后接线，不构成启动依赖，详见 [session/run-health.md](../session/run-health.md)）
   - DreamingScheduler（spawn 后台任务，定时扫描 archived 会话，触发记忆挖掘与升格）
   - ApprovalFlow（注入 Permission Engine、AgentRegistry）
4. **层 4**（依赖层 3）：
   - SessionManager（注入 Daemon 构造的 LLM 调用器（LlmCaller）、SqliteStorage、AgentRegistry、ToolRegistry、SkillRegistry、SessionConfigProvider，初始化完成后执行启动恢复扫描，详见 [session/session-recovery.md](../session/session-recovery.md)）。LlmCaller 是 [common/core-traits](../common/core-traits.md#llmcaller) 定义的 LLM 调用接口，其具体实现（FallbackLlmCaller，gateway 域提供，桥接 LLM 模块 UnifiedChatClient）由 Daemon 在层 2 LLM Registry 就绪后、本层初始化前实例化并经 SessionManager 注入会话，各 ConversationSession 经其发起请求（真实 Provider 调用仍由 LLM 模块完成）
   - SpawnController（创建并管理子 session，启动依赖 AgentRegistry、ToolRegistry；spawn 前置校验与权限判定经 Permission Engine（子 Agent 权限继承、Deny 沿链路传播）、Agent 配置（深度/并发/超时阈值），详见 [agent/agent-spawn.md](../agent/agent-spawn.md)；ConfigManager、Permission Engine 及子 session 所需能力（经 SessionManager 提供的 spawn 上下文获取）为构造注入的运行时引用，不构成启动依赖。SpawnController 与 SessionManager 同层，Daemon 在层 4 初始化完成后将其引用接线给 SessionManager，供后者处理 spawn 请求时调用）
   - System Prompt 构建器（含 DynamicPromptBuilder；Daemon 在层 4 初始化完成后接线给 SessionManager，后者持有并向各 session 注入；构建由 SessionManager 触发，持有 AgentRegistry、SkillRegistry、ToolRegistry 引用，详见 [system_prompt/README.md](../system_prompt/README.md)）
5. **层 5**（依赖层 4）：Gateway（注入 adapters、SessionManager、permission、renderers；安装 SlashDispatcher（详见 [slash/README.md](../slash/README.md)）；注入 ApprovalFlow）
6. **层 6**（依赖层 5）：Admin RPC Server（启动 Unix domain socket 管理服务，承载 CLI 的管理接入：一是接收 CLI Admin 命令（agent/skill 查询，命令面见 [cli/admin.md](../cli/admin.md)）；二是接纳 CLI Chat 经该连接接入的 terminal 渠道——terminal 插件注册、消息链路与启动存活检测（不可达则 CLI Chat 报错退出、不自动拉起 daemon，见 [cli/chat.md](../cli/chat.md)））
7. 全部完成后**进入消息循环**

**LLM 能力缺失时的行为**：若启动时 LLM 能力不可用（如 models.json 缺失、未配置任何可用模型/供应商，Daemon 无法组装 LlmCaller），系统仍正常启动，不静默丢弃用户消息——未获 LlmCaller 的会话收到需 LLM 处理的消息时，由 [Session](../session/README.md) 经 [Gateway](../gateway/README.md) 的系统通知接口以明确错误回复告知用户 LLM 未就绪（Daemon 自身不处理消息、不代理该回复），而非假装处理或丢失消息。LlmCaller 在层 2 LLM Registry 就绪后由 Daemon 构造、层 4 注入 SessionManager 并接管至各会话；补齐 LLM 配置属重启类变更（见 [config 需求 §F7](../../requirements/config.md)），由 Daemon 择机重启后生效（重启重建 LLM Registry 与 LlmCaller 接线，会话对话状态保留）。

### 关闭路径

Daemon 关闭由 ShutdownHandle 统一协调，分阶段执行。详见 [shutdown.md](shutdown.md)。注意：配置触发的网关重启（见 [restart.md](restart.md)）不经过本关闭流程——只重建承载重启类配置的组件、会话层不动；但系统处于待重启状态或重建执行期间收到关闭信号时，仍直接进入 forceful 关闭、中止待执行/进行中的重建（见 [shutdown.md](shutdown.md)）。

高层概览（对应 [shutdown.md](shutdown.md) 的 Phase 0–7，此处按流程合并概述）：

1. 信号到达（经 platform 模块订阅的关闭信号），ShutdownHandle 判定模式（graceful / forceful）
2. 关闭入站接收 + Drain 已有消息
3. Session 停止（委托 SessionManager，graceful 模式等工具完成、LLM 流结束再停；forceful 模式立即 kill）
4. 停止系统级后台任务
5. 最终持久化 + 关闭出站 + 关闭存储
6. 退出

关闭开始时已接收、尚未处理完的消息不静默丢弃——承接机制详见 [shutdown.md](shutdown.md) Phase 1。

graceful 模式由用户掌控节奏：接收进度通知，可随时升级为 forceful。forceful 不做等待，依赖 recovery 在下次启动时恢复未完成操作。

配置触发的网关重启的完整数据流（待重启状态、变更合并、择机窗口、重建执行与兜底、完成通知）见 [restart.md](restart.md)。

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

- **上游**：[platform 模块](../platform/README.md)（操作系统进程管理器——Daemon 经其进程管理接口自注册 PID、订阅关闭信号（SIGTERM / SIGINT），并经其配置目录接口确定配置根目录；进程、信号、配置目录等 OS 差异统一由其封装消化）。
- **上游**：[config 模块](../config/README.md)（配置变更确认后触发配置触发的网关重启，见 [restart.md](restart.md)）；Owner（经 CLI 停止命令触发关闭、经 Admin RPC 接入 CLI Admin / CLI Chat，见 [cli](../cli/README.md)）。
- **下游**：Daemon 初始化/管理以下模块。

| 模块 | 关系 |
|------|------|
| ConfigManager | Daemon 持有；提供各配置文件合并后的数据结构 |
| SqliteStorage | Daemon 持有；持久化存储后端 |
| ShutdownHandle | Daemon 持有；关闭门控、drain 等待与状态查询（见 [shutdown.md](shutdown.md)） |
| SessionConfigProvider | Daemon 持有的配置查询组件；提供 per-agent 会话配置（字段定义见 [session/session-lifecycle.md](../session/session-lifecycle.md)） |
| MemoryConfigProvider | Daemon 持有的配置查询组件；提供记忆子系统配置（字段语义见 [memory/config](../memory/config.md)） |
| Permission Engine | Daemon 持有；提供权限判定（Agent 维度规则延迟加载） |
| AgentRegistry | Daemon 持有；提供 Agent 配置档案（由 ConfigManager 加载结果填充） |
| ToolRegistry | Daemon 持有；提供工具注册表 |
| SkillRegistry | Daemon 持有；提供技能注册表（含 bundled skills） |
| LLM Registry | Daemon 持有；提供统一 LLM Client，供组装 LlmCaller（内部架构见 [llm/README.md](../llm/README.md)） |
| SessionManager | Daemon 持有；注入 LlmCaller、SqliteStorage、各注册表与 SessionConfigProvider |
| System Prompt 构建器（含 DynamicPromptBuilder） | SessionManager 持有并向各 session 注入；由 SessionManager 触发构建；持有 AgentRegistry、SkillRegistry、ToolRegistry 引用（见 [system_prompt/README.md](../system_prompt/README.md)） |
| Renderers / Plugins | Daemon 装配；各平台 Renderer 封装为 Plugin |
| IM Adapters | Daemon 创建；各平台适配器 |
| Gateway | Daemon 持有；注入 adapters、SessionManager、permission、renderers |
| Admin RPC Server | Daemon 持有；承载 CLI 管理接入（命令面见 [cli/admin.md](../cli/admin.md)） |
| ArchiveSweeper | Daemon spawn 的系统级后台任务；依赖 SqliteStorage + SessionConfigProvider（见 [session/session-lifecycle.md](../session/session-lifecycle.md)） |
| AnnounceSweeper | Daemon spawn 的系统级后台任务；经 SessionManager 扫描 spawn_tree（见 [session/run-health.md](../session/run-health.md)） |
| ApprovalFlow | Daemon 创建 adapter 包装 permission 的 ApprovalFlow 实现（对应 [common/core-traits](../common/core-traits.md) 的 ApprovalSubmission），注入 Gateway；Daemon 持有该 adapter |
| SpawnController | Daemon 持有；创建并管理子 session（校验/权限判定与阈值来源见 [agent/agent-spawn.md](../agent/agent-spawn.md)）；由 Session 模块在处理 spawn 请求时调用 |
| Config Hot Reload | Daemon spawn 的系统级后台任务；重载校验失败保留旧配置并经 IM 通知 Owner（见 [config/hot-reload.md](../config/hot-reload.md)）；重启类变更触发择机网关重启（见 [restart.md](restart.md)） |
| DreamingScheduler | Daemon spawn 的系统级后台任务；依赖 SqliteStorage 与 MemoryConfigProvider（见 [memory/dreaming.md](../memory/dreaming.md)） |

- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：SkillRegistryQuery、SkillListingProvider、PermissionEvaluator、ApprovalSubmission、ShutdownSignal；消费/装配：LlmCaller、MetricsEmitter、AgentRegistryQuery、SpawnValidator、AgentConfigLookup、TaskManager、AuditLogger——由 Daemon 作为组合根在启动装配时注入到对应消费方（部分实现由 gateway / permission / tasks 等模块提供））
- **无关**：**Processor Chain**（无调用关系）——处理器链由 Gateway 调度，Daemon 不直接参与


