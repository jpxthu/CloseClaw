# Daemon

## 概述

- 关联需求文档：[requirements/daemon.md](../../requirements/daemon.md)
- 一句话：Daemon 是进程入口和组件胶水层，负责系统启动时的组件初始化与依赖注入、系统级后台任务启动、配置触发的网关重启，以及优雅关闭。Daemon 自身不含业务逻辑。

## 架构

### 依赖驱动的启动顺序

启动采用依赖声明模型：每个组件声明自身依赖，启动时拓扑排序确定执行顺序。同层组件并行初始化，同层内按组件名称字母序执行以保证确定性。存在循环依赖时拒绝启动并报错。同层并行使启动耗时收敛于各层关键路径；后台任务类组件以 spawn 方式非阻塞启动，不进入消息接收关键路径，外部入口就绪前不接受消息。

各组件依赖关系及所属层由声明自动推导，完整分层依赖表如下：

| 层 | 组件 | 依赖 |
|----|------|------|
| 1 | ConfigManager | 无 |
| 1 | SqliteStorage | 无 |
| 2 | SessionConfigProvider | ConfigManager |
| 2 | MemoryConfigProvider | ConfigManager |
| 2 | AgentRegistry | ConfigManager |
| 2 | Config Hot Reload | ConfigManager（出站通知通道为运行时引用，IM Adapters 就绪后接线，不构成启动依赖） |
| 2 | LLM Registry | ConfigManager |
| 2 | SkillRegistry | ConfigManager |
| 2 | Renderers / Plugins | ConfigManager |
| 2 | Permission Engine | ConfigManager |
| 2 | PlanArchiveSweeper | ConfigManager |
| 3 | IM Adapters | Renderers, ConfigManager |
| 3 | ToolRegistry | SkillRegistry |
| 3 | ArchiveSweeper | SqliteStorage, SessionConfigProvider |
| 3 | AnnounceSweeper | SqliteStorage, SessionConfigProvider |
| 3 | DreamingScheduler | SqliteStorage, MemoryConfigProvider |
| 3 | ApprovalFlow | Permission Engine, AgentRegistry |
| 4 | SessionManager | LlmCaller, SqliteStorage, AgentRegistry, SkillRegistry, ToolRegistry, SessionConfigProvider |
| 4 | SpawnController | AgentRegistry, ToolRegistry |
| 4 | System Prompt 构建器 | AgentRegistry, SkillRegistry, ToolRegistry |
| 5 | Gateway | SessionManager, IM Adapters, Permission Engine, ApprovalFlow, Renderers / Plugins |
| 6 | Admin RPC Server | Gateway |

上表中的后台任务类组件（Config Hot Reload、ArchiveSweeper、AnnounceSweeper、PlanArchiveSweeper、DreamingScheduler）构成系统级后台任务的权威清单；新增系统级后台任务时必须同步更新本表与 [shutdown.md](shutdown.md) 的系统级后台任务清单及停止设计。

初始化完成后进入消息循环，由 Gateway 接管所有消息处理。

### 配置触发的网关重启

重启类配置变更的判定（判定标准与归属矩阵）依据 [config 需求 §F7](../../requirements/config.md)（生效机制与重启类判定），变更确认见 [config 需求 §F4](../../requirements/config.md)（配置重载）；确认后由 Daemon 择机执行无损重启：

