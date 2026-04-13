# Permission 模块规格说明书

> 本文件描述 `src/permission/` 模块的精确功能说明，即"系统现在是什么"。
> 不是需求文档，不含开发步骤、issue 号、验收标准或工期估算。

---

## 一、模块概述

**职责**：为 Agent 提供操作授权服务（文件、命令、网络、工具调用、跨 Agent 通信、配置写入）。运行在独立 OS 进程中，实现安全隔离。

**定位**：`src/permission/` 是 CloseClaw 的核心安全组件，位于所有操作执行之前。

---

## 二、核心类型

### 2.1 RuleSet

解析自 `permissions.json`，包含一组规则及默认值：

```rust
pub struct RuleSet {
    pub version: String,
    pub rules: Vec<Rule>,
    pub defaults: Defaults,
    pub template_includes: Vec<String>,    // 要加载的模板名列表
    pub agent_creators: HashMap<String, String>,  // agent_id → creator_user_id
}
```

### 2.2 Rule

一条权限规则：

```rust
pub struct Rule {
    pub name: String,
    pub subject: Subject,          // 规则适用对象
    pub effect: Effect,             // Allow 或 Deny
    pub actions: Vec<Action>,      // 操作规格（与 template 互斥，二选一）
    pub template: Option<TemplateRef>,  // 模板引用（与 actions 互斥）
    pub priority: i32,             // 越大越先评估
}
```

**约束**：`actions` 和 `template` 互斥，至少填一个。

### 2.3 Subject

规则适用对象，支持两种匹配模式：

```rust
pub enum Subject {
    AgentOnly {
        agent: String,
        match_type: MatchType,  // Exact 或 Glob
    },
    UserAndAgent {
        user_id: String,
        agent: String,
        user_match: MatchType,
        agent_match: MatchType,
    },
}
```

向后兼容旧格式 `{"agent": "xxx"}` 自动解析为 `AgentOnly`。

### 2.4 Action

操作规格，支持 7 种类型：

```rust
pub enum Action {
    File { operation: String, paths: Vec<String> },
    Command { command: String, args: CommandArgs },
    Network { hosts: Vec<String>, ports: Vec<u16> },
    ToolCall { skill: String, methods: Vec<String> },
    InterAgent { agents: Vec<String> },
    ConfigWrite { files: Vec<String> },
    All,   // 匹配任意操作（用于管理员全权限规则）
}
```

### 2.5 CommandArgs

命令参数约束：

```rust
pub enum CommandArgs {
    Any,                          // 不限制参数
    Allowed { allowed: Vec<String> },   // 只有这些参数允许
    Blocked { blocked: Vec<String> },    // 这些参数拒绝
}
```

### 2.6 PermissionRequest

请求信封，支持两种格式：

```rust
pub enum PermissionRequest {
    WithCaller { caller: Caller, request: PermissionRequestBody },
    Bare(PermissionRequestBody),   // 向后兼容，无调用者元数据
}
```

### 2.7 PermissionRequestBody

实际请求内容，6 种操作类型：

```rust
pub enum PermissionRequestBody {
    FileOp { agent, path, op },
    CommandExec { agent, cmd, args },
    NetOp { agent, host, port },
    ToolCall { agent, skill, method },
    InterAgentMsg { from, to },
    ConfigWrite { agent, config_file },
}
```

### 2.8 Caller

调用者元数据：

```rust
pub struct Caller {
    pub user_id: String,      // 用户 open_id，空=系统调用
    pub agent: String,         // Agent 实例 ID（必填）
    pub creator_id: String,    // Agent 创建者 user_id（用于 creator 规则）
}
```

### 2.9 PermissionResponse

引擎响应：

```rust
pub enum PermissionResponse {
    Allowed { token: String },   // 含临时令牌
    Denied { reason: String, rule: String },
}
```

---

## 三、PermissionEngine

### 3.1 公开方法

```rust
pub struct PermissionEngine { /* 私有字段 */ }

impl PermissionEngine {
    pub fn new(rules: RuleSet) -> Self
    pub fn check(&self, agent_id: &str, action: &str) -> PermissionResponse
    pub fn evaluate(&self, request: PermissionRequest) -> PermissionResponse
    pub fn reload_rules(&mut self, rules: RuleSet)
    pub fn load_templates(&mut self, templates: HashMap<String, Template>)
}
```

