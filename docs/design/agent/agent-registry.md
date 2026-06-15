# AgentRegistry

## 概述

AgentRegistry 是 Agent 模块的核心运行时组件，承担三大职责：

1. **配置查询**：以 `agent_id` 为键提供 `ResolvedAgentConfig` 的只读查找，启动时由 Daemon 从 Config 加载结果一次性填充
2. **Agent 生命周期管理**：提供 `register` / `spawn` / `kill` / `remove` / `send_message` 接口，管理 agent 的创建、进程启动、消息发送、停止和元数据清理
3. **树形层级查询**：基于 agent 的 `parent_id` 关系，提供 `get_children` / `get_parent` / `get_ancestors` / `get_descendants` / `is_ancestor_of` 接口，支持树形层级导航

## 架构

AgentRegistry 内部维护三个并发安全的 HashMap，分别存储 agent 元数据（`Agent`）、进程句柄（`AgentProcessHandle`）和配置（`ResolvedAgentConfig`）。它不涉及权限校验或配置合并——这些职责分别属于 Permission、Config 模块。

```
Daemon 启动 / Config Hot Reload
      │
      │  populate() / reload_config()
      │  Vec<ResolvedAgentConfig>
      ▼
AgentRegistry
      │
      ├── 配置查询层 (configs: HashMap)
      │   get_config(agent_id) → Option<ResolvedAgentConfig>
      │
      ├── 生命周期管理层 (agents: HashMap, processes: HashMap)
      │   register(name, parent_id) → Agent
      │   spawn(name, parent_id, binary_path, ...) → Agent
      │   kill(agent_id) → ()
      │   remove(agent_id) → Agent
      │   send_message(agent_id, message) → ()
      │
      └── 树形层级查询层 (基于 agents 的 parent_id 关系)
          get(agent_id) → Agent
          get_children(parent_id) → Vec<Agent>
          get_parent(agent_id) → Option<Agent>
          get_ancestors(agent_id) → Vec<Agent>
          get_descendants(agent_id) → Vec<Agent>
          is_ancestor_of(ancestor_id, descendant_id) → bool
```

接口职责：

- 启动时由 Daemon 调用 `populate()` 批量填充配置
- 运行时通过 `get_config()` 查询配置，通过 `get()` 查询 agent 元数据
- 生命周期方法 `register()` / `spawn()` / `kill()` / `remove()` / `send_message()` 管理 agent 进程和元数据
- 树查询方法提供基于 `parent_id` 的层级导航

**热重载策略**：Config Hot Reload 检测到 agent 配置变更 → 重新加载 → 通知 Daemon → Daemon 调用 `reload_config()` 替换全部配置。已运行的 session 是否感知变更由各消费模块自行决定——AgentRegistry 只负责提供最新数据，不推送变更通知。

## 数据流

### 启动填充

```
Daemon 启动
  ↓
ConfigManager.load_agents()           // 加载配置到 ConfigManager 内部
  ↓
cm.agents() → HashMap<String, ResolvedAgentConfig>
  ↓
AgentRegistry.populate(Vec<ResolvedAgentConfig>)  // 批量填充 configs
  ↓
注册表就绪，各消费模块可查询
```

> `register()` 是独立的 agent 生命周期方法（创建元数据），与 config 填充无关。
> `populate()` 仅填充 `configs` HashMap，不涉及 `agents` HashMap。

### 运行时查询

```
模块需要 agent 配置
  ↓
AgentRegistry.get_config(agent_id) → Option<ResolvedAgentConfig>
  ├── 命中 → 返回 ResolvedAgentConfig（异步、owned 值）
  └── 未命中 → None（调用方自行处理，通常是配置缺失错误）

模块需要 agent 元数据
  ↓
AgentRegistry.get(agent_id) → RegistryResult<Agent>
  ├── 命中 → 返回 Agent（id / name / parent_id / created_at）
  └── 未命中 → AgentNotFound 错误
```

### 热重载

```
Config Hot Reload 检测 agent 配置变更
  ↓
ConfigManager.reload_agents() → Result<(), ConfigLoadError>  // 重新加载配置文件
  ↓
cm.agents() → HashMap<String, ResolvedAgentConfig>  // 获取最新配置
  ↓
AgentRegistry.reload_config(configs)  // 全量替换 configs
  ↓
注册表内容替换，消费模块下次查询获取新数据
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Daemon | 启动时调用 `populate()` 填充注册表 |
| Config Hot Reload | 检测到 agent 配置变更后通知 Daemon，Daemon 调用 reload() 更新注册表 |
| Config | 提供 `ResolvedAgentConfig` 数据源（Config 只负责文件 IO 和合并，不参与注册） |

### 下游

| 模块 | 消费方式 |
|------|---------|
| Session | 创建 session 时查询 agent 配置（模型、workspace、工具集、skill 列表等） |
| System Prompt | 查询 bootstrap 模式配置 |
| Skills Registry | 查询 agent 的 skills 白名单 |
| Tools Registry | 查询 agent 的 tools 白名单 / 黑名单 |

### 无关

| 模块 | 说明 |
|------|------|
| Gateway | AgentRegistry 不参与消息路由 |
| IM Adapter | AgentRegistry 不涉及平台通信 |
| Processor Chain | AgentRegistry 不参与消息处理 |
