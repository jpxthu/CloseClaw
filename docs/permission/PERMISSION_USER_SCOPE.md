# CloseClaw Permission Engine — 用户维度权限扩展设计

> 版本：1.0  
> 状态：设计文档（待实现）  
> 目标读者：开发者、架构师、运维人员

---

## 1. 概述

### 1.1 背景与动机

当前 Permission Engine 的 `Subject` 仅支持 `agent` 单一维度，所有权限规则均按 Agent ID 匹配。这意味着拥有同一 Agent 实例的所有用户共享完全相同的权限策略。随着系统规模增长，这种设计暴露了两个核心问题：

1. **无法区分用户**：Alice 和 Bob 可能共用同一个 `dev-agent`，但他们的权限需求不同（例如 Alice 是项目 Owner，Bob 是协作者）。
2. **规则爆炸**：管理员需要为每个用户 × 每个 Agent 组合编写大量重复规则，维护成本极高。

### 1.2 设计目标

本扩展在保持与现有 Agent 维度规则完全向后兼容的前提下，引入**用户维度**的细粒度控制：

| 目标 | 说明 |
|------|------|
| **Subject 双重匹配** | `Subject` 支持 `user_id` + `agent` 双重匹配-key（AND 关系） |
| **模板系统** | 权限配置模板化，支持模板继承与组合，减少规则重复 |
| **Creator 默认完整权限** | Agent 创建者自动获得该 Agent 的完整（all-actions Allow）权限 |
| **Action 模块化扩展** | 新增 Action 类型无需修改核心评估逻辑 |
| **向后兼容** | 现有 Agent-only 规则、测试套件零改动通过 |

### 1.3 术语定义

| 术语 | 定义 |
|------|------|
| **Caller** | 发起 PermissionRequest 的实体，包含 `user_id`（消息来源用户）和 `agent`（Agent 实例 ID） |
| **Agent-only Rule** | 现有仅含 `agent` 字段的规则，兼容模式 |
| **User+Agent Rule** | 新增的含 `user_id` + `agent` 的双重匹配规则 |
| **Creator Rule** | 由系统自动生成的隐式规则，授予 Agent 创建者完整权限 |
| **Template** | 可被继承/组合的权限规则片段，存储于 `templates/` 目录 |
| **TemplateRef** | 规则中引用模板的声明，记录模板名称及覆盖参数 |

---

## 2. 数据模型

### 2.1 Subject（扩展）

```rust
// src/permission/engine.rs

/// Subject that a rule applies to.
/// 
/// Supports three matching modes:
/// - `AgentOnly`: legacy mode, matches only by `agent` field
/// - `UserAndAgent`: dual-key match, both `user_id` AND `agent` must match
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "match_mode", rename_all = "snake_case")]
pub enum Subject {
    /// Legacy agent-only matching (backward compatible).
    /// Equivalent to the old `Subject { agent, match_type }` structure.
    AgentOnly {
        agent: String,
        #[serde(default)]
        match_type: MatchType,
    },
    /// Dual-key matching: both user_id AND agent must match.
    UserAndAgent {
        user_id: String,
        agent: String,
        #[serde(default)]
        user_match: MatchType,
        #[serde(default)]
        agent_match: MatchType,
    },
}

impl Subject {
    /// Returns the agent portion for index building and lookup.
    pub fn agent_id(&self) -> &str { ... }

    /// Returns the user_id portion (empty string for AgentOnly).
    pub fn user_id(&self) -> &str { ... }

    /// Returns true if this is an AgentOnly subject.
    pub fn is_agent_only(&self) -> bool { ... }
}
```

**匹配语义：**

- `AgentOnly`：与现有行为完全一致，按 `agent` + `match_type` 匹配。
- `UserAndAgent`：**AND 关系**，即 `(user_id 匹配) AND (agent 匹配)` 才认为 Subject 匹配。
- `user_match` / `agent_match` 独立设置，支持精确匹配或 Glob 匹配。

**JSON 示例：**

