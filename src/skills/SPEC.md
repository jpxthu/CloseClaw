# Skills 模块规格书

> 本文件描述 `src/skills/` 模块的当前实现状态，不代表设计意图或未来计划。

## 1. 模块概述

Skills 模块提供 Agent 可调用的可复用工具能力，采用插件化架构。核心类型为 `Skill` trait，所有技能均通过 `SkillRegistry` 统一注册和管理。

**子模块**：
- `registry` — Skill trait 定义 + SkillRegistry 注册中心
- `builtin` — 内置技能实现（file_ops、git_ops、search、permission_query、skill_discovery、coding_agent、skill_creator）
- `coding_agent` — AI 编码任务委托技能（stub）
- `skill_creator` — 技能创建辅助技能

---

## 2. 核心类型

### 2.1 `SkillManifest`

```rust
pub struct SkillManifest {
    pub name: String,           // 技能唯一名称，如 "file_ops"
    pub version: String,        // 版本号，如 "1.0.0"
    pub description: String,     // 一句话描述
    pub author: Option<String>,
    pub dependencies: Vec<String>, // 外部依赖（如 clawhub）
}
```

### 2.2 `SkillInput`

```rust
pub struct SkillInput {
    pub skill_name: String,   // 目标技能名
    pub method: String,       // 方法名
    pub args: serde_json::Value, // JSON 参数
    pub agent_id: String,     // 调用者 Agent ID
}
```

### 2.3 `SkillOutput`

```rust
pub struct SkillOutput {
    pub success: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}
```

### 2.4 `SkillError`

```rust
pub enum SkillError {
    NotFound(String),                        // 技能不存在
    MethodNotFound { skill: String, method: String }, // 方法不存在
    ExecutionFailed(String),                 // 执行失败
    InvalidArgs(String),                     // 参数错误
    PermissionDenied(String),               // 权限不足
}
```

---

## 3. Skill Trait

```rust
#[async_trait]
pub trait Skill: Send + Sync {
    fn manifest(&self) -> SkillManifest;
    fn methods(&self) -> Vec<&str>;
    async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError>;
}
```

所有技能必须实现 `Skill` trait，并通过 `async_trait` 宏支持异步执行。

---

## 4. SkillRegistry

```rust
pub struct SkillRegistry {
    skills: tokio::sync::RwLock<HashMap<String, Arc<dyn Skill>>>,
}
```

**公开方法**：

| 方法 | 签名 | 说明 |
|------|------|------|
| `new` | `() -> Self` | 构造空注册中心 |
| `register` | `async fn(&self, skill: Arc<dyn Skill>)` | 注册技能（异步） |
| `get` | `async fn(&self, name: &str) -> Option<Arc<dyn Skill>>` | 按名称获取技能 |
| `list` | `async fn(&self) -> Vec<String>` | 列出所有已注册技能名 |
| `contains` | `async fn(&self, name: &str) -> bool` | 检查技能是否存在 |
| `unregister` | `async fn(&self, name: &str) -> bool` | 注销技能，返回是否成功 |

**并发安全**：内部使用 `tokio::sync::RwLock`，读操作不阻塞写，写操作阻塞所有读。

---

## 5. 内置技能

### 5.0 技能结构体定义

以下为各技能的公开结构体及构造器（mod.rs 未全部 re-export，但在 `builtin.rs` 中为 `pub`）：

```rust
// builtin.rs
pub struct FileOpsSkill {
    engine: Option<Arc<PermissionEngine>>,
}
impl FileOpsSkill {
    pub fn new() -> Self                 // 无引擎，所有操作被拒绝
    pub fn with_engine(Arc<PermissionEngine>) -> Self
}

pub struct GitOpsSkill { /* 无 engine 字段 */ }
impl GitOpsSkill {
    pub fn new() -> Self
}

pub struct SearchSkill { /* stub */ }
impl SearchSkill {
    pub fn new() -> Self
}

pub struct PermissionSkill {
    engine: Option<Arc<PermissionEngine>>,
}
impl PermissionSkill {
    pub fn new() -> Self                // 无引擎，返回 allowed: null
    pub fn with_engine(Arc<PermissionEngine>) -> Self
}

pub struct SkillDiscoverySkill {
    engine: Option<Arc<PermissionEngine>>,
}
impl SkillDiscoverySkill {
    pub fn new() -> Self               // 无权限引擎
    pub fn with_engine(Arc<PermissionEngine>) -> Self
}

// 辅助聚合类型
pub struct BuiltinSkills;
impl BuiltinSkills {
    pub fn all() -> Vec<Arc<dyn Skill>>                   // 无引擎版本
    pub fn all_with_engine(Arc<PermissionEngine>) -> Vec<Arc<dyn Skill>>  // 带引擎版本
}
```

> 注：`CodingAgentSkill` 已在 §5.6 记录其结构体和 `new()` 构造器。

### 5.1 `file_ops` — 文件操作技能

**说明**：提供文件系统读写能力。可选注入 `PermissionEngine`，注入后权限检查生效。

**构造方式**：
- `FileOpsSkill::new()` — 无权限引擎，所有操作被拒绝
- `FileOpsSkill::with_engine(engine)` — 关联权限引擎

**方法**：

| 方法 | 权限动作 | 参数 | 返回 |
|------|---------|------|------|
| `read` | `file_read` | `{path, agent_id?}` | `{content}` |
| `write` | `file_write` | `{path, content, agent_id?}` | `{success}` |
| `exists` | `file_read` | `{path, agent_id?}` | `{exists}` |
| `delete` | `file_write` | `{path, agent_id?}` | `{success}` |
| `list` | `file_read` | `{path?}` | `{entries: Vec<String>}` |

