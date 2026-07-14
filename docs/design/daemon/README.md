# Daemon

## 概述

- 关联需求文档：[requirements/daemon.md](../requirements/daemon.md)
- 一句话：Daemon 是进程入口和组件胶水层，负责系统启动时的组件初始化与依赖注入、后台任务启动，以及优雅关闭。Daemon 自身不含业务逻辑。

## 架构

### 依赖驱动的启动顺序

启动采用依赖声明模型：每个组件声明自身依赖，启动时拓扑排序确定执行顺序。同层组件并行初始化，同层内按组件名称字母序执行以保证确定性。存在循环依赖时拒绝启动并报错。

各组件依赖关系及所属层由声明自动推导，以下为当前已知的代表性层级：

| 层 | 组件 | 依赖 |
|----|------|------|
| 1 | ConfigManager | 无 |
| 1 | Storage | 无 |
| 2 | SessionConfigProvider | ConfigManager |
| 2 | AgentRegistry | ConfigManager |
| 2 | Config Hot Reload | ConfigManager |
| 2 | Skills Registry | ConfigManager（创建注册表骨架，加载 bundled skills） |
| 2 | Renderers / Plugins | ConfigManager |
| 3 | IM Adapters | Renderers, ConfigManager |
| 3 | Permission Engine | AgentRegistry |
| 3 | Tools Registry | Skills Registry |
| 3 | ArchiveSweeper | Storage, SessionConfigProvider |
| 3 | Skill Watcher | Skills Registry |
| 3 | SpawnController | AgentRegistry |
| 3 | DreamingScheduler | Storage, SessionConfigProvider |
| 3 | System Prompt 构建器 | AgentRegistry, Skills Registry |
| 4 | Session Manager | Storage, AgentRegistry, Skills Registry, Tools Registry |
| 4 | ApprovalFlow | Permission Engine, AgentRegistry |
| 5 | Gateway | Session Manager, IM Adapters, Permission Engine, ApprovalFlow |
| 5 | Admin RPC Server | Gateway |

初始化完成后进入消息循环，由 Gateway 接管所有消息处理。

Daemon 持有 SessionManager、Gateway 和 ApprovalFlow 的引用，管理其生命周期。

### 子功能

| 文档 | 简述 |
|------|------|
| [shutdown.md](shutdown.md) | 关闭全流程：ShutdownHandle 协调器、graceful/forceful 双模、阶段化执行、recovery 衔接、用户可见进度通知 |

## 数据流

### 启动路径

```
进程启动
  →
Daemon 启动（依赖驱动，按拓扑序分层执行）
  │
  ├── 层 1（无依赖，并行初始化）
  │   ├── ConfigManager（多文件合并、凭据分离、环境变量加载、主配置文件迁移）
  │   └── Storage（初始化持久化存储）
  │
  ├── 层 2（依赖层 1，并行初始化）
  │   ├── SessionConfigProvider（ConfigManager 加载后作为独立组件暴露，提供 per-agent 的 idle/purge 阈值）
  │   ├── AgentRegistry（创建空注册表 → ConfigManager 加载 agent 配置 → populate 填充）
  │   ├── Config Hot Reload（spawn 后台任务，监听配置文件变更，触发增量/全量重载）
  │   ├── Skills Registry（创建注册表骨架，加载 bundled skills）
  │   └── Renderers / Plugins（各平台 Renderer 封装为 Plugin 并注册）
  │
  ├── 层 3（依赖层 2，并行初始化）
  │   ├── IM Adapters（各平台 Adapter 创建，注入对应 Renderer）
  │   ├── Permission Engine（加载全局默认策略，Agent 维度规则延迟加载）
  │   ├── Tools Registry（各模块注册工具定义，SpawnController 注入）
  │   ├── ArchiveSweeper（spawn 后台任务，定时扫描 idle session 归档 + 过期 archive 清理，详见 [session-lifecycle.md](../session/session-lifecycle.md)）
  │   ├── Skill Watcher（spawn 后台任务，监听 skill 文件变更）
  │   ├── SpawnController（校验 Agent spawn 权限，注入 ToolRegistry）
  │   ├── DreamingScheduler（spawn 后台任务，定时扫描 archived 会话，触发记忆挖掘与升格）
  │   └── System Prompt 构建器（SessionManager 内部调用构建函数，持有 Prompt 覆盖配置，初始为空）
  │
  ├── 层 4（依赖层 3）
  │   ├── Session Manager（注入 storage、agent registry、tool/skill registry，初始化完成后执行启动恢复扫描）
  │   └── ApprovalFlow（注入 Permission Engine、AgentRegistry）
  │
  ├── 层 5（依赖层 4）
  │   ├── Gateway（注入 adapters、processor registry、renderers、session manager、permission；安装 SlashDispatcher；注入 ApprovalFlow）
  │   └── Admin RPC Server（启动 Unix domain socket 管理服务，接收 CLI Admin 命令）
  │
  └── 进入消息循环
```