```json
// AgentOnly (backward compatible with current format)
{
  "match_mode": "agent_only",
  "agent": "dev-agent-01",
  "match_type": "glob"
}

// UserAndAgent (new dual-key format)
{
  "match_mode": "user_and_agent",
  "user_id": "ou_123456",
  "agent": "dev-agent-01",
  "user_match": "exact",
  "agent_match": "glob"
}
```

**向后兼容处理：**

解析时若遇到旧的 `Subject { agent, match_type / match }` 格式（无 `match_mode` 字段），自动转换为 `Subject::AgentOnly { agent, match_type }`，确保现有规则文件无需修改即可正常工作。

### 2.2 Rule（不变，新增可选字段）

```rust
// src/permission/engine.rs

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub name: String,
    pub subject: Subject,
    pub effect: Effect,
    pub actions: Vec<Action>,
    /// Optional template reference for template composition.
    /// If present, this rule inherits from the named template and
    /// may override specific fields.
    #[serde(default)]
    pub template: Option<TemplateRef>,
    /// Optional priority for evaluation ordering.
    /// Higher number = evaluated first. Default = 0.
    #[serde(default)]
    pub priority: i32,
}
```

### 2.3 TemplateRef

```rust
// src/permission/engine.rs

/// Reference to a template, optionally with parameter overrides.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TemplateRef {
    /// Name of the template to inherit from.
    pub name: String,
    /// Optional field-level overrides.
    /// Supported override keys: "effect", "actions", "agent" (for glob expansion).
    #[serde(default)]
    pub overrides: HashMap<String, serde_json::Value>,
}
```

### 2.4 Template（新增）

```rust
// src/permission/templates.rs (new file)

/// A permission template — a named, reusable fragment of rules.
/// Templates are stored as standalone files under templates/ and
/// can be inherited/composed by actual rules.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Template {
    /// Unique template name (used in TemplateRef).
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// The base subject pattern for this template.
    /// Resolved at composition time using the calling rule's context.
    pub subject: TemplateSubject,
    /// Default effect if not overridden.
    #[serde(default)]
    pub effect: Effect,
    /// List of action specifications.
    pub actions: Vec<Action>,
    /// Templates this template extends (single inheritance only).
    #[serde(default)]
    pub extends: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TemplateSubject {
    /// Matches any caller; subject is provided by the composing rule.
    Any,
    /// Fixed agent pattern.
    Agent { agent: String, match_type: MatchType },
    /// Fixed user+agent pattern.
    UserAndAgent {
        user_id: String,
        agent: String,
        user_match: MatchType,
        agent_match: MatchType,
    },
}
```

### 2.5 PermissionRequest（改造）

新增 `caller` 字段，携带发起请求的用户信息。Agent-only 请求（现有调用方）`caller` 字段可选，引擎内部回退到仅-agent 匹配。

```rust
// src/permission/engine.rs

/// Metadata about who/what initiated a permission request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Caller {
    /// The user ID of the message source (e.g. Feishu open_id, ou_xxx).
    /// Empty if the request originates from a system/infrastructure caller.
    #[serde(default)]
    pub user_id: String,
    /// The agent instance ID (always present).
    pub agent: String,
    /// Optional: the user ID of the agent's creator (for creator-rule generation).
    #[serde(default)]
    pub creator_id: String,
}

impl PermissionRequest {
    /// Extract the agent ID from the inner request variant.
    pub fn agent_id(&self) -> &str {
        match self {
            PermissionRequest::FileOp { agent, .. } => agent,
            PermissionRequest::CommandExec { agent, .. } => agent,
            PermissionRequest::NetOp { agent, .. } => agent,
            PermissionRequest::ToolCall { agent, .. } => agent,
            PermissionRequest::InterAgentMsg { from, .. } => from,
            PermissionRequest::ConfigWrite { agent, .. } => agent,
        }
    }

    /// Returns the Caller metadata (new field, never panics — returns default if absent).
    pub fn caller(&self) -> &Caller {
        static EMPTY: OnceLock<Caller> = OnceLock::new();
        // Caller is attached as a separate field on the enum (see 6.1).
        // For backward compatibility with tests, a default empty Caller is returned
        // when no caller info is present.
        self.caller_unchecked()
    }

    fn caller_unchecked(&self) -> &Caller { ... }
}
```