- **待重启状态**：Daemon 进入待重启状态并记录待生效变更，系统正常运行——会话正常处理消息、新消息不受影响，重启执行前继续按旧配置运行
- **变更合并**：待重启期间（重启执行前）新到达的重启类变更并入同一次待执行集合，不重复重启；重建执行期间到达的重启类变更并入下一次待执行集合
- **择机窗口**：Daemon 经 SessionManager 查询全部会话的四维活跃维度（与关闭流程同一套判定，见 [shutdown.md](shutdown.md)），全部为否即满足窗口；无活跃会话时立即执行
- **执行流程**：会话层完全不动，Daemon 重建 Gateway 及因持有 Gateway 引用而需随之重建的下游组件（如 Admin RPC Server——其管理接口连接在重建期间断开，CLI Chat / CLI Admin 按各自重连语义处理）；重建期间的入站消息由 Gateway 的持久化入站队列承载并跨越重建窗口，新 Gateway 就绪后按原到达顺序补投（见 [gateway 需求 §F6](../../requirements/gateway.md)、[gateway 消息队列](../gateway/README.md)）；执行中的出站消息按 graceful 语义收尾（不涉及 forceful 场景）
- **重建失败兜底**：新 Gateway 初始化或接线失败时中止本次重建，保留旧组件与旧配置继续运行，并经 IM 通知 Owner（会话层始终不受影响）
- **无窗口兜底**：存在活跃会话时持续等待、不强制打断任何会话；Owner 可随时改用强制关闭——即放弃本次待执行重启、改走完整 forceful 关闭流程（[shutdown.md](shutdown.md)），下次启动时配置自然生效
- **完成通知**：重启完成后经 IM 通知 Owner，附本次生效的配置变更概要

Daemon 作为组合根，创建并持有下表全部组件（含 ConfigManager、SqliteStorage）的引用，管理其生命周期；各组件的所有权与装配关系见「模块关系」。

### 子功能

| 文档 | 简述 |
|------|------|
| [shutdown.md](shutdown.md) | 关闭全流程：ShutdownHandle 协调器、graceful/forceful 双模、阶段化执行、recovery 衔接、用户可见进度通知 |

## 数据流

### 启动路径

启动按层序执行：层 1 → 层 2 → 层 3 → 层 4 → 层 5 → 层 6，上一层全部完成后进入下一层；同层内组件并行初始化。

1. **层 1**（无依赖）：
   - ConfigManager（多文件合并、凭据分离、环境变量加载、主配置文件迁移）
   - SqliteStorage（初始化持久化存储）
2. **层 2**（依赖层 1）：
   - SessionConfigProvider（ConfigManager 加载后作为独立组件暴露，提供 per-agent 的 idle/purge 阈值与扫描间隔等会话配置，供 ArchiveSweeper、AnnounceSweeper、SessionManager 消费）
   - MemoryConfigProvider（ConfigManager 加载后作为独立组件暴露，提供记忆子系统配置（含 dreaming.schedule），供 DreamingScheduler 消费）
   - AgentRegistry（创建空注册表 → ConfigManager 加载 agent 配置 → populate 填充）
   - Config Hot Reload（spawn 后台任务，监听配置文件变更，触发增量重载；重载校验失败时保留旧配置运行并经 IM 通知 Owner——出站通道为运行时引用，IM Adapters 就绪后接线，不构成启动依赖，详见 [config/hot-reload.md](../config/hot-reload.md)）
   - SkillRegistry（创建注册表骨架，加载 bundled skills）
   - LLM Registry（Daemon 侧的模型装配组件：从 ConfigManager 取得 models.json 供应商定义与凭据，构造统一 LLM Client（UnifiedChatClient）实例，见 [llm/README.md](../llm/README.md)；无可用模型/供应商时构造为空、不阻塞启动）
   - Renderers / Plugins（各平台 Renderer 封装为 Plugin 并注册）
   - Permission Engine（加载全局默认策略，Agent 维度规则延迟加载）
   - PlanArchiveSweeper（spawn 后台任务，定时扫描「全部步骤终态」的 plan，将最后访问超过配置天数的自动归档；终态定义与归档规则详见 [mode/README.md](../mode/README.md)）
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
   - System Prompt 构建器（SessionManager 触发构建，持有 AgentRegistry、SkillRegistry、ToolRegistry 引用，详见 [system_prompt/README.md](../system_prompt/README.md)）
5. **层 5**（依赖层 4）：Gateway（注入 adapters、SessionManager、permission、renderers；安装 SlashDispatcher（详见 [slash/README.md](../slash/README.md)）；注入 ApprovalFlow）
6. **层 6**（依赖层 5）：Admin RPC Server（启动 Unix domain socket 管理服务，承载 CLI 的管理接入：一是接收 CLI Admin 命令（agent/skill 查询，命令面见 [cli/admin.md](../cli/admin.md)）；二是接纳 CLI Chat 经该连接接入的 terminal 渠道——terminal 插件注册、消息链路与启动存活检测（不可达则 CLI Chat 报错退出、不自动拉起 daemon，见 [cli/chat.md](../cli/chat.md)））
7. 全部完成后**进入消息循环**

