# SPEC: Multi-Agent Agent 生命周期状态机

> Issue: [#145](https://github.com/jpxthu/CloseClaw/issues/145)

## 1. 概述

本文档定义 CloseClaw 多智能体系统中 **Agent 生命周期状态机**的扩展设计。核心目标：

1. 细化 `AgentState` 枚举，携带明确的状态语义
2. 定义状态转换事件，确保状态变更可追踪
3. 实现父子级联生命周期管理
4. 规范化各状态的资源清理规范

## 2. 现有结构

### 2.1 现有 AgentState（待扩展）

```rust
// src/agent/mod.rs
pub enum AgentState {
    Idle,       // Created, not started
    Running,    // Actively processing
    Waiting,    // Waiting for response
    Suspended,  // Paused by scheduler (旧版，无原因区分)
    Stopped,    // Completed or killed
    Error,      // Crashed with error (旧版，无错误信息)
}
```

### 2.2 现有 Agent 结构

```rust
pub struct Agent {
    pub id: String,
    pub name: String,
    pub state: AgentState,
    pub parent_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_heartbeat: DateTime<Utc>,
}
```

## 3. 扩展设计

### 3.1 新增类型

```rust
/// Agent 生命周期状态枚举
pub enum AgentState {
    /// 初始状态，Agent 创建后未启动
    Idle,
    /// 运行中，正在处理任务
    Running,
    /// 等待中，等待外部响应（如用户输入、子 Agent 结果）
    Waiting,
    /// 暂停状态（带原因）
    Suspended(SuspendedReason),
    /// 已停止，任务完成或被主动终止
    Stopped,
    /// 错误状态（带错误信息）
    Error(ErrorInfo),
}

/// 暂停原因
#[derive(Debug, Clone)]
pub enum SuspendedReason {
    /// 强制暂停：调度器因资源不足或时间片耗尽而暂停，需从检查点恢复
    Forced,
    /// 主动暂停：Agent 主动请求暂停（等待用户输入或特定事件），可从暂停点无缝继续
    Self,
}

/// 错误信息
#[derive(Debug, Clone)]
pub struct ErrorInfo {
    /// 错误描述
    pub message: String,
    /// 是否可恢复
    pub recoverable: bool,
}
```

### 3.2 状态转换事件

每个状态转换必须发出 `AgentStateTransition` 事件：

```rust
/// 状态转换触发原因
#[derive(Debug, Clone)]
pub enum TransitionTrigger {
    /// 用户主动请求
    UserRequest,
    /// 系统关闭信号
    SystemShutdown,
    /// 运行错误触发
    Error,
    /// 父 Agent 级联触发
    ParentCascade,
    /// 调度器触发
    Scheduler,
}

/// 状态转换事件
#[derive(Debug, Clone)]
pub struct AgentStateTransition {
    pub from_state: AgentState,
    pub to_state: AgentState,
    pub trigger: TransitionTrigger,
    pub timestamp: DateTime<Utc>,
}
```

## 4. 状态行为规范

### 4.1 状态进入/退出动作

| 状态 | 进入时 | 退出时 |
|------|--------|--------|
| `Idle` | 初始化资源 | — |
| `Running` | — | 记录检查点（checkpoint） |
| `Waiting` | — | — |
| `Suspended::Forced` | 发送 SIGSTOP + 记录检查点 | — |
| `Suspended::Self` | 记录暂停点（pause point） | — |
| `Stopped` | 关闭文件句柄、取消 pending 请求 | — |
| `Error` | 保留 ErrorInfo、关闭相关资源 | — |

### 4.2 检查点内容

检查点（Checkpoint）应包含：
- 当前执行位置（函数名 + 文件名 + 行号）
- 关键局部变量快照（序列化为 JSON）
- 父 Agent ID
- 创建时间戳

检查点存储路径：`~/.closeclaw/agents/<agent_id>/checkpoints/<checkpoint_id>.json`

### 4.3 暂停点内容

暂停点（Pause Point）应包含：
- 当前执行位置（函数名 + 文件名 + 行号）
- 调用栈快照
- 父 Agent ID
- 创建时间戳

暂停点存储路径：`~/.closeclaw/agents/<agent_id>/pause_points/<pause_id>.json`

## 5. 级联生命周期管理

### 5.1 级联规则

父 Agent 状态变更时，子 Agent 按以下规则响应：

| 父 → | 子 → |
|------|------|
| `Stopped` | `Stopped` |
| `Error` | `Stopped`（非 Error，正常停止） |
| `Suspended::Forced` | `Suspended::Forced` |
| `Running` | 从 `Suspended` 恢复到 `Running` |
| 父销毁 | 元数据从注册表删除，子链断裂 |

### 5.2 级联顺序

**先子后父**：所有子 Agent 完成状态转换后，再处理父 Agent。

### 5.3 级联方法签名

```rust
impl Agent {
    /// 停止当前 Agent 及所有子 Agent
    pub async fn stop_with_descendants(&self) -> Result<(), AgentError>;
    
    /// 暂停当前 Agent 及所有子 Agent（带原因）
    pub async fn suspend_with_descendants(&self, reason: SuspendedReason) -> Result<(), AgentError>;
    
    /// 恢复当前 Agent（从暂停/错误恢复）
    pub async fn resume(&self) -> Result<(), AgentError>;
}
```

## 6. 优雅关闭配置

### 6.1 配置字段

```rust
pub struct AgentConfig {
    /// 等待子 Agent 优雅关闭的超时时间（秒），默认 30s
    pub wait_timeout_secs: u64,
    /// 超时后强制 SIGTERM 等待时长（秒），默认 10s
    pub grace_period_secs: u64,
}
```

### 6.2 两阶段关闭流程

1. **优雅阶段**（`wait_timeout_secs`）：发送停止信号，等待子 Agent 自行关闭
2. **强制阶段**（`grace_period_secs`）：超时后发送 SIGTERM，仍未关闭则 SIGKILL
3. 最终：从不响应的 Agent 强制脱离父子关系

## 7. AgentRegistry 扩展

### 7.1 新增方法

```rust
impl AgentRegistry {
    /// 停止指定 Agent（可选级联）
    pub async fn stop_agent(&self, id: &str, cascade: bool) -> Result<(), AgentError>;
    
    /// 暂停指定 Agent（可选级联）
    pub async fn suspend_agent(&self, id: &str, reason: SuspendedReason, cascade: bool) -> Result<(), AgentError>;
    
    /// 恢复指定 Agent
    pub async fn resume_agent(&self, id: &str) -> Result<(), AgentError>;
    
    /// 销毁 Agent（不可逆）
    pub async fn destroy_agent(&self, id: &str, require_confirmation: bool) -> Result<DestroyConfirmation, AgentError>;
}
```

### 7.2 销毁确认

当 `require_confirmation=true` 时，`destroy_agent` 返回确认提示：

```rust
pub struct DestroyConfirmation {
    pub agent_id: String,
    pub message: String,
    pub confirm_token: String, // 用于确认操作的 token
}
```

调用方必须使用 `confirm_token` 确认销毁操作，不可绕过。

## 8. 实现计划

### Phase 1：扩展 AgentState 枚举

- [ ] 新增 `SuspendedReason` 枚举
- [ ] 新增 `ErrorInfo` 结构体
- [ ] 扩展 `AgentState` 枚举变体
- [ ] 更新现有 `Agent` 结构体

### Phase 2：实现 AgentStateTransition 事件

- [ ] 新增 `TransitionTrigger` 枚举
- [ ] 新增 `AgentStateTransition` 结构体
- [ ] 实现状态转换事件发布机制

### Phase 3：清理动作 + 检查点机制

- [ ] 定义各状态进入/退出时的清理动作
- [ ] 实现检查点（Checkpoint）数据结构与存储
- [ ] 实现暂停点（Pause Point）数据结构与存储

### Phase 4：级联操作

- [ ] 实现 `stop_with_descendants()`
- [ ] 实现 `suspend_with_descendants()`
- [ ] 实现 `resume()`
- [ ] 实现级联顺序控制（先子后父）

### Phase 5：优雅关闭配置

- [ ] 新增 `wait_timeout_secs` 和 `grace_period_secs` 配置
- [ ] 实现两阶段关闭流程
- [ ] 更新 AgentConfig 默认值

### Phase 6：集成到 AgentRegistry

- [ ] 新增 `stop_agent(cascade)` 方法
- [ ] 新增 `suspend_agent(reason, cascade)` 方法
- [ ] 新增 `resume_agent()` 方法
- [ ] 新增 `destroy_agent(require_confirmation)` 方法

## 9. 依赖

- 无外部依赖

## 10. 验收标准

1. `Suspended` 细分为 `Forced` 和 `Self` 两种，代码中有明确区分
2. `Error` 携带 `ErrorInfo`，包含 `message` 和 `recoverable` 字段
3. 父 Agent 变为 `Stopped`/`Error`/`Suspended` 时，子 Agent 按规则级联响应
4. 销毁操作返回确认提示（`require_confirmation=true` 时），不可绕过
5. 各状态转换发出 `AgentStateTransition` 事件，包含 from/to/trigger/timestamp
6. 检查点存储在 `~/.closeclaw/agents/<id>/checkpoints/` 目录
7. 优雅关闭遵循 wait_timeout → grace_period → SIGKILL 流程

---

*本文档由 EDA 生成，对应 Issue #145*
