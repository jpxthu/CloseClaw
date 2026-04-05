# 权限引擎 — API 参考

## `use closeclaw::permission::{...}` — 重新导出

所有公共类型都从 `closeclaw::permission` 重新导出：

```rust
use closeclaw::permission::{
    PermissionEngine, PermissionRequest, PermissionResponse,
    Rule, RuleSet, Effect, Subject, MatchType,
    Action, CommandArgs, Defaults,
    Sandbox, SandboxState, SandboxError,
    SecurityPolicy, IpcChannel,
    SandboxRequest, SandboxResponse,
    TemplateRef, PermissionRequestWithCaller, Caller,
};
```

---

## `PermissionEngine`

主要规则评估引擎。所有方法都是异步的。

### `PermissionEngine::new(rules: RuleSet) -> Self`

从解析后的 `RuleSet` 创建新引擎。预构建 agent→rule 索引以实现 O(1) 查找。

```rust
let engine = PermissionEngine::new(rules);
```

### `engine.evaluate(request: PermissionRequest) -> PermissionResponse`

根据加载的规则集评估单个权限请求。

```rust
let response = engine.evaluate(request).await;
match response {
    PermissionResponse::Allowed { token } => {
        println!("action allowed, token: {}", token);
    }
    PermissionResponse::Denied { reason, rule } => {
        println!("action denied by rule '{}': {}", rule, reason);
    }
}
```

---

## `PermissionRequest`

`evaluate()` 的输入。区分有无调用者元数据的两种形式：

```rust
/// 无调用者元数据的权限请求
pub enum PermissionRequest {
    FileOp { agent: String, path: String, op: String },
    CommandExec { agent: String, cmd: String, args: Vec<String> },
    NetOp { agent: String, host: String, port: u16 },
    ToolCall { agent: String, skill: String, method: String },
    InterAgentMsg { from: String, to: String },
    ConfigWrite { agent: String, config_file: String },
}

/// 带调用者元数据的权限请求（用于 inter-agent 场景）
pub struct PermissionRequestWithCaller {
    pub caller: Caller,
    pub body: PermissionRequestBody,
}

/// 调用者元数据
pub struct Caller {
    pub agent_id: String,
    pub user_id: Option<String>,
    pub rule_name: Option<String>,
}

/// 权限请求体（对应 Action::InterAgent）
pub enum PermissionRequestBody {
    InterAgentMsg { from: String, to: String },
}
```

在 JSON 中变体用 `"type"` 标记：

```json
{ "type": "file_op", "agent": "...", "path": "...", "op": "..." }
```

### 变体（PermissionRequest）

| 变体 | 字段 | 类别 |
|---|---|---|
| `FileOp` | `agent`, `path`, `op` | `defaults.file` |
| `CommandExec` | `agent`, `cmd`, `args` | `defaults.command` |
| `NetOp` | `agent`, `host`, `port` | `defaults.network` |
| `ToolCall` | `agent`, `skill`, `method` | `defaults.file` |
| `InterAgentMsg` | `from`, `to` | `defaults.inter_agent` |
| `ConfigWrite` | `agent`, `config_file` | `defaults.config` |

### `request.agent_id() -> &str`

从请求中提取 agent 标识符。

```rust
let agent = request.agent_id();
```

---

## `PermissionResponse`

`evaluate()` 的输出。

### 变体

```rust
PermissionResponse::Allowed { token: String }
// token 是一个短生命周期的 opaque 字符串，例如 "perm_1710000000_a1b2c3d4"

PermissionResponse::Denied { reason: String, rule: String }
// reason 描述操作被拒绝的原因
// rule 是匹配的拒绝规则的名称或 "default"
```

---

## `RuleSet`

```rust
pub struct RuleSet {
    pub version: String,
    pub rules: Vec<Rule>,
    pub defaults: Defaults,
    /// 模板名称列表，用于运行时模板展开
    pub template_includes: Vec<String>,
    /// Agent 创建者映射：agent_id -> creator_user_id
    pub agent_creators: HashMap<String, String>,
}
```

可以从 JSON 反序列化：

```json
{
  "version": "1.0",
  "defaults": { "file": "deny", "command": "deny", ... },
  "rules": [...]
}
```

---

## `Rule`

```rust
pub struct Rule {
    pub name: String,
    pub subject: Subject,
    pub effect: Effect,
    pub actions: Vec<Action>,
    /// 关联的权限模板引用
    pub template: Option<TemplateRef>,
    /// 规则优先级，数值越大优先级越高，匹配时取最高优先级规则
    pub priority: i32,
}
```

---

## `Effect`

```rust
pub enum Effect {
    Deny,  // 默认
    Allow,
}
```

控制匹配的规则是允许还是拒绝操作。

---

## `Subject`

权限主体，支持纯 Agent 模式和 User+Agent 联合模式。

```rust
pub enum Subject {
    /// 仅匹配 Agent（向后兼容原有行为）
    AgentOnly {
        agent: String,
        match_type: MatchType,
    },
    /// 同时匹配 User 和 Agent
    UserAndAgent {
        user_id: String,
        user_match: MatchType,
        agent_match: MatchType,
    },
}
```

