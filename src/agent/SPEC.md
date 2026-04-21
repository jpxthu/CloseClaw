# Agent 模块规格书

> 本文档描述 `src/agent/` 模块的精确功能说明，以代码为准。

---

## 1. 模块概述

Agent 模块负责 CloseClaw 多智能体系统的核心运行时管理。

**核心设计**：每个 Agent 是一个独立进程，通过 stdin/stdout 以 JSON 行协议与父进程通信。AgentRegistry 作为中心注册表，管理所有 Agent 实例的生命周期。消息送达由 InboxManager 保证，支持持久化、重试和死信处理。

**边界**：不涉及 LLM 调用（llm/）、平台适配（im/）、权限引擎执行（permission/）；通过 AgentRegistry 被 session 和 gateway 引用。

---

## 2. 公开接口

### 2.1 构造

- `Agent::new` — 创建新 Agent 实例（Idle 状态）
- `AgentRegistry::new` — 创建带心跳超时的注册表
- `AgentRegistry::new_with_graceful_shutdown` — 创建带优雅关闭三参数（心跳/等待/宽限期）的注册表
- `create_registry` — 创建共享注册表（Arc 包装）
- `InboxManager::new` — 创建收件箱管理器（异步）

### 2.2 配置

- `AgentConfig::load` — 从 JSON 文件加载配置
- `AgentConfig::save` — 将配置写入 JSON 文件
- `AgentPermissions::load` — 从 JSON 文件加载权限
- `AgentPermissions::save` — 将权限写入 JSON 文件
- `CommunicationConfig::default_with_parent` — 基于父 Agent 创建仅允许父子通信的白名单
- `CommunicationConfig::can_send_to` — 检查目标是否在出站白名单中（支持 `*` 通配符）
- `CommunicationConfig::can_receive_from` — 检查源是否在入站白名单中（支持 `*` 通配符）
- `AgentPermissions::is_allowed` — 检查指定 action 是否在权限范围内
- `check_communication_allowed` — 中心仲裁：双向检查两个 Agent 之间是否允许互通
- `check_max_depth` — 沿父链回溯计算当前深度，检查是否超过 max_child_depth 上限
- `InboxManager::config` — 获取收件箱配置引用

### 2.3 主操作

- `AgentRegistry::register` — 注册新 Agent（仅元数据，不启动进程）
- `AgentRegistry::spawn` — 注册 + 验证二进制路径 + 启动子进程（失败时回滚注册）
- `AgentRegistry::update_state` — 更新 Agent 状态（含合法性校验）
- `AgentRegistry::update_heartbeat` — 更新指定 Agent 的心跳时间戳
- `AgentRegistry::stop_agent` — 停止 Agent（可选级联停止后代，先子后父）
- `AgentRegistry::suspend_agent` — 暂停 Agent（可选级联暂停后代；Forced 原因在暂停前额外执行 save_checkpoint）
- `AgentRegistry::resume_agent` — 恢复 Agent 运行（从 Suspended 或 Error(recoverable) 恢复到 Running）
- `AgentRegistry::destroy_agent` — 不可逆销毁，两阶段：require_confirmation=true 时返回 DestroyConfirmation（含 confirm_token）
- `AgentRegistry::confirm_destroy` — 用 token 确认销毁操作
- `AgentRegistry::send_message` — 向 Agent 进程 stdin 发送消息
- `InboxManager::push` — 投递消息到收件箱（Task/Lateral 持久化到 pending/）
- `InboxManager::pull` — 接收方拉取所有 pending 消息并自动 ack
- `InboxManager::ack` — 手动确认消息
- `InboxManager::mark_dead_letter` — 将消息标记为死信

### 2.4 查询

- `Agent::is_alive` — 心跳检查（判断 Agent 是否存活）
- `Agent::is_terminal` — 判断是否处于终态（Stopped 或 Error）
- `Agent::set_state` — 更新 Agent 运行时状态
- `Agent::update_heartbeat` — 更新心跳时间戳
- `Agent::emit_transition` — 发出状态转换事件
- `AgentRegistry::get` — 按 ID 获取 Agent
- `AgentRegistry::get_alive` — 按 ID 获取 Agent（心跳必须在存活窗口内）
- `AgentRegistry::list` — 列出所有 Agent
- `AgentRegistry::list_alive` — 仅列出心跳存活的 Agent
- `AgentRegistry::list_by_state` — 按状态过滤列表
- `AgentRegistry::get_children` — 获取直接子 Agent
- `AgentRegistry::get_parent` — 获取父 Agent
- `AgentRegistry::get_ancestors` — 获取祖先链（不含自身）
- `AgentRegistry::get_descendants` — 递归获取所有后代（广度优先）
- `AgentRegistry::is_ancestor_of` — 判断亲缘关系
- `AgentRegistry::count` — 获取注册 Agent 总数
- `AgentRegistry::wait_timeout_secs` — 获取优雅关闭等待超时（秒）
- `AgentRegistry::grace_period_secs` — 获取优雅关闭宽限期（秒）
- `InboxManager::get_stats` — 获取通讯统计（pending/acked/dead_letter 计数 + 延迟采样）

### 2.5 状态与检查点工具

- `is_valid_transition` — 校验状态转换是否合法（Stopped 是终态；Error 除非 recoverable 否则不可恢复）
- `save_checkpoint / load_checkpoint / list_checkpoints` — 检查点持久化（Forced 挂起时执行）
- `save_pause_point / load_pause_point / list_pause_points` — 暂停点持久化（SelfRequested 挂起时执行）
- `agent_base_dir` — 计算 Agent 数据目录路径（`~/.closeclaw/agents/<agent_id>/`）

