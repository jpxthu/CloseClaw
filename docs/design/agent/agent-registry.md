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
| Daemon | 启动时调用 `populate()` 批量填充 configs；运行时通过 `register()` 注册 agent 元数据 |
| Config Hot Reload | 检测到 agent 配置变更后通知 Daemon，Daemon 调用 `reload_config()` 更新注册表 |
| Config | 提供 `ResolvedAgentConfig` 数据源（Config 只负责文件 IO 和合并，不参与注册） |

### 下游

| 模块 | 消费方式 |
|------|---------|
| SessionManager | 持有 `Arc<AgentRegistry>`，通过 `get_agent_config()` 调用 `get_config()` 查询 agent 配置 |
| SessionsSpawnTool | 通过 `BuiltinToolContext.agent_registry` 直接调用 `get_config()` 查询父 agent 配置（模型继承等） |

### 无关

| 模块 | 说明 |
|------|------|
| Gateway | 不直接依赖 AgentRegistry，通过 SessionManager 间接获取 agent 配置 |
| IM Adapter | AgentRegistry 不涉及平台通信 |
| Processor Chain | AgentRegistry 不参与消息处理 |

## 生命周期管理

AgentRegistry 提供五个生命周期方法，管理 agent 的创建、进程启停和元数据清理：

| 方法 | 签名 | 语义 |
|------|------|------|
| `register` | `register(name, parent_id) → RegistryResult<Agent>` | 创建 agent 元数据（UUID 生成 id、记录 name/parent_id/created_at），不启动进程。`parent_id` 可选，用于建立层级关系。 |
| `spawn` | `spawn(name, parent_id, binary_path, bootstrap_minimal, workspace_dir) → RegistryResult<Agent>` | 先调用 `register()` 创建元数据，再调用 `AgentProcess::spawn()` 启动子进程，将进程句柄存入 `processes` HashMap。`binary_path` 必须是绝对路径且文件存在，否则失败并自动清理已注册的元数据。 |
| `kill` | `kill(id) → RegistryResult<()>` | 通过进程句柄终止运行中的 agent 进程。仅影响 `processes` HashMap，不影响元数据。 |
| `remove` | `remove(id) → RegistryResult<Agent>` | 先 `kill()` 终止进程，再从 `processes` 和 `agents` HashMap 中移除对应条目。返回被移除的 Agent 元数据。 |
| `send_message` | `send_message(id, message) → RegistryResult<()>` | 通过进程句柄向 agent 的 stdin 发送消息。若进程不存在则返回 `AgentNotFound` 错误。 |

**错误类型**（`RegistryError`）：
- `AgentNotFound`：按 id 查找 agent 时未找到
- `AgentAlreadyExists`：agent 已存在（预留，当前 `register` 不检查）
- `InvalidStateTransition`：无效状态转换（预留）
- `DestroyConfirmationRequired`：销毁确认（预留）
- `ProcessError`：进程启动/终止失败

## 树形层级查询

AgentRegistry 基于 agent 的 `parent_id` 字段构建树形层级关系，提供以下查询接口：

| 方法 | 签名 | 语义 |
|------|------|------|
| `get` | `get(id) → RegistryResult<Agent>` | 按 id 查找单个 agent 元数据，未找到返回 `AgentNotFound` 错误 |
| `get_children` | `get_children(parent_id) → Vec<Agent>` | 获取 `parent_id` 匹配的直接子 agent 列表，时间复杂度 O(n) |
| `get_parent` | `get_parent(agent_id) → Option<Agent>` | 获取指定 agent 的父级元数据，根节点返回 `None` |
| `get_ancestors` | `get_ancestors(agent_id) → Vec<Agent>` | 沿 `parent_id` 链向上遍历，返回从直接父级到根节点的祖先列表（不含自身） |
| `get_descendants` | `get_descendants(agent_id) → Vec<Agent>` | 广度优先遍历所有后代节点，返回完整后代列表 |
| `is_ancestor_of` | `is_ancestor_of(ancestor_id, descendant_id) → bool` | 判断 `ancestor_id` 是否在 `descendant_id` 的祖先链中 |

**补充查询方法**：
- `list() → Vec<Agent>`：返回所有已注册的 agent 列表
- `count() → usize`：返回已注册 agent 数量

所有树查询方法均基于 `agents` HashMap 的 `parent_id` 字段实时计算，不维护额外索引。

## Agent 元数据存储

AgentRegistry 内部维护三个并发安全的 HashMap，分别存储不同职责的数据：

| 存储 | 类型 | 职责 |
|------|------|------|
| `agents` | `RwLock<HashMap<String, Agent>>` | 存储 agent 元数据（id/name/parent_id/created_at），支持注册、查询、删除 |
| `processes` | `RwLock<HashMap<String, AgentProcessHandle>>` | 存储运行中 agent 的子进程句柄，支持启动、终止、消息发送 |
| `configs` | `RwLock<HashMap<String, ResolvedAgentConfig>>` | 存储 agent 配置（只读查询层），由 `populate()` 批量填充、`reload_config()` 全量替换 |

**设计要点**：
- 三个 HashMap 使用独立的 `RwLock`，读操作（`get_config`/`get`/树查询）不互斥，写操作（`register`/`remove`/`populate`）按 HashMap 粒度加锁
- `agents` 和 `configs` 的生命周期由 Daemon 管理（启动填充/热重载），`processes` 的生命周期由运行时操作管理（spawn/remove）
- `AgentRegistry` 不持有 `Agent` 的进程状态——运行时状态（心跳、活跃度）由 session 模块负责