> **注意**：由于现有 `PermissionRequest` 使用**外部标签枚举**（`#[serde(tag = "type")]`），在其内部直接添加字段会破坏序列化兼容性。改造方案见第 6 节。

### 2.6 RuleSet（扩展）

```rust
// src/permission/engine.rs

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleSet {
    pub version: String,
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub defaults: Defaults,
    /// Names of templates to load from the templates/ directory.
    /// Templates are loaded in order; later templates with the same name
    /// override earlier ones.
    #[serde(default)]
    pub template_includes: Vec<String>,
    /// Agent creator mapping: agent_id -> creator_user_id.
    /// Used to automatically generate creator full-access rules.
    #[serde(default)]
    pub agent_creators: HashMap<String, String>,
}
```

---

## 3. 匹配逻辑

### 3.1 双重匹配规则

`Subject::UserAndAgent` 的匹配算法：

```
matches_subject(caller: Caller, subject: Subject) -> bool

if subject.match_mode == AgentOnly:
    return match(subject.agent, caller.agent, subject.match_type)

if subject.match_mode == UserAndAgent:
    user_ok   = match(subject.user_id,   caller.user_id,   subject.user_match)
    agent_ok  = match(subject.agent,     caller.agent,     subject.agent_match)
    return user_ok AND agent_ok
```

其中 `match(pattern, value, match_type)`：
- `Exact`：字符串相等（大小写敏感）
- `Glob`：使用现有 `glob_match()` 函数（支持 `*`、`**`、`?`）

### 3.2 评估时序

```
evaluate(request: PermissionRequest)
│
├─ 1. 提取 caller = request.caller
│
├─ 2. 查找候选规则
│   │
│   ├─ a) Agent-only 规则索引 (O(1) by agent_id)
│   │
│   ├─ b) User+Agent 规则索引 (O(1) by (user_id, agent_id) 复合键)
│   │       索引键格式: "{user_id}:{agent_id}"
│   │
│   └─ c) Glob 回退扫描（仅当 a/b 均无命中时）
│       对所有 Subject 尝试匹配，结果加入候选集
│
├─ 3. 生成 Creator Rule（内存中，不持久化）
│   若 caller.user_id == agent_creators[caller.agent]:
│       构造隐式 Rule { name: "__creator__", subject: AgentOnly(agent=caller.agent),
│                       effect: Allow, actions: [Action::All] }
│       优先级最高（priority = i32::MAX）
│
├─ 4. 按 priority 降序排列候选规则
│   （priority 相等保持原顺序，即文件中出现的顺序）
│
├─ 5. 遍历规则，对每条匹配规则的 actions 执行 action_match
│   └─ 若 Deny 出现：立即返回 Denied（AWS IAM 风格）
│
└─ 6. 若无匹配规则：回退到 defaults 对应 action type 的 Effect
```

### 3.3 索引结构

```rust
// PermissionEngine struct
pub struct PermissionEngine {
    rules: RwLock<RuleSet>,

    // ---- Agent-only index (existing) ----
    // agent_id -> list of rule indices
    agent_rule_index: RwLock<HashMap<String, Vec<usize>>>,

    // ---- NEW: User+Agent dual-key index ----
    // "{user_id}:{agent_id}" -> list of rule indices
    user_agent_rule_index: RwLock<HashMap<String, Vec<usize>>>,

    // ---- NEW: Templates ----
    templates: RwLock<HashMap<String, Template>>,
}
```

**索引构建时机**：`PermissionEngine::new()` 和 `reload_rules()` 时。

**复合键格式**：`format!("{}:{}", user_id, agent_id)`。  
`user_id` 为空字符串时不写入索引（视为 Agent-only）。

### 3.4 Action 匹配

现有 `rule_matches_request()` 逻辑保持不变。新规则中 `Action::All`（见第 9 节扩展性）可匹配任意 Action 类型，用于 Creator Rule 的全权限授予。

---

## 4. 模板系统

### 4.1 目录结构

