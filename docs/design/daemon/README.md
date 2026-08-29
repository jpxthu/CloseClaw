# Daemon

## 概述

- 关联需求文档：[requirements/daemon.md](../../requirements/daemon.md)
- 一句话：Daemon 是进程入口和组件胶水层，负责系统启动时的组件初始化与依赖注入、后台任务启动、配置触发的网关择机重启，以及优雅关闭。Daemon 自身不含业务逻辑。

## 架构

### 依赖驱动的启动顺序

启动采用依赖声明模型：每个组件声明自身依赖，启动时拓扑排序确定执行顺序。同层组件并行初始化，同层内按组件名称字母序执行以保证确定性。存在循环依赖时拒绝启动并报错。

各组件依赖关系及所属层由声明自动推导，完整分层依赖表如下：

| 层 | 组件 | 依赖 |
|----|------|------|
| 1 | ConfigManager | 无 |
| 1 | Storage | 无 |
| 2 | SessionConfigProvider | ConfigManager |
| 2 | AgentRegistry | ConfigManager |
| 2 | Config Hot Reload | ConfigManager（出站通知通道为运行时引用，IM Adapters 就绪后接线，不构成启动依赖） |
| 2 | LLM Registry | ConfigManager |
| 2 | Skills Registry | ConfigManager |
| 2 | Renderers / Plugins | ConfigManager |
| 2 | Permission Engine | ConfigManager |
| 2 | PlanArchiveSweeper | ConfigManager |
| 3 | IM Adapters | Renderers, ConfigManager |
| 3 | Tools Registry | Skills Registry |
| 3 | ArchiveSweeper | Storage, SessionConfigProvider |
| 3 | AnnounceSweeper | Storage, SessionConfigProvider |
| 3 | DreamingScheduler | Storage, SessionConfigProvider |
| 3 | ApprovalFlow | Permission Engine, AgentRegistry |
| 4 | Session Manager | LLM Registry, Storage, AgentRegistry, Skills Registry, Tools Registry, SessionConfigProvider |
| 4 | SpawnController | ConfigManager, AgentRegistry, Permission Engine |
| 4 | System Prompt 构建器 | AgentRegistry, Skills Registry, Tools Registry |
| 5 | Gateway | Session Manager, IM Adapters, Permission Engine, ApprovalFlow, Renderers / Plugins |
| 6 | Admin RPC Server | Gateway |

上表中的后台任务类组件（Config Hot Reload、ArchiveSweeper、AnnounceSweeper、PlanArchiveSweeper、DreamingScheduler）构成 Daemon 级后台任务的权威清单；新增后台任务时必须同步更新本表与 [shutdown.md](shutdown.md) 的后台任务清单及停止设计。

初始化完成后进入消息循环，由 Gateway 接管所有消息处理。

### 配置触发的网关重启

重启类配置变更的判定与变更确认由配置模块完成（见 [config 需求 §F4](../../requirements/config.md)），确认后由 Daemon 执行择机无损重启：

- **待重启状态**：Daemon 进入待重启状态并记录待生效变更，系统正常运行——会话正常处理消息、新消息不受影响，重启执行前继续按旧配置运行
- **变更合并**：待重启期间新到达的重启类变更并入同一次待执行集合，不重复重启
- **择机窗口**：Daemon 经 SessionManager 查询全部会话的四维活跃状态（与关闭流程同一套判定，见 [shutdown.md](shutdown.md)），全部为否即满足窗口；无活跃会话时立即执行
- **执行流程**：会话层完全不动，Daemon 重建 Gateway 及持有其引用的下游组件（如 Admin RPC Server）；执行期间入站消息由 Gateway 暂存、完成后按原到达顺序补投（见 [gateway 需求 §F6](../../requirements/gateway.md)），执行中的出站消息按优雅关闭语义收尾
- **无窗口兜底**：存在活跃会话时持续等待、不强制打断任何会话；Owner 可随时改用强制关闭优先执行（forceful 语义见 [shutdown.md](shutdown.md)）
- **完成通知**：重启完成后经 IM 通知 Owner，附本次生效的配置变更概要

Daemon 持有 AgentRegistry、Session Manager、Gateway、ApprovalFlow、SpawnController 和 System Prompt 构建器的引用，管理其生命周期。

### 子功能

| 文档 | 简述 |
|------|------|
| [shutdown.md](shutdown.md) | 关闭全流程：ShutdownHandle 协调器、graceful/forceful 双模、阶段化执行、recovery 衔接、用户可见进度通知 |

## 数据流

### 启动路径

启动按层序执行：层 1 → 层 2 → 层 3 → 层 4 → 层 5 → 层 6，上一层全部完成后进入下一层；同层内组件并行初始化。

