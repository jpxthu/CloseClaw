# Agent Module

> Manages agent lifecycle, state, and inter-agent communication.

## Components

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

## Usage
```rust
let registry = AgentRegistry::new();
registry.register(agent).await;
let agents = registry.list().await;
```