```
configs/
└── permissions/
    ├── permissions.json          # 主规则文件（引用模板）
    └── templates/
        ├── __builtins__.json     # 内置基础模板（可选）
        ├── developer.json        # 开发权限模板
        ├── readonly.json         # 只读权限模板
        └── admin.json            # 管理员权限模板
```

### 4.2 模板文件格式

**`templates/developer.json`**：

```json
{
  "name": "developer",
  "description": "Standard development permissions: read/write code, run git/cargo, no network.",
  "extends": ["__builtins__"],
  "subject": { "type": "agent", "agent": "dev-*", "match_type": "glob" },
  "effect": "allow",
  "actions": [
    { "type": "file",  "operation": "read",  "paths": ["**"] },
    { "type": "file",  "operation": "write", "paths": ["/home/admin/code/**"] },
    { "type": "command", "command": "git",   "args": { "allowed": ["status", "log", "diff", "add", "commit", "push", "pull"] } },
    { "type": "command", "command": "cargo","args": { "allowed": ["build", "test", "run", "check"] } },
    { "type": "command", "command": "rustc", "args": "any" },
    { "type": "command", "command": "rustfmt", "args": "any" }
  ]
}
```

**`templates/readonly.json`**：

```json
{
  "name": "readonly",
  "description": "Read-only access to all resources.",
  "subject": { "type": "any" },
  "effect": "allow",
  "actions": [
    { "type": "file", "operation": "read", "paths": ["**"] }
  ]
}
```

**`templates/admin.json`**：

```json
{
  "name": "admin",
  "description": "Full access, inherits developer template and adds config write.",
  "extends": ["developer"],
  "subject": { "type": "agent", "agent": "admin-*", "match_type": "glob" },
  "effect": "allow",
  "actions": [
    { "type": "config_write", "files": ["**"] }
  ]
}
```

### 4.3 模板继承与组合

**继承规则（单继承）**：

- 模板可声明 `extends: [parent_name]`，继承父模板的所有 `actions`。
- 父模板必须已加载（通过 `template_includes` 或内置模板）。
- 循环继承检测：加载时发现循环则报错。

**组合（Rule 引用 Template）**：

在 `permissions.json` 的规则中使用 `template` 字段：

```json
{
  "name": "alice-developer",
  "subject": {
    "match_mode": "user_and_agent",
    "user_id": "ou_alice123",
    "agent": "dev-agent-01"
  },
  "effect": "allow",
  "template": {
    "name": "developer",
    "overrides": {
      "actions": [
        { "type": "file", "operation": "write", "paths": ["/home/admin/code/closeclaw/**"] }
      ]
    }
  }
}
```

**覆盖（Overrides）**：

- `overrides.effect`：覆盖模板的默认 Effect。
- `overrides.actions`：完全替换模板的 actions 列表。
- `overrides.agent`：替换模板中的 agent 模式（用于基于模板创建实例级规则）。

### 4.4 模板加载逻辑

```
load_templates(config_dir: &Path) -> HashMap<String, Template>

1. 读取 configs/permissions/templates/ 目录下所有 .json 文件
2. 解析每个文件为 Template
3. 按文件名排序后依次处理（保证内置模板先加载）
4. 构建继承图，检测循环引用
5. 对每个模板，递归展开 extends（展平 actions 列表）
6. 返回展开后的模板 Map（name -> Template）
```

**模板注册到引擎**：`RuleSetBuilder::template_includes` 字段指定加载哪些模板文件。

---

## 5. 默认权限规则生成（Creator Rule）

### 5.1 规则

> **Creator Rule**：若 `Caller.user_id` 与该 Agent 的创建者 `agent_creators[agent]` 匹配，则该 Caller 自动获得该 Agent 的**全部操作**允许权限，无需任何显式规则。

### 5.2 agent_creators 配置

在 `permissions.json` 中声明：

```json
{
  "version": "1.0",
  "agent_creators": {
    "dev-agent-01": "ou_creator_john",
    "prod-agent-01": "ou_creator_jane"
  },
  "rules": [...],
  "defaults": { ... }
}
```

### 5.3 运行时生成逻辑

