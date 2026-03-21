# Permission Engine — API Reference

## `use closeclaw::permission::{...}` — Re-exports

All public types are re-exported from `closeclaw::permission`:

```rust
use closeclaw::permission::{
    PermissionEngine, PermissionRequest, PermissionResponse,
    Rule, RuleSet, Effect, Subject, MatchType,
    Action, CommandArgs, Defaults,
    Sandbox, SandboxState, SandboxError,
    SecurityPolicy, IpcChannel,
    SandboxRequest, SandboxResponse,
};
```

---

## `PermissionEngine`

Main rule evaluation engine. All methods are async.

### `PermissionEngine::new(rules: RuleSet) -> Self`

Creates a new engine from a parsed `RuleSet`. Pre-builds the agent→rule-index for O(1) lookup.

```rust
let engine = PermissionEngine::new(rules);
```

### `engine.evaluate(request: PermissionRequest) -> PermissionResponse`

Evaluate a single permission request against the loaded ruleset.

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

The input to `evaluate()`. Each variant represents a different action category.

Variants are tagged with `"type"` in JSON:

```json
{ "type": "file_op", "agent": "...", "path": "...", "op": "..." }
```

### Variants

| Variant | Fields | Category |
|---|---|---|
| `FileOp` | `agent`, `path`, `op` | `defaults.file` |
| `CommandExec` | `agent`, `cmd`, `args` | `defaults.command` |
| `NetOp` | `agent`, `host`, `port` | `defaults.network` |
| `ToolCall` | `agent`, `skill`, `method` | `defaults.file` |
| `InterAgentMsg` | `from`, `to` | `defaults.inter_agent` |
| `ConfigWrite` | `agent`, `config_file` | `defaults.config` |

### `request.agent_id() -> &str`

Extract the agent identifier from a request.

```rust
let agent = request.agent_id();
```

---

## `PermissionResponse`

The output of `evaluate()`.

### Variants

```rust
PermissionResponse::Allowed { token: String }
// token is a short-lived, opaque string e.g. "perm_1710000000_a1b2c3d4"

PermissionResponse::Denied { reason: String, rule: String }
// reason describes why the action was denied
// rule is the name of the matching deny rule or "default"
```

---

## `RuleSet`

```rust
pub struct RuleSet {
    pub version: String,
    pub rules: Vec<Rule>,
    pub defaults: Defaults,
}
```

Deserializable from JSON:

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
}
```

---

## `Effect`

```rust
pub enum Effect {
    Deny,  // default
    Allow,
}
```

Controls whether matching rules allow or deny the action.

---

## `Subject`

```rust
pub struct Subject {
    pub agent: String,
    pub match_type: MatchType,
}
```

### `subject.matches(agent_id: &str) -> bool`

Returns `true` if the subject matches the given agent ID.

```rust
let subject = Subject {
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
    Exact,  // default
    Glob,   // supports * and ** glob patterns
}
```

Controls how the `agent` field in a subject is matched.

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
}
```

Each action type corresponds to a `PermissionRequest` variant.

---

## `CommandArgs`

```rust
pub enum CommandArgs {
    Any,                           // any arguments allowed
    Allowed { allowed: Vec<String> },   // all request args must be in this list
    Blocked { blocked: Vec<String> },   // deny if any arg is in this list
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

Applied when no rule matches an incoming request. All fields default to `Effect::Deny`.

---

## `Sandbox`

Manages the engine subprocess lifecycle.

### `Sandbox::new(ipc_path: impl Into<PathBuf>) -> Self`

Create a new sandbox at the given Unix socket path.

```rust
let sandbox = Sandbox::new("/tmp/closeclaw-engine.sock");
```

### `sandbox.with_policy(policy: SecurityPolicy) -> Self`

Chainable builder to set the security policy.

```rust
let sandbox = Sandbox::new("/tmp/engine.sock")
    .with_policy(SecurityPolicy::default_restrictive());
```

### `sandbox.spawn() -> Result<(), SandboxError>`

Spawn the engine subprocess. Blocks until the engine is responsive.

```rust
let mut sandbox = Sandbox::new("/tmp/engine.sock");
sandbox.spawn().await?;
```

### `sandbox.restart() -> Result<(), SandboxError>`

Shutdown and re-spawn the engine.

### `sandbox.shutdown() -> Result<(), SandboxError>`

Cleanly terminate the engine process.

### `sandbox.evaluate(request: PermissionRequest) -> Result<PermissionResponse, SandboxError>`

Send a permission request to the engine over IPC. Returns an error if the engine is not running or times out.

```rust
let response = sandbox.evaluate(request).await?;
```

### `sandbox.reload_rules(rules: RuleSet) -> Result<(), SandboxError>`

Hot-reload the engine's ruleset without restarting the process.

```rust
sandbox.reload_rules(new_rules).await?;
```

### `sandbox.state() -> SandboxState`

Returns the current engine state (`Unstarted`, `Running`, `Crashed`, `Shutdown`).

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

Implements `std::error::Error` via `thiserror`.

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

Security policies applied inside the engine subprocess.

### `SecurityPolicy::default_restrictive() -> Self`

Creates a policy with `seccomp = true` and `landlock = true` on Linux; `false` on other platforms.

### `policy.apply() -> anyhow::Result<()>`

Applies the policy inside the engine subprocess. Called automatically by `run_engine_subprocess`.

---

## `IpcChannel`

```rust
pub struct IpcChannel { path: PathBuf }
```

Low-level Unix socket IPC channel. Use `Sandbox` for full lifecycle management.

---

## `run_engine_subprocess(ipc_path, rules)`

Starts the engine in subprocess mode. Called automatically when the binary is started with `SANDBOX_ENGINE=1`.

```rust
// Called by main.rs when SANDBOX_ENGINE=1
run_engine_subprocess(ipc_path, rules).await?;
```
