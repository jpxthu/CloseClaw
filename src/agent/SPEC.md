# Agent 模块规格书

> 本文档描述 `src/agent/` 模块的精确功能说明，以代码为准。

---

## 1. 模块职责

Agent 模块负责 CloseClaw 多智能体系统的核心运行时管理，包括：

- Agent 实例的生命周期状态管理（创建、运行、暂停、恢复、停止）
- 跨进程通信（通过 stdin/stdout 的 JSON 消息协议）
- Agent 注册表（集中管理所有 Agent 实例的元数据和进程句柄）
- Agent 配置与权限管理（config.json / permissions.json）
- Agent 间消息收件箱（Inbox），支持持久化、重试、死信

---

## 2. 核心数据结构

### 2.1 Agent 状态机 (`state.rs`)

```rust
// 状态枚举
pub enum AgentState {
    Idle,                          // 初始状态，未启动
    Running,                       // 正在处理任务
    Waiting,                       // 等待外部响应
    Suspended(SuspendedReason),     // 暂停（带原因）
    Stopped,                       // 正常停止
    Error(ErrorInfo),              // 错误（带错误信息）
}

// 暂停原因
pub enum SuspendedReason {
    Forced,        // 强制暂停（调度器因资源/时间片）
    SelfRequested, // 主动暂停（等待用户输入）
}

// 错误信息
pub struct ErrorInfo {
    pub message: String,       // 错误描述
    pub recoverable: bool,    // 是否可恢复
}

// 状态转换触发原因
pub enum TransitionTrigger {
    UserRequest,
    SystemShutdown,
    Error,
    ParentCascade,
    Scheduler,
}

// 状态转换事件
pub struct AgentStateTransition {
    pub from_state: AgentState,
    pub to_state: AgentState,
    pub trigger: TransitionTrigger,
    pub timestamp: DateTime<Utc>,
}

impl ErrorInfo { pub fn new(message: String, recoverable: bool) -> Self; }
impl AgentStateTransition { pub fn new(from, to, trigger) -> Self; }
impl SourceLocation { pub fn new(function, file, line) -> Self; }

// 销毁二次确认（防止误操作）
pub struct DestroyConfirmation {
    pub agent_id: String,       // 被销毁的 Agent ID
    pub message: String,       // 人类可读确认消息
    pub confirm_token: String,  // 唯一确认令牌（用于 confirm_destroy 调用）
}
```

### 2.2 检查点和暂停点 (`state.rs`)

```rust
pub struct SourceLocation {
    pub function: String,
    pub file: String,
    pub line: u32,
}

pub struct Checkpoint {
    pub id: String,
    pub agent_id: String,
    pub location: SourceLocation,
    pub variables_json: String,        // 关键变量快照（JSON）
    pub parent_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub struct PausePoint {
    pub id: String,
    pub agent_id: String,
    pub location: SourceLocation,
    pub call_stack_json: String,      // 调用栈快照
    pub parent_id: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

检查点存储路径：`~/.closeclaw/agents/<agent_id>/checkpoints/<id>.json`  
暂停点存储路径：`~/.closeclaw/agents/<agent_id>/pause_points/<id>.json`

### 2.3 Agent 实例 (`mod.rs`)

```rust
pub struct Agent {
    pub id: String,                      // UUID
    pub name: String,                   // 人读名称
    pub state: AgentState,               // 当前状态
    pub parent_id: Option<String>,       // 父 Agent ID
    pub created_at: DateTime<Utc>,      // 创建时间
    pub last_heartbeat: DateTime<Utc>,  // 最后心跳
}
```

### 2.4 进程管理 (`process.rs`)

```rust
// 进程消息信封（JSON over stdin/stdout）
pub struct ProcessMessage {
    pub msg_type: String,           // "heartbeat" | "task" | "result" | "error"
    pub from: String,              // 发送方 Agent ID
    pub to: Option<String>,        // 接收方 Agent ID（None 表示广播）
    pub payload: serde_json::Value,
}