### 2.6 清理

- `AgentRegistry::remove` — 移除 Agent（杀进程 + 清理元数据）
- `AgentRegistry::kill` — 强制终止 Agent 进程
- `AgentRegistry::cleanup_dead` — 扫描心跳超时的非终态 Agent 并清理
- `InboxManager::gc` — 垃圾回收过期 acked/dead_letter 消息（按保留天数）
- `InboxManager::process_retries` — 周期性处理 pending 消息重试（检查是否超最大重试次数并标记死信）

### 2.7 进程管理

- `AgentProcess::spawn` — 启动 Agent 子进程（传入 binary_path 和 agent_id）
- `AgentProcess::spawn_with_args` — 启动带额外命令行参数的 Agent 子进程
- `AgentProcessHandle::send_message` — 通过 stdin 发送原始字符串消息
- `AgentProcessHandle::send_json` — 发送结构化 ProcessMessage（自动序列化，末尾追加 `\n`）
- `AgentProcessHandle::kill` — 终止进程
- `AgentProcessHandle::wait` — 等待进程退出并返回退出码
- `AgentProcessHandle::pid` — 获取进程 ID
- `AgentProcessHandle::agent_id` — 获取所属 Agent ID
- `AgentProcess::create_message` — 构造 JSON 消息字符串
- `AgentProcess::parse_message` — 从 JSON 字符串解析 ProcessMessage
- `spawn_output_reader` — 异步读取进程 stdout 并解析 JSON 行，通过 channel 返回
- `spawn_error_reader` — 异步读取进程 stderr 原始行，通过 channel 返回

---

## 3. 架构与结构

### 3.1 子模块划分

| 文件 | 职责 |
|------|------|
| `mod.rs` | `Agent` 核心结构体定义 |
| `state.rs` | 状态机、状态转换校验、Checkpoint/PausePoint 持久化 |
| `process.rs` | 子进程管理（spawn、IPC、stdout/stderr 异步读取） |
| `registry/` | 全局注册表（Agent 生命周期中心） |
| `registry/mod.rs` | RegistryError、AgentRegistry 结构体、构造器、SharedAgentRegistry |
| `registry/query.rs` | 只读查询方法（get/list/ancestors/descendants 等） |
| `registry/lifecycle.rs` | 基础生命周期（register/spawn/update_state/remove/kill 等） |
| `registry/cascade.rs` | 级联操作（stop/suspend/resume/destroy 等） |
| `config.rs` | AgentConfig、AgentPermissions、CommunicationConfig |
| `inbox/` | InboxManager（消息队列 + 重试 + 死信 + 统计） |
| `inbox/types.rs` | InboxConfig, MessageType, MessageStatus, InboxMessage, DeadLetterRecord, CommStats |
| `inbox/manager.rs` | InboxManager struct + impl |

### 3.2 关键数据流

**Agent 启动**：`register`（创建元数据）→ `spawn`（启动子进程，状态跃迁 Idle→Running）

**级联暂停**（Forced）：`suspend_with_descendants` → 先子后父 → Forced 在暂停前额外执行 `save_checkpoint`

**消息投递**：`InboxManager::push` → 持久化到 `pending/` → `pull` 时 ack 并移入 `acked/` → 超过 max_retry 移入 `dead_letter/`

### 3.3 进程通信格式

每条消息为一行 JSON（`\n` 分隔），通过 stdin/stdout 交换：

```json
{"type": "heartbeat" | "task" | "result" | "error", "from": "agent_id", "to": "agent_id" | null, "payload": {...}}
```

**ProcessMessage** — JSON 消息信封结构，包含 `msg_type`、`from`、`to`、`payload` 字段。
**ProcessError** — 进程管理错误（SpawnError / ProcessNotFound / ProcessAlreadyRunning / CommunicationError / UnexpectedExit）。
**RegistryError** — 注册表操作错误（AgentNotFound / AgentAlreadyExists / InvalidStateTransition / DestroyConfirmationRequired / ProcessError）。

### 3.4 持久化布局

```
~/.closeclaw/agents/<agent_id>/
├── config.json          # AgentConfig
├── permissions.json     # AgentPermissions
├── checkpoints/         # Checkpoint JSON 文件
├── pause_points/        # PausePoint JSON 文件
└── inbox/
    ├── pending/         # 待处理消息
    ├── acked/          # 已确认消息
    └── dead_letter/    # 死信记录
```

### 3.5 核心类型

- `AgentState` — 运行时状态（Idle / Running / Waiting / Suspended / Stopped / Error）
- `SuspendedReason` — 暂停原因（Forced / SelfRequested）
- `TransitionTrigger` — 状态转换触发原因（UserRequest / SystemShutdown / Error / ParentCascade / Scheduler）
- `MessageType` — 消息类型（Task 需确认且重试 / Heartbeat 不持久化不重试 / Lateral 持久化但不重试）
- `MessageStatus` — 消息状态（Pending / Acked / DeadLetter）
- `InboxMessage` — 收件箱消息（带重试状态）
- `DeadLetterRecord` — 死信持久化记录
- `CommStats` — 通讯统计快照
- `CommunicationCheckResult` — 通信检查结果（Allowed / SourceNotInTargetInbound / TargetNotInSourceOutbound）
- `MaxDepthCheckResult` — 深度检查结果（Allowed / ExceedsMaxDepth）
- `PermissionLimits` — 对 exec 类动作的细化限制（允许的命令/路径/超时）
- `ActionPermission` — 单个动作类别的权限配置