**LLM 能力缺失时的行为**：若启动时 LLM 能力不可用（如 models.json 缺失、未配置任何可用模型/供应商，Daemon 无法组装 LlmCaller），系统仍正常启动，不静默丢弃用户消息——Daemon 未向会话注入 LlmCaller 时，收到需 LLM 处理的消息以明确错误回复告知用户 LLM 未就绪，而非假装处理或丢失消息。LlmCaller 在层 2 LLM Registry 就绪后由 Daemon 构造、层 4 注入 SessionManager 并接管至各会话；补齐 LLM 配置属重启类变更（见 [config 需求 §F7](../../requirements/config.md)），需完整重启系统以重走启动路径——配置触发的网关重启仅重建 Gateway 及因持有其引用而需随之重建的下游组件、会话层不动，无法重新完成会话层的 LlmCaller 接线。

### 关闭路径

Daemon 关闭由 ShutdownHandle 统一协调，分阶段执行。详见 [shutdown.md](shutdown.md)。注意：配置触发的网关重启（见「配置触发的网关重启」）不经过本关闭流程，仅重建 Gateway 及因持有其引用而需随之重建的下游组件、会话层不动；执行中的出站消息按 graceful 语义收尾（该路径不涉及 forceful 场景）。

高层概览：

1. 信号到达（经 platform 模块订阅的关闭信号），ShutdownHandle 判定模式（graceful / forceful）
2. 关闭入站接收 + Drain 已有消息
3. Session 停止（委托 SessionManager，graceful 模式等工具完成、LLM 流结束再停；forceful 模式立即 kill）
4. 停止系统级后台任务
5. 最终持久化 + 关闭出站 + 关闭存储
6. 退出

关闭开始时已接收、尚未处理完的消息不静默丢弃——承接路径（drain 等待 + Gateway 持久化入站队列重启重放，at-least-once + 去重）详见 [shutdown.md](shutdown.md) Phase 1。

graceful 模式由用户掌控节奏：接收进度通知，可随时升级为 forceful。forceful 不做等待，依赖 recovery 在下次启动时恢复未完成操作。

### 配置触发的网关重启路径

配置模块确认重启类变更 → Daemon 进入待重启状态（系统正常运行，新消息不受影响）→ 择机窗口满足（经 SessionManager 查全部会话四维活跃维度均为否）→ 重建 Gateway 及因持有其引用而需随之重建的下游组件（会话层不动；入站消息由 Gateway 持久化入站队列承载）→ 补投暂存消息、出站按 graceful 语义收尾 → 经 IM 通知 Owner（附本次生效的配置变更概要）。

机制的完整说明（待重启状态、变更合并、执行流程、重建失败兜底、无窗口兜底、完成通知）见「架构 / 配置触发的网关重启」。

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

- **上游**：[platform 模块](../platform/README.md)（操作系统进程管理器——Daemon 经其进程管理接口自注册 PID、订阅关闭信号（SIGTERM / SIGINT），并经其配置目录接口确定配置根目录；进程、信号、配置目录等 OS 差异统一由其封装消化）。
- **下游**：Daemon 初始化/管理以下模块。