1. **层 1**（无依赖）：
   - ConfigManager（多文件合并、凭据分离、环境变量加载、主配置文件迁移）
   - Storage（初始化持久化存储）
2. **层 2**（依赖层 1）：
   - SessionConfigProvider（ConfigManager 加载后作为独立组件暴露，提供 per-agent 的 idle/purge 阈值）
   - AgentRegistry（创建空注册表 → ConfigManager 加载 agent 配置 → populate 填充）
   - Config Hot Reload（spawn 后台任务，监听配置文件变更，触发增量重载；重载校验失败时经 IM 通知 Owner——出站通道为运行时引用，IM Adapters 就绪后接线，不构成启动依赖，详见 [config/hot-reload.md](../config/hot-reload.md)）
   - Skills Registry（创建注册表骨架，加载 bundled skills）
   - LLM Registry（读取 models.json 供应商定义与凭据，构造 LLM Client（UnifiedChatClient）实例，内部链路详见 [llm/README.md](../llm/README.md)）
   - Renderers / Plugins（各平台 Renderer 封装为 Plugin 并注册）
   - Permission Engine（加载全局默认策略，Agent 维度规则延迟加载）
   - PlanArchiveSweeper（spawn 后台任务，定时扫描「全部步骤终态」的 plan，将最后访问超过配置天数的自动归档；终态定义与归档规则详见 [mode/README.md](../mode/README.md)）
3. **层 3**（依赖层 2）：
   - IM Adapters（各平台 Adapter 创建，注入对应 Renderer）
   - Tools Registry（各模块注册工具定义）
   - ArchiveSweeper（spawn 后台任务，定时扫描 idle session 归档 + 过期 archive 清理；归档前查询 SessionManager 四维活跃状态——该运行时引用在 Session Manager 就绪后接线，不构成启动依赖，详见 [session/session-lifecycle.md](../session/session-lifecycle.md)）
   - AnnounceSweeper（spawn 后台任务，定时扫描 spawn_tree 补推完成通知与僵死检测——扫描经 Session Manager 进行，该运行时引用在 Session Manager 就绪后接线，不构成启动依赖，详见 [session/run-health.md](../session/run-health.md)）
   - DreamingScheduler（spawn 后台任务，定时扫描 archived 会话，触发记忆挖掘与升格）
   - ApprovalFlow（注入 Permission Engine、AgentRegistry）
4. **层 4**（依赖层 3）：
   - Session Manager（注入 LLM Registry 构造的 LLM Client、Storage、AgentRegistry、Tools Registry、Skills Registry、SessionConfigProvider，初始化完成后执行启动恢复扫描）
   - SpawnController（创建并管理子 session；spawn 前置校验与权限判定经 Permission Engine（子 Agent 权限继承、Deny 沿链路传播）、Agent 配置（深度/并发/超时阈值），详见 [agent/agent-spawn.md](../agent/agent-spawn.md)；子 session 所需能力经 Session Manager 提供的 spawn 上下文获取（运行时引用，Session Manager 就绪后接线，不构成启动依赖））
   - System Prompt 构建器（SessionManager 触发构建，持有 AgentRegistry、SkillsRegistry、ToolsRegistry 引用，详见 [system_prompt/README.md](../system_prompt/README.md)）
5. **层 5**（依赖层 4）：Gateway（注入 adapters、session manager、permission、renderers；安装 SlashDispatcher（详见 [slash/README.md](../slash/README.md)）；注入 ApprovalFlow）
6. **层 6**（依赖层 5）：Admin RPC Server（启动 Unix domain socket 管理服务，接收 CLI Admin 命令）
7. 全部完成后**进入消息循环**

### 关闭路径

Daemon 关闭由 ShutdownHandle 统一协调，分阶段执行。详见 [shutdown.md](shutdown.md)。注意：配置触发的网关择机重启（见「配置触发的网关重启」）不经过本关闭流程，仅重建 Gateway 层、会话层不动，执行中的出站消息按优雅关闭语义收尾。

高层概览：

1. 信号到达，ShutdownHandle 判定模式（Graceful / Forceful）
2. 关闭入站接收 + Drain 已有消息
3. Session 停止（委托 SessionManager，graceful 模式等工具完成、LLM 流结束再停；forceful 模式立即 kill）
4. 停止后台任务
5. 最终持久化 + 关闭出站 + 关闭存储
6. 退出

Graceful 模式由用户掌控节奏：接收进度通知，可随时升级为 forceful。Forceful 不做等待，依赖 recovery 在下次启动时恢复未完成操作。

### 配置触发的网关重启路径

1. 配置模块确认重启类变更
2. Daemon 进入待重启状态（系统正常运行，新消息不受影响）
3. 监测择机窗口：经 SessionManager 查全部会话四维活跃状态
   - 有活跃会话 → 持续等待（期间新重启类变更并入同一次）；Owner 可随时改用强制关闭优先执行
   - 无活跃会话 → 立即执行，进入下一步