- `check`：简化的粗糙权限检查，接受字符串 action 名（`"file_read"`、`"exec"` 等），内部构造 `Bare` 请求
- `evaluate`：完整评估流程，支持 `WithCaller` 包装

### 3.2 评估算法

```
Step 0: Creator 规则短路
  - 若 caller.user_id == agent_creators[agent_id]，直接 Allow

Step 1: 构建候选规则列表
  a. O(1) 用户+Agent 复合索引查找
  b. O(1) Agent-only 索引查找
  c. Glob 回退（仅在 a/b 均无结果时）

Step 2: 按 priority 降序排序

Step 3: 模板展开
  - 对每个 template 引用，替换为模板中的实际 actions

Step 4: AWS IAM 风格求值（Deny 优先）
  - 遍历所有匹配规则，遇到 Deny 立即返回
  - 无 Deny 且有匹配规则 → Allow
  - 无任何匹配 → Step 5

Step 5: 默认策略
  - 根据操作类型查 Defaults，决定 Allow 或 Deny
```

### 3.3 O(1) 索引

引擎内部维护两张索引：

- `agent_rule_index: HashMap<String, Vec<usize>>` — agent_id → 规则下标列表
- `user_agent_rule_index: HashMap<String, Vec<usize>>` — `"user_id:agent_id"` → 规则下标列表

查找时先查索引，索引无结果才做 Glob 遍历。

---

## 四、模板系统（templates.rs）

### 4.1 Template

模板是规则的可复用片段：

```rust
pub struct Template {
    pub name: String,
    pub description: String,
    pub subject: TemplateSubject,
    pub effect: Effect,
    pub actions: Vec<Action>,
    pub extends: Vec<String>,   // 单继承
}
```

### 4.2 TemplateSubject

```rust
pub enum TemplateSubject {
    Any,   // 继承者提供 subject
    Agent { agent, match_type },
    UserAndAgent { user_id, agent, user_match, agent_match },
}
```

### 4.3 模板加载

```rust
pub fn load_templates_from_dir(config_dir: &Path) -> Result<HashMap<String, Template>, TemplateLoadError>
```

- 从 `config_dir/templates/` 目录读取所有 `.json` 文件
- 解析后展开继承链（单继承，支持循环检测）
- 返回展开后的模板 Map

### 4.4 TemplateLoadError

```rust
pub enum TemplateLoadError {
    IoError(PathBuf, std::io::Error),
    ParseError(PathBuf, serde_json::Error),
    TemplateNotFound(String),
    CycleDetected(String),
}
```

---

## 五、Sandbox 模块（sandbox/mod.rs）

### 5.1 Sandbox

子进程生命周期管理器：

```rust
pub struct Sandbox {
    pub state: SandboxState,
    // ... 私有字段
}

pub enum SandboxState {
    Unstarted,
    Running,
    Crashed,
    Shutdown,
}
```

**方法**：

```rust
impl Sandbox {
    pub fn new(engine: PermissionEngine) -> Self
    pub fn with_policy(self, policy: SecurityPolicy) -> Self
    pub async fn spawn(&mut self) -> Result<(), SandboxError>
    pub async fn restart(&mut self) -> Result<(), SandboxError>
    pub async fn shutdown(&mut self) -> Result<(), SandboxError>
    pub async fn evaluate(&self, request: PermissionRequest) -> Result<PermissionResponse, SandboxError>
    pub async fn reload_rules(&self, rules: RuleSet) -> Result<(), SandboxError>
    pub fn state(&self) -> SandboxState
}
```

**生命周期**：`Unstarted` → `spawn()` → `Running` → `restart()`（可循环）→ `shutdown()` → `Shutdown`

### 5.2 IpcChannel

Unix domain socket IPC 通信：

```rust
pub struct IpcChannel { /* 私有 */ }

impl IpcChannel {
    pub fn new(path: PathBuf) -> Self
    pub async fn serve<S: HandleRequest + Send + Sync + 'static>(&self, handler: Arc<S>) -> Result<(), SandboxError>
    pub async fn call(&self, request: SandboxRequest) -> Result<SandboxResponse, SandboxError>
}
```

