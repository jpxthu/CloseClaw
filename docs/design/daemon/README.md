# Daemon

## 概述

Daemon 是进程入口和组件胶水层。它负责系统启动时的组件初始化与依赖注入、后台任务启动，以及优雅关闭。Daemon 自身不含业务逻辑。

## 架构

Daemon 启动分为五个阶段，按依赖顺序依次初始化：

```
基础设施 → 能力注册 → 运行时组件 → 消息通道 → 后台任务
```

- **基础设施**：Config 加载（含 .env + openclaw.json 迁移）、Storage 初始化、SessionConfigProvider 初始化
- **能力注册**：AgentRegistry、Permission Engine、Agent Config 扫描、Tools Registry + DiskSkillRegistry
- **运行时组件**：Session Manager、System Prompt 构建、Renderer / Plugin 注册
- **消息通道**：IM Adapters、Gateway（含 SlashDispatcher + ApprovalFlow 注入）
- **后台任务**：ArchiveSweeper、Skill Watcher、Config Hot Reload

初始化完成后进入消息循环，由 Gateway 接管所有消息处理。

Daemon 持有 SessionManager 和 Gateway 的引用，作为二者的所有者管理其生命周期。

## 数据流

### 启动路径

```
进程启动
  →
Daemon 启动
  ──── 基础设施 ────
  ├── 1. Config 加载（多文件合并、凭据分离、.env 加载、openclaw.json 迁移）
  ├── 2. Storage 初始化（SqliteStorage）
  ├── 3. SessionConfigProvider 初始化（读取 session_config.json，提供 per-agent 的 idle/purge 阈值）
  ──── 能力注册 ────
  ├── 4. AgentRegistry 初始化
  ├── 5. Permission Engine 初始化（加载全局默认策略，Agent 维度规则延迟加载）
  ├── 6. Agent Config 扫描（两级优先级扫描 + 字段合并 → 注册表）
  ├── 7. Tools Registry + DiskSkillRegistry（各模块 register_tools() 注册工具定义，SpawnController 注入；扫描五层 skill 目录）
  ──── 运行时组件 ────
  ├── 8. Session Manager 创建（注入 storage、agent config、tool/skill registry）
  ├── 9. System Prompt 构建（SessionManager 内部调用构建函数，持有 PromptOverrides，初始为 None）
  ├── 10. Renderer / Plugin 注册（各平台 Renderer 与 Adapter 封装为 Plugin 并注册）
  ──── 消息通道 ────
  ├── 11. IM Adapters（各平台 Adapter 创建，注入对应 Renderer）
  ├── 12. Gateway 创建（注入 adapters、processor registry、renderers、session manager、permission；安装 SlashDispatcher；注入 ApprovalFlow）
  ──── 后台任务 ────
  ├── 13. ArchiveSweeper spawn（独立后台任务，定时扫描 idle session 归档 + 过期 archive 清理，详见 session 文档）
  ├── 14. Skill Watcher spawn（独立后台任务，监听 skill 文件变更）
  ├── 15. Config Hot Reload spawn（监听配置文件变更，触发增量/全量重载）
  └── 进入消息循环
```

### 关闭路径

Daemon 收到关闭信号后分阶段执行。不做 session 粒度的停止操作——全部委托 SessionManager 统一处理。SessionManager 负责遍历 session 树、按正确顺序停止、处理超时和脏标记。

```
SIGTERM（首次）
  →
Daemon graceful 关闭
  ├── 1. 停止接收新消息（IM Adapters 关闭入站接收）
  ├── 2. 关闭所有活跃 Session
  │     委托 SessionManager 以 graceful 模式关闭，含超时参数
  │     Daemon 不感知 session 树结构和停止顺序
  ├── 3. 停止后台任务（ArchiveSweeper、Skill Watcher）
  ├── 4. 持久化所有活跃 Session（CheckpointManager 最终 flush）
  ├── 5. 关闭消息通道出站（IM Adapters 停止发送、Gateway 清理注册表）
  ├── 6. 关闭 Storage 连接
  └── 退出进程（如有 session 超时未完成则标记 dirty，下次启动告警）

SIGTERM（重复）或 SIGINT
  →
Daemon forceful 关闭
  ├── 1. 停止接收新消息
  ├── 2. 委托 SessionManager 以 forceful 模式关闭所有 Session
  ├── 3. 停止后台任务
  ├── 4-6. 同 graceful 路径
  └── 退出进程
```

## 模块关系

### 上游

操作系统进程管理器。

### 下游（Daemon 初始化/管理哪些模块）

| 模块 | 关系 |
|------|------|
| Config | 启动时加载，传入各组件 |
| Storage | 启动时初始化 SqliteStorage |
| SessionConfigProvider | 启动时加载 session_config.json，提供给 ArchiveSweeper 和 Session Manager |
| Permission Engine | 启动时加载权限规则 |
| Agent Config | 启动时扫描合并 agent 配置 |
| Tools Registry | 启动时注册所有工具 |
| Skills Registry | 启动时注册所有 skill |
| Session Manager | 启动时创建并注入依赖，Daemon 持有其所有权 |
| System Prompt | SessionManager 内部调用 build_full_system_prompt() 构建，持有 PromptOverrides（初始 None） |
| Processor Registry | 由 Gateway 内部管理，Daemon 不直接创建 |
| Renderer Set | 启动时注册各平台 Renderer |
| IM Adapters | 启动时创建各平台适配器 |
| Gateway | 启动时创建并注入依赖，Daemon 持有其所有权 |
| ArchiveSweeper | 启动时 spawn 后台任务（依赖 Storage + SessionConfigProvider，详见 Session 设计文档） |
| AgentRegistry | 启动时创建 agent 注册表，Daemon 持有其所有权 |
| ApprovalFlow | 启动时创建并注入 Gateway，Daemon 持有其所有权 |
| SpawnController | 启动时创建并注入 ToolRegistry，校验 Agent spawn 权限 |
| Config Hot Reload | 启动时 spawn 后台任务，监听配置文件变更并触发重载 |
| Skill Watcher | 启动时 spawn 后台任务 |

### 无关

- **LLM Provider**（无调用关系）：Daemon 不调用 LLM
- **Processor Chain**（无调用关系）：处理器链由 Gateway 调度，Daemon 不直接参与
- **Renderer**（无调用关系）：渲染由 Gateway 选择和调度