pub struct AgentProcessHandle {
    child: Arc<RwLock<Child>>,     // tokio Child 进程
    pid: u32,
    agent_id: String,
}
```

### 2.5 注册表 (`registry.rs`)

```rust
pub struct AgentRegistry {
    agents: RwLock<HashMap<String, Agent>>,           // ID → Agent
    processes: RwLock<HashMap<String, AgentProcessHandle>>, // ID → 进程句柄
    heartbeat_timeout_secs: i64,
    wait_timeout_secs: u64,     // 优雅关闭等待超时
    grace_period_secs: u64,     // SIGTERM 宽限期
}
```

### 2.6 配置 (`config.rs`)

```rust
// AgentConfig — 存储为 config.json
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub max_child_depth: u32,         // 默认 3
    pub created_at: DateTime<Utc>,
    pub state: AgentConfigState,       // Running | Suspended | Stopped
    pub communication: CommunicationConfig,
    pub wait_timeout_secs: Option<u64>,
    pub grace_period_secs: Option<u64>,
}

// 通讯白名单
pub struct CommunicationConfig {
    pub outbound: Vec<String>,   // 允许主动发给谁
    pub inbound: Vec<String>,     // 允许接收谁的消息
}

// AgentPermissions — 存储为 permissions.json
pub struct AgentPermissions {
    pub agent_id: String,
    pub permissions: HashMap<String, ActionPermission>,
    pub inherited_from: Option<String>,
}

// 权限限制
pub struct PermissionLimits {
    pub commands: Vec<String>,   // 允许的命令列表（serde default）
    pub paths: Vec<String>,      // 允许的路径（serde default）
    pub timeout_ms: Option<u64>, // 超时限制（毫秒）
}

// 单个 action 的权限配置
pub struct ActionPermission {
    pub allowed: bool,
    pub limits: PermissionLimits,
}

// AgentPermissions 方法
impl AgentPermissions {
    pub fn load(path: &Path) -> io::Result<Self>;  // 从 JSON 文件加载
    pub fn save(&self, path: &Path) -> io::Result<()>;              // 保存到 JSON 文件
    pub fn is_allowed(&self, action: &str) -> bool;               // 检查 action 是否允许
}

// AgentConfig 通讯方法（见下方自由函数形式）
```

### 2.7 消息收件箱 (`inbox.rs`)

```rust
pub enum MessageType {
    Task,       // 需要确认
    Heartbeat,  // 不持久化，不重试
    Lateral,    // 横向消息
}

pub enum MessageStatus {
    Pending,
    Acked,
    DeadLetter,
}

pub struct InboxMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub msg_type: MessageType,
    pub payload: serde_json::Value,
    pub status: MessageStatus,
    pub retry_count: u32,
    pub max_retry: u32,
    pub created_at: DateTime<Utc>,
    pub acked_at: Option<DateTime<Utc>>,
    pub dead_letter_at: Option<DateTime<Utc>>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

pub struct InboxConfig {
    pub poll_interval_secs: u64,       // 默认 5s
    pub max_retry: u32,               // 默认 3
    pub base_delay_ms: u64,           // 默认 1000ms（指数退避）
    pub max_delay_ms: u64,            // 默认 60000ms
    pub jitter_ms: u64,               // 默认 500ms
    pub timeout_ms: u64,              // 默认 10000ms
    pub acked_ttl_days: i64,         // 默认 7 天
    pub dead_letter_ttl_days: i64,    // 默认 30 天
    pub alert_webhook: Option<String>,
}
```

收件箱存储路径：`~/.closeclaw/agents/<agent_id>/inbox/{pending,acked,dead_letter}/`

### 2.8 InboxMessage 方法

```rust
impl InboxMessage {
    pub fn new(...) -> Self;                              // 构造函数
    pub fn calculate_next_retry(&self, config: &InboxConfig) -> Option<DateTime<Utc>>; // 指数退避计算下次重试时间，config 提供 max_retry 和 base_delay
    pub fn should_persist(&self) -> bool;                 // Task 和 Lateral 持久化，Heartbeat 不持久化
    pub fn should_retry(&self) -> bool;                  // 未达 max_retry 时返回 true
    pub fn ack(&mut self);                                // 标记为已确认，设置 acked_at
    pub fn dead_letter(&mut self, reason: String);        // 标记为死信，设置 dead_letter_at 和 last_error
}
```

### 2.9 死信和通信统计

```rust
// 死信记录（持久化到 dead_letter 目录）
pub struct DeadLetterRecord {
    pub msg_id: String,                      // 消息 ID
    pub original_msg: InboxMessage,          // 原始消息
    pub failure_reason: String,             // 失败原因
    pub last_error: Option<String>,          // 最近一次错误（来自 InboxMessage.last_error）
    pub retry_count: u32,                   // 重试次数
    pub dead_letter_at: DateTime<Utc>,     // 死信时间戳
}
impl DeadLetterRecord {
    pub fn new(msg: InboxMessage, reason: &str) -> Self;
}