```
evaluate(request):
    caller = request.caller
    if !caller.creator_id.is_empty()
       AND caller.user_id == agent_creators[caller.agent]:
        // 生成隐式全权限规则
        implicit_rule = Rule {
            name: "__creator_full_access__",
            subject: Subject::AgentOnly { agent: caller.agent, match_type: Exact },
            effect: Allow,
            actions: [Action::All],
            template: None,
            priority: i32::MAX,   // 最高优先级
        }
        // 直接返回 Allowed（creator 全权限，无需后续评估）
        return Allowed { token: generate_token() }
    ...
```

**注意**：Creator Rule 的 `Action::All` 是一个新的"万能 Action"类型，用于表示"所有操作均允许"。其匹配逻辑返回 `true` for all requests。

### 5.4 与显式规则的优先级

- Creator Rule 优先级最高（显式 Deny 规则**不会**覆盖 Creator Rule），这是**有意的设计选择**——Agent 创建者不应被任何配置错误锁定。
- 若需要限制创建者权限，管理员必须在 `agent_creators` 中**不声明**该创建者关系，改为使用普通规则。

---

## 6. PermissionRequest 改造

### 6.1 改造方案

现有 `PermissionRequest` 使用外部标签枚举（`#[serde(tag = "type")]`），直接添加字段会破坏 JSON 序列化兼容性（现有消息 `{"type":"file_op", ...}` 无 `caller` 字段）。

**采用方案：包装枚举（Wrap）**，保持原有变体不变：

```rust
// src/permission/engine.rs

/// Permission request envelope — wraps the typed request with caller metadata.
/// 
/// For backward compatibility with existing callers that send bare
/// `PermissionRequest` variants, the engine also accepts bare requests
/// (without caller info) and treats them as:
///   - caller.user_id = ""
///   - caller.agent   = extracted from the request variant
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PermissionRequest {
    /// Full request with caller metadata (new format).
    WithCaller {
        caller: Caller,
        #[serde(flatten)]
        request: PermissionRequestBody,
    },
    /// Backward-compatible bare request without caller info.
    Bare(PermissionRequestBody),
}

/// The actual request body (mirrors the existing PermissionRequest variants).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionRequestBody {
    FileOp    { agent: String, path: String, op: String },
    CommandExec { agent: String, cmd: String, args: Vec<String> },
    NetOp     { agent: String, host: String, port: u16 },
    ToolCall  { agent: String, skill: String, method: String },
    InterAgentMsg { from: String, to: String },
    ConfigWrite { agent: String, config_file: String },
}
```

**序列化兼容性**：

| 发送方格式 | 引擎解析结果 |
|-----------|-------------|
| `{"type":"file_op","agent":"dev","path":"/a","op":"read"}` | `PermissionRequest::Bare(...)` — 现有所有调用方无需修改 |
| `{"caller":{"user_id":"ou_xxx","agent":"dev"},"type":"file_op",...}` | `PermissionRequest::WithCaller{...}` — 新调用方使用 |

由于 `#[serde(untagged)]`，JSON 反序列化时会优先尝试 `WithCaller`（因为它的字段更多），失败后回退到 `Bare`，**零破坏性**。

### 6.2 Caller 结构

```rust
// src/permission/engine.rs

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Caller {
    /// The user ID of the message source (e.g. Feishu open_id).
    /// Empty string means "system caller" or "backward-compatible bare request".
    #[serde(default)]
    pub user_id: String,
    /// The agent instance ID.
    pub agent: String,
    /// The user ID of the agent's creator (used for creator-rule matching).
    /// If empty, looked up from agent_creators map at evaluation time.
    #[serde(default)]
    pub creator_id: String,
}
```

### 6.3 辅助方法（添加到 PermissionRequest 枚举）

```rust
impl PermissionRequest {
    /// Returns the caller metadata, with empty defaults for Bare requests.
    pub fn caller(&self) -> Caller { ... }

    /// Returns the agent ID from the request body.
    pub fn agent_id(&self) -> &str { ... }

    /// Converts a bare request to a request with caller.
    pub fn with_caller(self, caller: Caller) -> PermissionRequest {
        match self {
            PermissionRequest::Bare(body) => PermissionRequest::WithCaller { caller, request: body },
            other @ PermissionRequest::WithCaller { .. } => other,
        }
    }
}
```