| 模块 | 关系 |
|------|------|
| ConfigManager | 启动时加载各配置文件，合并为各组件所需的数据结构 |
| SqliteStorage | 启动时初始化持久化存储 |
| SessionConfigProvider | 启动时加载 session.json，提供给各后台扫描任务（ArchiveSweeper、AnnounceSweeper）和 SessionManager |
| MemoryConfigProvider | 启动时加载 memory.json，提供给 DreamingScheduler（记忆子系统配置，含 dreaming.schedule） |
| Permission Engine | 启动时加载全局默认策略，Agent 维度规则延迟加载 |
| PlanArchiveSweeper | 启动时 spawn 后台任务，定时扫描「全部步骤终态」的 plan，将最后访问超过配置天数的自动归档到 workspace/plans/archive/（终态定义与归档规则见 [mode/README.md](../mode/README.md)） |
| AgentRegistry | 启动时创建 agent 注册表，从 ConfigManager 加载结果填充。Daemon 持有其所有权 |
| ToolRegistry | 启动时注册所有工具 |
| SkillRegistry | 启动时创建注册表骨架，加载 bundled skills |
| LLM Registry | 启动时从 ConfigManager 取得 models.json 供应商定义与凭据，构造统一 LLM Client（LLM 模块内部架构详见 [llm/README.md](../llm/README.md)），供 Daemon 组装 LlmCaller 使用 |
| SessionManager | 启动时创建并注入依赖（Daemon 构造的 LLM 调用器 LlmCaller、SqliteStorage、AgentRegistry、ToolRegistry、SkillRegistry、SessionConfigProvider），Daemon 持有其所有权 |
| System Prompt 构建器 | SessionManager 触发构建系统 prompt，持有 AgentRegistry、SkillRegistry、ToolRegistry 引用，详见 [system_prompt/README.md](../system_prompt/README.md) |
| Renderers / Plugins | 启动时注册各平台 Renderer |
| IM Adapters | 启动时创建各平台适配器 |
| Gateway | 启动时创建并注入依赖，Daemon 持有其所有权 |
| Admin RPC Server | 启动时创建 Unix domain socket 管理服务，承载 CLI 的管理接入：接收 CLI Admin 命令（agent/skill 查询，命令面见 [cli/admin.md](../cli/admin.md)），并接纳 CLI Chat 的 terminal 渠道连接（[TerminalPlugin](../cli/chat.md) 插件注册、消息链路、启动存活检测） |
| ArchiveSweeper | 启动时 spawn 后台任务（依赖 SqliteStorage + SessionConfigProvider；归档前查询 SessionManager 四维活跃维度（运行时引用，SessionManager 就绪后接线），详见 [session/session-lifecycle.md](../session/session-lifecycle.md)） |
| AnnounceSweeper | 启动时 spawn 后台任务，定时扫描 spawn_tree 补推完成通知与僵死检测（扫描经 SessionManager 进行，运行时引用，详见 [session/run-health.md](../session/run-health.md)） |
| ApprovalFlow | Daemon 启动时创建 adapter 包装 permission 的 ApprovalFlow 实现（对应 [common/core-traits](../common/core-traits.md) 的 ApprovalSubmission，由 Daemon 实现），并注入到 Gateway；Daemon 持有该 adapter 的所有权 |
| SpawnController | 启动时创建，负责创建并管理子 session，启动依赖 AgentRegistry、ToolRegistry；ConfigManager、Permission Engine 为构造注入的运行时引用。spawn 前置校验（深度/并发/白名单）与权限判定在此完成，深度/并发/超时阈值来自 Agent 配置，权限经 Permission Engine（子 Agent 权限继承、Deny 沿链路传播），详见 [agent/agent-spawn.md](../agent/agent-spawn.md)。子 session 所需能力经 SessionManager 提供的 spawn 上下文获取（运行时引用，SessionManager 就绪后接线）。由 Session 模块在处理 spawn 请求时调用 |
| Config Hot Reload | 启动时 spawn 后台任务，监听配置文件变更并触发增量重载；重载校验失败时保留旧配置运行并经 IM 通知 Owner（出站通道为运行时引用，IM Adapters 就绪后接线，不构成启动依赖，详见 [config/hot-reload.md](../config/hot-reload.md)）。变更确认为重启类时触发 Daemon 择机网关重启（见「配置触发的网关重启」） |
| DreamingScheduler | 启动时 spawn 后台任务（依赖 SqliteStorage 与 MemoryConfigProvider），定时扫描 archived 会话触发记忆挖掘与升格（先 dreaming，见 [memory/dreaming.md](../memory/dreaming.md)；后 mining） |

- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：SkillRegistryQuery、SkillListingProvider、PermissionEvaluator、ApprovalSubmission、ShutdownSignal；消费：LlmCaller、MetricsEmitter、AgentRegistryQuery、SpawnValidator、AgentConfigLookup、TaskManager、AuditLogger）
- **无关**：**Processor Chain**（无调用关系）——处理器链由 Gateway 调度，Daemon 不直接参与


