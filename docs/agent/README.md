# Agent Module

> CloseClaw 多智能体系统核心模块

## 文档索引

| 文档 | 内容 |
|------|------|
| **[Multi-Agent Architecture](MULTI_AGENT_ARCHITECTURE.md)** | 层级架构、权限系统、通讯机制、经验共享 |
| [Permission System](../permission/OVERVIEW.md) | 权限引擎、规则定义、审计日志 |

## 核心概念

### Agent
每个 Agent 是独立的工作单元，拥有：
- 独立的配置目录和权限边界
- 层级关系（父 → 子 → 孙）
- 能力继承和经验共享机制

### AgentState
```rust
pub enum AgentState {
    Idle,       // Created, not started
    Running,    // Actively processing
    Waiting,    // Waiting for response
    Suspended,  // Paused by scheduler
    Stopped,    // Completed or killed
    Error,      // Crashed with error
}
```

### Agent
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

### AgentRegistry
Manages all agent lifecycles. Thread-safe using `tokio::sync::RwLock`.

```rust
pub struct AgentRegistry {
    agents: RwLock<HashMap<String, Agent>>,
}
```

### AgentProcess
Manages OS process for each agent. Communication via stdin/stdout JSON.

## 快速参考

### 创建 Agent
```rust
let agent = Agent::new("项目A助手", parent_id);
registry.register(agent).await;
```

### 查询权限
```rust
let perms = permission_engine.check(agent.id, Action::Exec("git".into())).await;
```

### 发送消息
```rust
// 消息经 CloseClaw 中央仲裁
gateway.route_message(from_agent, to_agent, content).await?;
```

## 相关模块

- **Permission Engine**: 独立的 OS 进程，规则评估 + 沙盒隔离
- **Gateway**: 消息路由、协议适配、认证限流
- **Config System**: JSON 模块分离，支持热重载