---

## 7. 评估流程改造

### 7.1 完整评估流程（伪代码）

```
async fn evaluate(&self, request: PermissionRequest) -> PermissionResponse {
    let caller   = request.caller();
    let agent_id = request.agent_id();
    let ruleset  = self.rules.read().await;

    // ---- Step 0: Creator Rule（最高优先级）----
    if let Some(creator_id) = ruleset.agent_creators.get(agent_id) {
        if caller.user_id == *creator_id {
            return PermissionResponse::Allowed { token: generate_token() };
        }
    }

    // ---- Step 1: Build candidate rule list ----
    let mut candidates: Vec<usize> = Vec::new();
    let index_key = format!("{}:{}", caller.user_id, agent_id);

    // 1a. User+Agent dual-key index lookup (O(1))
    if let Some(indices) = self.user_agent_rule_index.read().await.get(&index_key) {
        candidates.extend(indices);
    }

    // 1b. Agent-only index lookup (O(1)) + glob fallback
    let agent_candidates = self.get_agent_candidates(agent_id).await;
    candidates.extend(agent_candidates);

    // 1c. Glob fallback (only if 1a and 1b produced nothing)
    if candidates.is_empty() {
        candidates = self.glob_scan(&caller, agent_id).await;
    }

    // ---- Step 2: Sort by priority (desc) ----
    candidates.sort_by(|&a, &b| {
        ruleset.rules[b].priority.cmp(&ruleset.rules[a].priority)
    });

    // ---- Step 3: Expand templates ----
    let expanded_rules = self.expand_templates(&candidates, &ruleset).await;

    // ---- Step 4: Evaluate ----
    for rule_idx in expanded_rules {
        let rule = &ruleset.rules[rule_idx];

        // Subject match
        if !self.subject_matches(&rule.subject, &caller) {
            continue;
        }

        // Action match
        if !self.rule_actions_match(rule, &request) {
            continue;
        }

        // Deny wins
        if rule.effect == Effect::Deny {
            return PermissionResponse::Denied {
                reason: format!("action denied by rule '{}'", rule.name),
                rule: rule.name.clone(),
            };
        }

        // First Allow wins
        return PermissionResponse::Allowed { token: generate_token() };
    }

    // ---- Step 5: Default fallback ----
    self.default_deny(&request, &ruleset.defaults, "no matching rule")
}
```

### 7.2 索引构建（reload 时）

```rust
fn rebuild_indices(&self, ruleset: &RuleSet) {
    let mut agent_index: HashMap<String, Vec<usize>> = HashMap::new();
    let mut user_agent_index: HashMap<String, Vec<usize>> = HashMap::new();

    for (idx, rule) in ruleset.rules.iter().enumerate() {
        match &rule.subject {
            Subject::AgentOnly { agent, .. } => {
                agent_index.entry(agent.clone()).or_default().push(idx);
            }
            Subject::UserAndAgent { user_id, agent, .. } => {
                let key = format!("{}:{}", user_id, agent);
                user_agent_index.entry(key).or_default().push(idx);
                // Also index by agent alone for backward-compatible glob scan
                agent_index.entry(agent.clone()).or_default().push(idx);
            }
        }
    }

    *self.agent_rule_index.write() = agent_index;
    *self.user_agent_rule_index.write() = user_agent_index;
}
```

### 7.3 与现有测试的兼容性

所有现有测试使用 `PermissionRequest::FileOp { agent, ... }` 等**裸变体**，在 `PermissionRequest::Bare` 分支下工作，无需任何修改。`evaluate()` 内部对 `Bare` 请求自动以空 `user_id` 执行评估，等价于现有的 Agent-only 逻辑。

---

## 8. 文件结构