- 协议：长度前缀 JSON 帧（4 字节网络字节序长度头 + JSON body）
- `serve()` 在引擎进程中使用，接收并处理请求
- `call()` 在 Host 进程中使用，发送请求并等待响应

### 5.3 SandboxRequest / SandboxResponse

IPC 消息类型（完整类型定义在 `sandbox/mod.rs`）：

```rust
pub enum SandboxRequest {
    Evaluate { request: PermissionRequest },
    ReloadRules { rules: RuleSet },
    Ping,
}

pub enum SandboxResponse {
    PermissionResponse(PermissionResponse),
    RulesReloaded,
    Pong,
    Error(String),
}
```

### 5.4 SecurityPolicy

Linux 安全策略（seccomp + landlock）：

```rust
pub struct SecurityPolicy {
    pub seccomp: bool,
    pub landlock: bool,
    pub allowed_fs_paths: Vec<PathBuf>,
    pub blocked_syscalls: Vec<String>,
}

impl SecurityPolicy {
    pub fn default_restrictive() -> Self
    pub fn apply(&self) -> anyhow::Result<()>  // stub：仅打印警告，不实际启用
}
```

**注意**：`apply()` 是演示级 stub，seccomp 和 landlock 均未实际激活内核级限制。

### 5.5 SandboxError

```rust
pub enum SandboxError {
    Ipc(String),
    IpcTimeout,
    ProcessError(String),
    InvalidState,
}
```

### 5.6 入口函数

```rust
pub async fn run_engine_subprocess() -> Result<(), SandboxError>
```

引擎进程的 main 入口：加载规则和模板 → 应用 SecurityPolicy → 启动 IPC server 等待请求。

---

## 六、Builder 模式

### 6.1 ActionBuilder（actions/mod.rs）

链式构建 `Action` 实例：

```rust
ActionBuilder::file(operation, paths)
ActionBuilder::command(command)          .allowed_args(vec![]) / .blocked_args(vec![])
ActionBuilder::network()                 .with_hosts(vec![]).with_ports(vec![])
ActionBuilder::tool_call(skill)          .with_methods(vec![])
ActionBuilder::inter_agent()             .with_agents(vec![])
ActionBuilder::config_write()            .with_files(vec![])
.build() -> Option<Action>
```

### 6.2 RuleBuilder（rules/mod.rs）

链式构建 `Rule` 实例：

```rust
RuleBuilder::new()
    .name("rule-name")
    .subject_agent("agent-id") / .subject_glob("agent-*") / .subject_user_and_agent(user, agent, um, am)
    .allow() / .deny()
    .action(action) / .actions(vec![action1, action2])
    .template(TemplateRef { name, overrides })
    .priority(0)
    .build() -> Result<Rule, RuleBuilderError>
```

### 6.3 RuleSetBuilder（rules/mod.rs）

链式构建 `RuleSet` 实例：

| 方法 | 说明 |
|------|------|
| `new() -> Self` | 创建空 Builder |
| `version(s)` | 设置 RuleSet 版本 |
| `rule(r)` / `rules(vec)` | 添加规则 |
| `default_file(e)` | 文件操作默认效果 |
| `default_command(e)` | 命令执行默认效果 |
| `default_network(e)` | 网络操作默认效果 |
| `default_inter_agent(e)` | 跨 Agent 通信默认效果 |
| `default_config(e)` | 配置写入默认效果 |
| `template_include(name)` | 添加模板引用 |
| `agent_creator(agent_id, user_id)` | 添加 agent 创建者映射 |
| `build() -> Result<RuleSet, RuleSetBuilderError>` | 构建 |

> 注：`RuleSetValidationError` 是 `RuleSetBuilderError` 的别名（两个名字均可用）。

### 6.4 验证函数（rules/validation.rs）

以下为 `validation` 子模块中的公开函数：

```rust
pub fn validate_rule(rule: &Rule) -> Vec<RuleValidationError>;
pub fn validate_ruleset(ruleset: &RuleSet) -> Vec<RuleSetValidationError>;
pub fn has_deny_rules(ruleset: &RuleSet) -> bool;
pub fn has_allow_rules(ruleset: &RuleSet) -> bool;
```

