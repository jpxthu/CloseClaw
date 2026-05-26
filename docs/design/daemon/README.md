# Daemon

## 概述

Daemon 是进程入口和组件胶水层。它负责系统启动时的组件初始化与依赖注入、后台任务启动，以及优雅关闭。Daemon 自身不含业务逻辑。

## 架构

Daemon 按依赖关系顺序初始化所有组件，将各组件连接成可运行的整体，启动后台任务后进入消息循环。

```
Daemon 启动
  ├── 阶段一：基础设施
  │     Config 加载 → Storage 初始化 → SessionConfigProvider 初始化 → Permission Engine → Agent Config 扫描
  ├── 阶段二：能力注册
  │     Tools Registry → Skills Registry
  ├── 阶段三：运行时组件
  │     Session Manager（持有 storage、agent config、tool/skill registry）
  │     System Prompt Builder（由 Session Manager 持有）
  ├── 阶段四：消息通道
  │     Processor Registry → Renderer Set → IM Adapters → Gateway
  ├── 阶段五：后台任务
  │     ArchiveSweeper spawn → Skill Watcher spawn
  └── 阶段六：附加入口
        ChatServer 启动（TCP，通过 Gateway 路由消息）
  →
进入消息循环（Gateway 接管）
```

初始化顺序由依赖关系决定——被依赖的组件先初始化，完成后注入给依赖方。

Daemon 持有 SessionManager 和 Gateway 的引用，作为二者的所有者管理其生命周期。

## 数据流

### 启动路径

```
进程启动
  →
Daemon 启动
  ├── 1. Config 加载（多文件合并、凭据分离）
  ├── 2. Storage 初始化（SqliteStorage）
  ├── 3. SessionConfigProvider 初始化（读取 session_config.json，提供 per-agent 的 idle/purge 阈值）
  ├── 4. Permission Engine 初始化（加载规则 + 默认策略）
  ├── 5. Agent Config 扫描（三级扫描 + 字段合并 → 注册表）
  ├── 6. Tools Registry + Skills Registry（注册所有工具和 skill）
  ├── 7. Session Manager 创建（注入 storage、agent config、tool/skill registry）
  ├── 8. System Prompt Builder 创建（由 Session Manager 持有引用）
  ├── 9. Processor Registry（注册入站/出站处理器，按 priority 排序）
  ├── 10. Renderer Set（各平台 Renderer 注册）
  ├── 11. IM Adapters（各平台 Adapter 创建，注入对应 Renderer）
  ├── 12. Gateway 创建（注入 adapters、processor registry、renderers、session manager、permission）
  ├── 13. ArchiveSweeper spawn（独立后台任务，定时扫描 idle session 归档 + 过期 archive 清理，详见 session 文档）
  ├── 14. Skill Watcher spawn（独立后台任务，监听 skill 文件变更）
  ├── 15. ChatServer 启动（TCP 服务器，通过 Gateway 路由消息）
  └── 进入消息循环
```

### 关闭路径

```
SIGINT / SIGTERM
  →
Daemon 关闭
  ├── 1. 停止附加入口（ChatServer 停止 TCP 监听）
  ├── 2. 停止接收新消息（IM Adapters 关闭入站接收）
  ├── 3. 等待进行中的 Session 完成（超时强制终止）
  ├── 4. 停止后台任务（ArchiveSweeper、Skill Watcher）
  ├── 5. 持久化所有活跃 Session
  ├── 6. 关闭消息通道出站（IM Adapters 停止发送、Gateway 清理注册表）
  ├── 7. 关闭 Storage 连接
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
| System Prompt Builder | 由 Session Manager 持有 |
| Processor Registry | 启动时注册处理器 |
| Renderer Set | 启动时注册各平台 Renderer |
| IM Adapters | 启动时创建各平台适配器 |
| Gateway | 启动时创建并注入依赖，Daemon 持有其所有权 |
| ArchiveSweeper | 启动时 spawn 后台任务（依赖 Storage + SessionConfigProvider，行为由 session 模块定义） |
| Skill Watcher | 启动时 spawn 后台任务 |
| ChatServer | 启动时创建 TCP 服务器 |

### 无关

- **LLM Provider**（无调用关系）：Daemon 不调用 LLM
- **Processor Chain**（无调用关系）：处理器链由 Gateway 调度，Daemon 不直接参与
- **Slash Command**（无调用关系）：斜杠指令由 Gateway 拦截分派
- **Renderer**（无调用关系）：渲染由 Gateway 选择和调度