// 通信统计
pub struct CommStats {
    pub agent_id: String,                   // Agent ID
    pub pending_count: u64,                 // 待确认消息数
    pub acked_count: u64,                  // 已确认消息数
    pub dead_letter_count: u64,             // 死信消息数
    pub avg_latency_ms: Option<f64>,        // 平均延迟（毫秒）
    pub max_latency_ms: Option<u64>,        // 最大延迟（毫秒）
}
impl CommStats {
    pub fn new(agent_id: String) -> Self;
}
```

---

## 3. 公开接口

### 3.1 Agent 结构体

```rust
impl Agent {
    pub fn new(name: String, parent_id: Option<String>) -> Self
    pub fn set_state(&mut self, state: AgentState)
    pub fn update_heartbeat(&mut self)
    pub fn is_alive(&self, heartbeat_timeout_secs: i64) -> bool
    pub fn is_terminal(&self) -> bool  // Stopped | Error(_) → true
    pub fn emit_transition(&self, from, to, trigger) -> AgentStateTransition
}
```

### 3.2 状态机工具函数 (`state.rs`)

```rust
pub fn is_valid_transition(from: &AgentState, to: &AgentState) -> bool
pub fn agent_base_dir(agent_id: &str) -> std::path::PathBuf
pub fn save_checkpoint(checkpoint: &Checkpoint) -> std::io::Result<std::path::PathBuf>
pub fn load_checkpoint(agent_id: &str, checkpoint_id: &str) -> std::io::Result<Checkpoint>
pub fn list_checkpoints(agent_id: &str) -> std::io::Result<Vec<Checkpoint>>
pub fn save_pause_point(pause_point: &PausePoint) -> std::io::Result<std::path::PathBuf>
pub fn load_pause_point(agent_id: &str, pause_id: &str) -> std::io::Result<PausePoint>
pub fn list_pause_points(agent_id: &str) -> std::io::Result<Vec<PausePoint>>
```

### 3.3 进程管理 (`process.rs`)

```rust
impl AgentProcess {
    pub async fn spawn(binary_path: &str, agent_id: &str) -> Result<AgentProcessHandle, ProcessError>
    pub async fn spawn_with_args(binary_path, agent_id, args) -> Result<AgentProcessHandle, ProcessError>
    pub fn create_message(from, to, msg_type, payload) -> Result<String, ProcessError>
    pub fn parse_message(raw: &str) -> Result<ProcessMessage, ProcessError>
}

impl AgentProcessHandle {
    pub async fn send_message(&mut self, message: &str) -> Result<(), ProcessError>
    pub async fn send_json(&mut self, msg: &ProcessMessage) -> Result<(), ProcessError>
    pub async fn kill(&mut self) -> Result<(), ProcessError>
    pub async fn wait(&mut self) -> Result<i32, ProcessError>
    pub fn pid(&self) -> u32
    pub fn agent_id(&self) -> &str
}