```
src/permission/
├── engine.rs          # 核心类型扩展：
│                      #   - Subject::UserAndAgent (新增变体)
│                      #   - Caller (新增)
│                      #   - PermissionRequest 包装枚举
│                      #   - PermissionEngine 索引扩展
│                      #   - 评估流程更新
│
├── templates.rs       # [NEW] 模板系统：
│                      #   - Template / TemplateSubject 结构
│                      #   - 模板加载器 (load_templates)
│                      #   - 模板继承解析 (expand_inheritance)
│                      #   - 模板组合器 (apply_template_ref)
│
├── actions.rs         # [UNCHANGED] Action builders
│                      # (Action::All 新增，见第9节)
│
├── rules.rs           # [UNCHANGED] RuleBuilder / RuleSetBuilder
│                      # (新增 subject_user_and_agent() 方法)
│
├── sandbox.rs         # [UNCHANGED] 沙箱管理
│
├── mod.rs             # 导出更新
│
└── tests/
    └── user_scope_test.rs   # [NEW] 用户维度权限测试

configs/permissions/
├── permissions.json   # 主规则文件（v2 格式）
└── templates/
    ├── __builtins__.json    # [NEW] 内置模板
    ├── developer.json       # [NEW] 示例模板
    ├── readonly.json        # [NEW] 示例模板
    └── admin.json           # [NEW] 示例模板

docs/permission/
└── PERMISSION_USER_SCOPE.md  # 本文档
```

---

## 9. 扩展性考虑

### 9.1 Action 类型扩展

新增 Action 类型只需：

1. 在 `engine.rs` 的 `Action` 枚举中添加新变体。
2. 在 `PermissionEngine::rule_matches_request()` 中添加对新变体的匹配分支。
3. 在 `Defaults` 结构中添加对应的默认 Effect 字段（`default_<new_action>`）。
4. 在 `PermissionEngine::default_deny()` 中添加对新类型的 default 处理。

**无需修改 Subject、Template 或评估流程**。

### 9.2 Action::All（全操作匹配）

```rust
// src/permission/engine.rs

/// A special action that matches ALL permission requests.
/// Used exclusively for Creator Rules to grant full access.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Action {
    // ... existing variants ...
    
    /// Matches any permission request. Used for admin/creator full-access rules.
    All,
}
```

匹配逻辑：`rule_matches_request(..., Action::All)` 对所有 request 类型返回 `true`。

### 9.3 模板继承扩展

未来可支持：

- **多继承**：`extends: ["base1", "base2"]`，actions 合并（去重）。
- **模板变量**：`{{user_id}}` 占位符在模板组合时展开。
- **模板条件**：`if: { user_id.starts_with("ou_admin") }` 条件渲染。

以上均为可选增强，当前实现保持单继承、无变量、无条件。

### 9.4 规则动态优先级

当前 `priority` 为编译时常量（`i32`）。未来可通过配置引入**运行时优先级表达式**（如 `"priority: 'user_id == creator_id ? 100 : 0'"`），需引入表达式求值器（e.g. `jsonnet` 或 `rhai`），属于高级特性，当前版本不实现。

---

## 10. 示例配置

### 10.1 完整 permissions.json（v2）

```json
{
  "version": "2.0",
  "template_includes": ["__builtins__", "developer", "readonly", "admin"],
  "agent_creators": {
    "dev-agent-01": "ou_john_creator",
    "prod-agent-01": "ou_jane_sre"
  },
  "rules": [
    {
      "name": "alice-developer",
      "subject": {
        "match_mode": "user_and_agent",
        "user_id": "ou_alice_123",
        "agent": "dev-agent-01",
        "user_match": "exact",
        "agent_match": "exact"
      },
      "effect": "allow",
      "actions": [
        { "type": "file",  "operation": "read",  "paths": ["**"] },
        { "type": "file",  "operation": "write", "paths": ["/home/admin/code/closeclaw/**"] },
        { "type": "command", "command": "git", "args": { "allowed": ["status", "log", "diff", "add", "commit", "push", "pull"] } },
        { "type": "command", "command": "cargo", "args": "any" }
      ]
    },
    {
      "name": "bob-readonly",
      "subject": {
        "match_mode": "user_and_agent",
        "user_id": "ou_bob_456",
        "agent": "dev-agent-01",
        "user_match": "exact",
        "agent_match": "exact"
      },
      "effect": "allow",
      "template": {
        "name": "readonly"
      }
    },
    {
      "name": "carol-admin",
      "subject": {
        "match_mode": "user_and_agent",
        "user_id": "ou_carol_admin",
        "agent": "prod-agent-01",
        "user_match": "exact",
        "agent_match": "glob"
      },
      "effect": "allow",
      "template": {
        "name": "admin",
        "overrides": {
          "effect": "allow"
        }
      }
    },
    {
      "name": "legacy-dev-agent-01-full",
      "subject": {
        "match_mode": "agent_only",
        "agent": "legacy-agent-01",
        "match_type": "exact"
      },
      "effect": "allow",
      "actions": [
        { "type": "file", "operation": "read",  "paths": ["**"] },
        { "type": "file", "operation": "write", "paths": ["/home/admin/code/**"] },
        { "type": "command", "command": "git",   "args": "any" },
        { "type": "command", "command": "cargo", "args": "any" },
        { "type": "network", "hosts": [], "ports": [443] }
      ]
    }
  ],
  "defaults": {
    "file":       "deny",
    "command":    "deny",
    "network":    "deny",
    "inter_agent": "deny",
    "config":     "deny"
  }
}
```