**行为**：
- 无引擎时，`agent_id` 可选（向后兼容）
- 有引擎时，`agent_id` 为必填；无权限时返回 `SkillError::PermissionDenied`

### 5.2 `git_ops` — Git 操作技能

**方法**：

| 方法 | 参数 | 返回 |
|------|------|------|
| `status` | `{}` | `{output}` (porcelain format) |
| `log` | `{}` | 最近 10 条 commit log |
| `commit` | `{message}` | `{success, output, error}` |
| `push` | `{}` | `{success, output, error}` |
| `pull` | `{}` | `{success, output, error}` |

**注意**：无权限控制，任何 Agent 均可调用。

### 5.3 `search` — 搜索技能（stub）

| 方法 | 参数 | 返回 |
|------|------|------|
| `search` | `{query}` | `{query, results: [], is_stub: true, message}` |

**当前状态**：完全 stub，不执行真实搜索，返回空结果。

### 5.4 `permission_query` — 权限查询技能

**说明**：允许 Agent 查询自身权限配置。

**构造方式**：
- `PermissionSkill::new()` — 无引擎，返回 `allowed: null`
- `PermissionSkill::with_engine(engine)` — 查询引擎

**方法**：

| 方法 | 参数 | 返回 |
|------|------|------|
| `query` | `{agent_id, action}` | `{allowed, agent_id, action, reason?}` |
| `list_actions` | `{}` | `{actions: [exec, file_read, file_write, network, spawn, tool_call, config_write]}` |

### 5.5 `skill_discovery` — 技能发现技能

**说明**：允许 Agent 从 ClawHub 市场搜索、安装、列出、更新技能。依赖外部 `clawhub` CLI。

**构造方式**：
- `SkillDiscoverySkill::new()` — 无权限引擎
- `SkillDiscoverySkill::with_engine(engine)` — 关联权限引擎，`install` 方法检查 `spawn` 权限

**方法**：

| 方法 | 参数 | 返回 |
|------|------|------|
| `find` | `{query}` | `{query, output, error?}` |
| `install` | `{agent_id, skill, version?}` | `{skill, version, output, error?}` |
| `list` | `{}` | `{output, error?}` |
| `update` | `{skill?}` | `{skill, output, error?}`（无 skill 参数时更新全部） |

**注意**：`install` 需要 `spawn` 权限（通过 PermissionEngine 检查），其他方法无权限控制。

### 5.6 `coding_agent` — 编码委托技能（stub）

**说明**：封装 OpenCode/Claude Code 处理复杂编码任务。

**构造方式**：
- `CodingAgentSkill::new(model: Option<String>)` — `model` 为 `None` 时使用内置默认模型

**方法**：

| 方法 | 参数 | 返回 |
|------|------|------|
| `delegate` | `{task, language?}` | `{status: "delegated", task, language, model, message}` |
| `review` | `{code}` | `{status: "review_complete", issues: [], message}` |
| `refactor` | `{code, goal?}` | `{status: "refactored", goal, message}` |
| `test` | `{code}` | `{status: "tests_generated", test_count: 0, message}` |

**当前状态**：全部为 stub，仅返回占位响应，不真实调用编码 agent。

### 5.7 `skill_creator` — 技能创建辅助技能

**说明**：帮助 Agent 理解如何为 CloseClaw 创建新技能。

**方法**：

| 方法 | 参数 | 返回 |
|------|------|------|
| `guide` | `{}` | `{content: markdown指南, format: "markdown"}` |
| `template` | `{}` | `{template: SKILL.md模板, format: "markdown"}` |
| `validate` | `{code}` | `{valid, checks: {has_async_trait_impl, has_manifest, has_execute, has_methods}}` |

---

## 6. 内置技能注册

```rust
// 无权限引擎版本
pub fn builtin_skills() -> Vec<Arc<dyn Skill>>

// 带权限引擎版本（PermissionSkill 功能可用）
pub fn builtin_skills_with_engine(engine: Arc<PermissionEngine>) -> Vec<Arc<dyn Skill>>
```

内置 7 个技能：file_ops、git_ops、search、permission_query、skill_discovery、coding_agent、skill_creator。

---

## 7. 模块导出（mod.rs）

```rust
pub mod builtin;
pub mod coding_agent;
pub mod registry;
pub mod skill_creator;

pub use builtin::{builtin_skills, builtin_skills_with_engine};
pub use coding_agent::CodingAgentSkill;
pub use registry::{Skill, SkillError, SkillInput, SkillManifest, SkillOutput, SkillRegistry};
pub use skill_creator::SkillCreatorSkill;
```

---

## 8. 已知限制

| 限制 | 说明 |
|------|------|
| `search` 技能为 stub | 搜索功能未实现，返回空结果 |
| `coding_agent` 技能为 stub | 编码委托未实现，仅返回占位值 |
| `git_ops` 无权限控制 | 任何 Agent 均可执行 git 操作 |
| `skill_discovery.install` 检查 `spawn` 权限 | 正确，其他方法无权限控制 |
| `clawhub` CLI 依赖 | `skill_discovery` 依赖系统已安装 `clawhub` |
| `builtin_skills()` 不接受 `agent_id` | 无法针对特定 Agent 过滤可用技能 |