pub async fn spawn_output_reader(handle: AgentProcessHandle) -> Result<Receiver<ProcessMessage>, ProcessError>
pub async fn spawn_error_reader(handle: AgentProcessHandle) -> Result<Receiver<String>, ProcessError>
```

### 3.4 注册表 (`registry.rs`)

```rust
impl AgentRegistry {
    pub fn new(heartbeat_timeout_secs: i64) -> Self
    pub fn new_with_graceful_shutdown(heartbeat, wait_timeout, grace_period) -> Self
    pub async fn register(&self, name, parent_id) -> RegistryResult<Agent>
    pub async fn spawn(&self, name, parent_id, agent_binary_path) -> RegistryResult<Agent>
    pub async fn get(&self, id: &str) -> RegistryResult<Agent>
    pub async fn get_alive(&self, id: &str) -> RegistryResult<Agent>
    pub async fn list(&self) -> Vec<Agent>
    pub async fn list_alive(&self) -> Vec<Agent>
    pub async fn list_by_state(&self, state: AgentState) -> Vec<Agent>
    pub async fn get_children(&self, parent_id: &str) -> Vec<Agent>
    pub async fn get_parent(&self, agent_id: &str) -> Option<Agent>
    pub async fn get_ancestors(&self, agent_id: &str) -> Vec<Agent>
    pub async fn is_ancestor_of(&self, ancestor_id, descendant_id) -> bool
    pub async fn get_descendants(&self, agent_id: &str) -> Vec<Agent>
    pub async fn update_state(&self, id, new_state, trigger) -> RegistryResult<Agent>
    pub async fn update_heartbeat(&self, id: &str) -> RegistryResult<()>
    pub async fn remove(&self, id: &str) -> RegistryResult<Agent>
    pub async fn kill(&self, id: &str) -> RegistryResult<()>
    pub async fn send_message(&self, id: &str, message: &str) -> RegistryResult<()>
    pub async fn cleanup_dead(&self) -> CleanupResult
    pub async fn count(&self) -> usize
    // 级联操作
    pub async fn stop_with_descendants(&self, id: &str) -> RegistryResult<()>
    pub async fn suspend_with_descendants(&self, id: &str, reason: SuspendedReason) -> RegistryResult<()>
    pub async fn resume(&self, id: &str) -> RegistryResult<Agent>
    pub async fn save_checkpoint(&self, agent_id, location_note) -> RegistryResult<Checkpoint>
    // AgentRegistry 扩展
    pub async fn stop_agent(&self, id: &str, cascade: bool) -> RegistryResult<()>
    pub async fn suspend_agent(&self, id: &str, reason: SuspendedReason, cascade: bool) -> RegistryResult<()>
    pub async fn resume_agent(&self, id: &str) -> RegistryResult<Agent>
    pub async fn destroy_agent(&self, id: &str, require_confirmation: bool) -> RegistryResult<Option<DestroyConfirmation>>
    pub async fn confirm_destroy(&self, id: &str, confirm_token: &str) -> RegistryResult<()>
    pub fn wait_timeout_secs(&self) -> u64
    pub fn grace_period_secs(&self) -> u64
}

pub type SharedAgentRegistry = Arc<AgentRegistry>;
pub fn create_registry(heartbeat_timeout_secs: i64) -> SharedAgentRegistry
```

### 3.5 配置 (`config.rs`)

```rust
impl AgentConfig {
    pub fn load(path: &Path) -> std::io::Result<Self>
    pub fn save(&self, path: &Path) -> std::io::Result<()>
}

impl CommunicationConfig {
    pub fn default_with_parent(parent_id: Option<&str>) -> Self
    pub fn can_send_to(&self, target_id: &str) -> bool
    pub fn can_receive_from(&self, source_id: &str) -> bool
}