### 10.2 调用示例

**带 Caller 的请求（新格式）**：

```json
{
  "caller": {
    "user_id": "ou_alice_123",
    "agent": "dev-agent-01",
    "creator_id": ""
  },
  "type": "file_op",
  "agent": "dev-agent-01",
  "path": "/home/admin/code/closeclaw/src/main.rs",
  "op": "write"
}
```

→ 匹配 `alice-developer` 规则 → **Allowed**

**带 Caller 的只读请求**：

```json
{
  "caller": { "user_id": "ou_bob_456", "agent": "dev-agent-01" },
  "type": "file_op",
  "agent": "dev-agent-01",
  "path": "/etc/shadow",
  "op": "read"
}
```

→ 匹配 `bob-readonly` 模板（readonly）→ `/etc/shadow` 不在 `**` 覆盖范围内 → **Denied**（或走 defaults）

**遗留 Bare 请求（向后兼容）**：

```json
{
  "type": "file_op",
  "agent": "dev-agent-01",
  "path": "/home/admin/code/closeclaw/src/main.rs",
  "op": "read"
}
```

→ `caller.user_id = ""`（空），走 Agent-only 索引 → 匹配 `legacy-dev-agent-01-full` 规则中的 `agent_only` 路径 → **Allowed**

### 10.3 Creator Rule 生效示例

```json
{
  "caller": { "user_id": "ou_john_creator", "agent": "dev-agent-01" },
  "type": "command_exec",
  "agent": "dev-agent-01",
  "cmd": "rm",
  "args": ["-rf", "/"]
}
```

→ `agent_creators["dev-agent-01"] == "ou_john_creator"` 匹配 → Creator Rule 生效 → **Allowed**（无论其他规则如何配置）

---

## 11. 实现 Checklist

- [ ] 在 `engine.rs` 中实现 `Subject::UserAndAgent` 变体及 `Subject::matches(caller)` 方法
- [ ] 实现 `Caller` 结构及 `PermissionRequest` 包装枚举（`WithCaller` / `Bare`）
- [ ] 添加 `user_agent_rule_index: HashMap<String, Vec<usize>>` 到 `PermissionEngine`
- [ ] 改造 `rebuild_indices()` 支持 User+Agent 复合索引
- [ ] 在 `evaluate()` 中实现 Creator Rule 检测（Step 0）
- [ ] 实现模板系统 `templates.rs`（`Template`、`load_templates`、`expand_inheritance`）
- [ ] 实现 `apply_template_ref()`（模板组合与 overrides）
- [ ] 在 `RuleSetBuilder` 中添加 `subject_user_and_agent()` 方法
- [ ] 在 `RuleBuilder` 中添加相应方法
- [ ] 添加 `Action::All` 变体
- [ ] 更新 `rule_matches_request()` 支持 `Action::All`
- [ ] 更新 `default_deny()` 处理新增 Action 类型
- [ ] 编写 `user_scope_test.rs` 测试用例
- [ ] 验证所有现有测试套件通过（零回归）
- [ ] 更新 `docs/permission/OVERVIEW.md` 反映新增架构