- `validate_rule`：返回所有验证错误的列表（空 Vec 表示验证通过）
- `validate_ruleset`：返回所有验证错误的列表（空 Vec 表示验证通过）
- `has_deny_rules`：RuleSet 中是否存在 effect=Deny 的规则
- `has_allow_rules`：RuleSet 中是否存在 effect=Allow 的规则

#### 错误枚举完整变体

```rust
/// Rule 验证失败的具体原因
pub enum RuleValidationError {
    EmptyName,                           // 规则名为空
    EmptySubjectAgent,                   // 缺少 subject agent
    NoActions,                           // 无任何 action
    ActionsAndTemplateMutuallyExclusive,  // action 和 template 不能同时指定
}

/// RuleSet 验证失败的具体原因
pub enum RuleSetValidationError {
    EmptyVersion,                        // 版本号为空
    InvalidRule(RuleValidationError),    // 含无效规则
}
```

---

## 七、模块导出（mod.rs）

```rust
pub use engine::{
    glob_match, action_matches_request,
    Action, Caller, CommandArgs, Defaults, Effect, MatchType,
    PermissionEngine, PermissionRequest, PermissionRequestBody,
    PermissionResponse, Rule, RuleSet, Subject, TemplateRef,
};
pub use rules::{
    validation, RuleBuilder, RuleBuilderError,
    RuleSetBuilder, RuleSetValidationError,
};
```

**注意**：`Sandbox`/`SandboxState`/`SandboxError`/`SecurityPolicy`/`IpcChannel`/`SandboxRequest`/`SandboxResponse` **未**在 `mod.rs` 顶层 re-export，需通过 `closeclaw::permission::sandbox` 访问。

---

## 八、Glob 匹配

```rust
pub fn glob_match(pattern: &str, value: &str) -> bool
```

支持：
- `*` — 任意字符序列（不含 `/`）
- `**` — 任意字符序列（含 `/`）
- `?` — 单个任意字符

---

## 九、数据流（Host → Engine）

```
Host 进程                          Engine 子进程
   │                                    │
   │  Sandbox::spawn()                  │
   │  → fork + exec engine binary       │
   │  → Unix socket 连接               │
   │                                    │
   │  Sandbox::evaluate(request)        │
   │  → IpcChannel::call()              │
   │  → SandboxRequest::Evaluate ──────►│  IpcChannel::serve() 接收
   │                                   │  → PermissionEngine::evaluate()
   │◄──────────────────────────────────│  → SandboxResponse::PermissionResponse
   │  Result<PermissionResponse>       │
```

---

## 十、已知偏差（代码 vs 文档）

| 偏差 | 类型 | 说明 |
|------|------|------|
| `closeclaw::permission` re-export 不完整 | 少了 | 文档列出 `Sandbox` 等类型在顶层 re-export，代码实际未导出；需用 `closeclaw::permission::sandbox::Xxx` 访问 |
| `PermissionRequestWithCaller` 类型名不存在 | 命名差异 | 文档用此名，代码用 `PermissionRequest::WithCaller` 变体（语义等价） |
| `PermissionRequest::WithCaller` 字段名 | 冲突 | 文档写 `body: PermissionRequestBody`，代码用 `request: PermissionRequestBody` |
| `TemplateRef.version` 字段 | 少了 | 文档描述此字段，代码中不存在（代码只有 `name` + `overrides`） |
| `templates.rs` 全部类型无 API 文档 | 少了 | 所有公开类型（`Template`/`TemplateSubject`/`load_templates_from_dir` 等）零文档 |
| `ActionBuilder` 链式方法无文档 | 少了 | 全部公开方法无 API 文档 |
| `RuleBuilder`/`RuleSetBuilder` 链式方法部分无文档 | 少了 | `subject_glob`/`allow`/`deny`/`actions`/`template`/`priority` 等方法无文档 |
| `SandboxRequest`/`SandboxResponse` 无文档 | 少了 | IPC 消息类型零文档 |
| `PermissionEngine::load_templates` 无文档 | 少了 | 新增公开方法无文档 |
| `PermissionEngine::rebuild_indices_with_rules` 公开但无文档 | 少了 | 内部辅助方法公开可见，无文档 |
| `SecurityPolicy::apply()` 为 stub | 说明 | 文档如实说明是演示级，代码一致 |

---

*最后更新：2026-04-11（Round 04）*