### 关闭路径

Daemon 关闭由 ShutdownHandle 统一协调，分阶段执行。详见 [shutdown.md](shutdown.md)。

高层概览：

```
信号到达
  ↓
ShutdownHandle 判定模式（Graceful / Forceful）
  ↓
关闭入站接收 + Drain 已有消息
  ↓
Session 停止（委托 SessionManager，graceful 模式等工具完成、LLM 流结束再停；forceful 模式立即 kill）
  ↓
停止后台任务
  ↓
最终持久化 + 关闭出站 + 关闭存储
  ↓
退出
```

Graceful 模式由用户掌控节奏：接收进度通知，可随时升级为 forceful。Forceful 不做等待，依赖 recovery 在下次启动时恢复未完成操作。

## 模块关系

### 上游

操作系统进程管理器。

### 下游（Daemon 初始化/管理哪些模块）

| 模块 | 关系 |
|------|------|
| ConfigManager | 启动时加载各配置文件，合并为各组件所需的数据结构 |
| Storage | 启动时初始化持久化存储 |
| SessionConfigProvider | 启动时加载 session_config.json，提供给 ArchiveSweeper 和 Session Manager |
| Permission Engine | 启动时加载权限规则 |
| AgentRegistry | 启动时创建 agent 注册表，从 ConfigManager 加载结果填充。Daemon 持有其所有权 |
| Tools Registry | 启动时注册所有工具 |
| Skills Registry | 启动时创建注册表骨架，加载 bundled skills |
| Session Manager | 启动时创建并注入依赖，Daemon 持有其所有权 |
| System Prompt 构建器 | SessionManager 内部调用系统 prompt 构建函数组装，持有 PromptOverrides（初始 None） |
| Renderers / Plugins | 启动时注册各平台 Renderer |
| IM Adapters | 启动时创建各平台适配器 |
| Gateway | 启动时创建并注入依赖，Daemon 持有其所有权 |
| Admin RPC Server | 启动时创建 Unix domain socket 管理服务，接收 CLI Admin 命令 |
| ArchiveSweeper | 启动时 spawn 后台任务（依赖 Storage + SessionConfigProvider，详见 [session-lifecycle.md](../session/session-lifecycle.md)） |
| ApprovalFlow | 启动时创建并注入 Gateway，Daemon 持有其所有权 |
| SpawnController | 启动时创建并注入 ToolRegistry，校验 Agent spawn 权限 |
| Config Hot Reload | 启动时 spawn 后台任务，监听配置文件变更并触发重载 |
| Skill Watcher | 启动时 spawn 后台任务 |
| DreamingScheduler | 定时扫描 archived 会话触发记忆挖掘与升格（先 dreaming 后 mining） |

### 无关

- **Processor Chain**（无调用关系）：处理器链由 Gateway 调度，Daemon 不直接参与