### `subject.matches(agent_id: &str) -> bool`

如果该主体匹配给定的 agent ID，则返回 `true`。

```rust
let subject = Subject::AgentOnly {
    agent: "dev-*".to_string(),
    match_type: MatchType::Glob,
};
assert!(subject.matches("dev-agent-01"));
assert!(!subject.matches("prod-agent"));
```

---

## `MatchType`

```rust
pub enum MatchType {
    Exact,  // 默认
    Glob,   // 支持 * 和 ** glob 模式
}
```

控制主体中 `agent` 字段的匹配方式。

---

## `Action`

```rust
pub enum Action {
    File { operation: String, paths: Vec<String> },
    Command { command: String, args: CommandArgs },
    Network { hosts: Vec<String>, ports: Vec<u16> },
    ToolCall { skill: String, methods: Vec<String> },
    InterAgent { agents: Vec<String> },
    ConfigWrite { files: Vec<String> },
    /// 匹配所有操作类型
    All,
}
```

---

## `TemplateRef`

权限模板引用，用于将规则与可复用的权限模板关联。

```rust
pub struct TemplateRef {
    /// 模板名称，对应 RuleSet.template_includes 中的条目
    pub name: String,
    /// 可选的版本约束，如 ">=1.0.0"
    pub version: Option<String>,
}
```

每种操作类型对应一个 `PermissionRequest` 变体。

---

## `CommandArgs`

```rust
pub enum CommandArgs {
    Any,                           // 允许任意参数
    Allowed { allowed: Vec<String> },   // 所有请求参数必须在此列表中
    Blocked { blocked: Vec<String> },   // 如果任何参数在列表中则拒绝
}
```

---

## `Defaults`

```rust
pub struct Defaults {
    pub file: Effect,
    pub command: Effect,
    pub network: Effect,
    pub inter_agent: Effect,
    pub config: Effect,
}
```

当没有规则匹配传入请求时应用。默认所有字段为 `Effect::Deny`。

---

## `Sandbox`

管理引擎子进程生命周期。

### `Sandbox::new(ipc_path: impl Into<PathBuf>) -> Self`

在给定的 Unix socket 路径创建一个新的沙箱。

```rust
let sandbox = Sandbox::new("/tmp/closeclaw-engine.sock");
```

### `sandbox.with_policy(policy: SecurityPolicy) -> Self`

可链式调用的构建器，用于设置安全策略。

```rust
let sandbox = Sandbox::new("/tmp/engine.sock")
    .with_policy(SecurityPolicy::default_restrictive());
```

### `sandbox.spawn() -> Result<(), SandboxError>`

生成引擎子进程。阻塞直到引擎响应。

```rust
let mut sandbox = Sandbox::new("/tmp/engine.sock");
sandbox.spawn().await?;
```

### `sandbox.restart() -> Result<(), SandboxError>`

关闭并重新生成引擎。

### `sandbox.shutdown() -> Result<(), SandboxError>`

干净地终止引擎进程。

### `sandbox.evaluate(request: PermissionRequest) -> Result<PermissionResponse, SandboxError>`

通过 IPC 发送权限请求到引擎。如果引擎未运行或超时则返回错误。

```rust
let response = sandbox.evaluate(request).await?;
```

### `sandbox.reload_rules(rules: RuleSet) -> Result<(), SandboxError>`

无需重启进程即可热重载引擎的规则集。

```rust
sandbox.reload_rules(new_rules).await?;
```

### `sandbox.state() -> SandboxState`

返回当前引擎状态（`Unstarted`、`Running`、`Crashed`、`Shutdown`）。

---

## `SandboxState`

```rust
pub enum SandboxState {
    Unstarted,
    Running,
    Crashed { exit_code: Option<i32> },
    Shutdown,
}
```

---

## `SandboxError`

```rust
pub enum SandboxError {
    Ipc(std::io::Error),
    IpcTimeout,
    ProcessError(String),
    InvalidState { state: SandboxState },
}
```

通过 `thiserror` 实现 `std::error::Error`。

---

## `SecurityPolicy`

```rust
pub struct SecurityPolicy {
    pub seccomp: bool,
    pub landlock: bool,
    pub allowed_fs_paths: Vec<PathBuf>,
    pub blocked_syscalls: Vec<String>,
}
```

在引擎子进程中应用的安全策略。

### `SecurityPolicy::default_restrictive() -> Self`

创建一个在 Linux 上 `seccomp = true` 和 `landlock = true` 的策略；在其他平台上为 `false`。

### `policy.apply() -> anyhow::Result<()>`

在引擎子进程内部应用策略。由 `run_engine_subprocess` 自动调用。

---

## `IpcChannel`

```rust
pub struct IpcChannel { path: PathBuf }
```

低级 Unix socket IPC 通道。使用 `Sandbox` 进行完整的生命周期管理。

---

## `run_engine_subprocess(ipc_path, rules)`

以子进程模式启动引擎。当二进制文件以 `SANDBOX_ENGINE=1` 启动时自动调用。

```rust
// 当 SANDBOX_ENGINE=1 时由 main.rs 调用
run_engine_subprocess(ipc_path, rules).await?;
```