pub fn check_communication_allowed(source: &AgentConfig, target: &AgentConfig) -> CommunicationCheckResult
pub fn check_max_depth<F>(agent_config: &AgentConfig, get_parent: F) -> MaxDepthCheckResult
```

### 3.6 收件箱 (`inbox.rs`)

```rust
impl InboxManager {
    pub async fn new(agent_id: String, config: InboxConfig) -> std::io::Result<Self>
    pub async fn push(&self, msg: InboxMessage) -> std::io::Result<()>
    pub async fn pull(&self, recipient_id: &str) -> std::io::Result<Vec<InboxMessage>>
    pub async fn ack(&self, msg_id: &str) -> std::io::Result<bool>
    pub async fn get_stats(&self) -> CommStats
    pub async fn mark_dead_letter(&self, msg_id: &str, reason: &str) -> std::io::Result<()>
    pub async fn gc(&self) -> std::io::Result<u64>
    pub async fn process_retries(&self) -> std::io::Result<()>
    pub fn config(&self) -> &InboxConfig
}
```

---

## 4. 行为规范

### 4.1 状态转换规则

`is_valid_transition(from, to)` 定义了合法状态转换：

| from \ to | Running | Waiting | Suspended | Stopped | Error |
|-----------|---------|---------|-----------|---------|-------|
| Idle | ✅ | ❌ | ✅ | ✅ | ✅ |
| Running | — | ✅ | ✅ | ✅ | ✅ |
| Waiting | ✅ | — | ✅ | ✅ | ✅ |
| Suspended | ✅ | ❌ | — | ✅ | ❌ |
| Error(recoverable) | ✅ | ❌ | ❌ | ❌ | — |
| Error(fatal) | ❌ | ❌ | ❌ | ❌ | — |
| Stopped | ❌ | ❌ | ❌ | ✅(同状态) | ❌ |

同状态转换始终合法（no-op）。

### 4.2 级联生命周期规则

父 Agent 状态变更时，子 Agent 按以下规则响应：

| 父状态 | 子 → | 实现 |
|--------|------|------|
| Stopped | Stopped | `stop_with_descendants` |
| Error | Stopped | `stop_with_descendants` |
| Suspended::Forced | Suspended::Forced | `suspend_with_descendants`，先做 checkpoint |
| Suspended::Self | Suspended::Self | `suspend_with_descendants`，不做 checkpoint |
| Running（从 Suspended 恢复）| Running | `resume()` |

级联顺序：**先子后父**（深度优先遍历 descendants）。

### 4.3 进程通信协议

Agent 与子进程通过 stdin/stdout 交换 JSON 消息行：
- 每条消息一行，以 `\n` 分隔
- 进程可发送 `heartbeat` 类型消息维持心跳
- 主进程可通过 `send_json()` 向子进程发消息
- stderr 由 `spawn_error_reader` 异步读取

### 4.4 收件箱重试策略

- `Task` 类型消息：指数退避重试（base × 2^retry_count），上限 max_delay_ms
- `Heartbeat` 类型：不持久化、不重试
- `Lateral` 类型：持久化但不重试
- 重试次数超过 max_retry 后移入 dead_letter

### 4.5 优雅关闭两阶段

1. **等待阶段**（wait_timeout_secs，默认 30s）：发送停止信号，等待子 Agent 自行关闭
2. **强制阶段**（grace_period_secs，默认 10s）：仍未关闭则 kill

---

## 5. 模块边界

- **依赖**：tokio（异步运行时）、tracing（日志）、serde、chrono、uuid、thiserror
- **被依赖**：session 模块通过 AgentRegistry 管理会话关联的 Agent；gateway 通过 spawn/spawn_with_args 启动 Agent 进程
- **不涉及**：平台适配（im/）、LLM 调用（llm/）、权限引擎执行（permission/）

---

## 6. 目录结构

```
src/agent/
├── mod.rs        # Agent 结构体定义
├── state.rs      # 状态机、Checkpoint、PausePoint、状态转换验证
├── process.rs    # 进程管理（spawn、IPC）
├── registry.rs   # AgentRegistry（注册表 + 全生命周期 API）
├── config.rs     # AgentConfig、AgentPermissions、CommunicationConfig
└── inbox.rs      # InboxManager（消息队列 + 重试 + 死信）
```