4. 重建 Gateway 及其下游受影响组件（会话层不动；入站消息由 Gateway 暂存）
5. 暂存消息按原到达顺序补投；出站按优雅关闭语义收尾
6. 经 IM 通知 Owner（附本次生效的配置变更概要）

## 模块关系

- **上游**：操作系统进程管理器。
- **下游**：Daemon 初始化/管理以下模块。

| 模块 | 关系 |
|------|------|
| ConfigManager | 启动时加载各配置文件，合并为各组件所需的数据结构 |
| Storage | 启动时初始化持久化存储 |
| SessionConfigProvider | 启动时加载 session_config.json，提供给各后台扫描任务（ArchiveSweeper、AnnounceSweeper、DreamingScheduler）和 Session Manager |
| Permission Engine | 启动时加载全局默认策略，Agent 维度规则延迟加载 |
| PlanArchiveSweeper | 启动时 spawn 后台任务，定时扫描「全部步骤终态」的 plan，将最后访问超过配置天数的自动归档到 workspace/plans/archive/（终态定义与归档规则见 [mode/README.md](../mode/README.md)） |
| AgentRegistry | 启动时创建 agent 注册表，从 ConfigManager 加载结果填充。Daemon 持有其所有权 |
| Tools Registry | 启动时注册所有工具 |
| Skills Registry | 启动时创建注册表骨架，加载 bundled skills |
| LLM Registry | 启动时读取 models.json 供应商定义与凭据文件，构造 LLM Client（UnifiedChatClient）并注入 Session Manager，由 Session Manager 传递给各 ConversationSession 使用（LLM 模块内部架构详见 [llm/README.md](../llm/README.md)） |
| Session Manager | 启动时创建并注入依赖（LLM Registry 构造的 LLM Client、Storage、AgentRegistry、Tools Registry、Skills Registry、SessionConfigProvider），Daemon 持有其所有权 |
| System Prompt 构建器 | SessionManager 触发构建系统 prompt，持有 AgentRegistry、SkillsRegistry、ToolsRegistry 引用，详见 [system_prompt/README.md](../system_prompt/README.md) |
| Renderers / Plugins | 启动时注册各平台 Renderer |
| IM Adapters | 启动时创建各平台适配器 |
| Gateway | 启动时创建并注入依赖，Daemon 持有其所有权 |
| Admin RPC Server | 启动时创建 Unix domain socket 管理服务，接收 CLI Admin 命令 |
| ArchiveSweeper | 启动时 spawn 后台任务（依赖 Storage + SessionConfigProvider；归档前查询 SessionManager 四维活跃状态（运行时引用，Session Manager 就绪后接线），详见 [session/session-lifecycle.md](../session/session-lifecycle.md)） |
| AnnounceSweeper | 启动时 spawn 后台任务，定时扫描 spawn_tree 补推完成通知与僵死检测（扫描经 Session Manager 进行，运行时引用，详见 [session/run-health.md](../session/run-health.md)） |
| ApprovalFlow | 启动时创建并注入到 Gateway，Daemon 持有其所有权 |
| SpawnController | 启动时创建，负责创建并管理子 session，依赖 ConfigManager、AgentRegistry 与 Permission Engine。spawn 前置校验（深度/并发/白名单）与权限判定在此完成，深度/并发/超时阈值来自 Agent 配置，权限经 Permission Engine（子 Agent 权限继承、Deny 沿链路传播），详见 [agent/agent-spawn.md](../agent/agent-spawn.md)。子 session 所需能力经 Session Manager 提供的 spawn 上下文获取（运行时引用，Session Manager 就绪后接线）。由 Session 模块在处理 spawn 请求时调用 |
| Config Hot Reload | 启动时 spawn 后台任务，监听配置文件变更并触发增量重载；重载校验失败时保留旧配置运行并经 IM 通知 Owner（出站通道为运行时引用，IM Adapters 就绪后接线，不构成启动依赖，详见 [config/hot-reload.md](../config/hot-reload.md)）。变更确认为重启类时触发 Daemon 择机网关重启（见「配置触发的网关重启」） |
| DreamingScheduler | 启动时 spawn 后台任务（依赖 Storage 与 SessionConfigProvider），定时扫描 archived 会话触发记忆挖掘与升格（先 dreaming 后 mining） |

- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：SkillRegistryQuery、SkillListingProvider、PermissionEvaluator、ApprovalSubmission；消费：LlmCaller、MetricsEmitter）
- **无关**：**Processor Chain**（无调用关系）——处理器链由 Gateway 调度，Daemon 不直接参与


