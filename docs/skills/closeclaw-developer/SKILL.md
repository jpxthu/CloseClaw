---
name: closeclaw-developer
description: |
  在 CloseClaw 框架内进行代码开发的指南，包括 Git 操作、测试、构建和 Rust 代码规范。
---

# CloseClaw Developer

## Overview

本 skill 为在 CloseClaw 框架内进行代码开发的 agent 提供指导。CloseClaw 是一个用 Rust 编写的轻量级多 agent 执行框架，核心特点包括规则驱动的权限引擎、模块化架构和热重载配置系统。

## Quick Reference

| 意图 | 工具/命令 | 说明 |
|------|-----------|------|
| 提交代码 | GitOpsSkill / `git commit` | 需要遵循 commit message 规范 |
| 推送代码 | GitOpsSkill / `git push` | 确保权限规则允许 |
| 拉取代码 | GitOpsSkill / `git pull` | 保持本地分支最新 |
| 查看 git 状态 | GitOpsSkill / `git status` | 查看工作区变更 |
| 运行测试 | `cargo test` | 运行所有测试 |
| 构建项目 | `cargo build` | 编译 release 或 debug 版本 |
| 代码检查 | `cargo clippy` | Rust linter |
| 格式化代码 | `cargo fmt` | 自动格式化 Rust 代码 |

## Detailed Usage

### Git 操作

使用内置的 `git_ops` skill：

```json
// git status
{ "method": "status", "args": {} }

// git commit
{ "method": "commit", "args": { "message": "feat: add new skill implementation" } }

// git push
{ "method": "push", "args": {} }

// git pull
{ "method": "pull", "args": {} }

// git log
{ "method": "log", "args": {} }
```

### 运行测试

```bash
# 运行所有测试
cargo test

# 运行特定测试
cargo test test_name

# 运行带日志的测试
RUST_LOG=debug cargo test

# 运行 doctests
cargo test --doc

# 运行特定包的测试
cargo test -p closeclaw-skills

# 查看测试覆盖率
cargo tarpaulin --output-dir target/cov
```

### 构建项目

```bash
# Debug 构建
cargo build

# Release 构建（优化版）
cargo build --release

# 仅构建特定 crate
cargo build -p closeclaw-skills

# 检查代码（不生成二进制）
cargo check

# 运行 clippy linter
cargo clippy -- -D warnings
```

### 代码规范

#### Rust 编码规范

1. **使用 `?` 操作符**处理错误，避免使用 `unwrap()` / `expect()`
2. **使用 `async/await`**进行异步操作，CloseClaw 使用 Tokio 运行时
3. **遵循 clippy 建议**，确保 `cargo clippy` 无警告
4. **为所有公共 API 编写文档注释**（`///`）
5. **使用 `tracing` 进行日志记录**，而非 `println!`

#### 模块结构

```
src/
├── main.rs              # 程序入口
├── lib.rs               # 库入口，定义公开 API
├── cli/                 # CLI 工具模块
├── gateway/             # 网关模块
├── agent/               # agent 运行时
├── permission/          # 权限引擎
│   ├── mod.rs
│   ├── engine.rs        # 核心评估器
│   ├── rules/mod.rs     # 规则构建器
│   └── actions/mod.rs   # Action 构建器
├── config/              # 配置系统
├── skills/              # Skill 系统
│   ├── mod.rs           # Skill trait 定义
│   ├── registry.rs      # Skill 注册表
│   └── builtin.rs       # 内置 skills
└── im/                  # IM 适配器
```

#### 关键类型

**Skill Trait** (`src/skills/registry.rs`):

```rust
#[async_trait]
pub trait Skill: Send + Sync {
    fn manifest(&self) -> SkillManifest;
    fn methods(&self) -> Vec<&str>;
    async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError>;
}
```

**PermissionEngine** (`src/permission/engine.rs`):

```rust
pub struct PermissionEngine {
    rules: RwLock<RuleSet>,
    agent_rule_index: RwLock<HashMap<String, Vec<usize>>>,
}

impl PermissionEngine {
    pub async fn evaluate(&self, request: PermissionRequest) -> PermissionResponse;
}
```

**Agent** (`src/agent/mod.rs`):

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

### 开发流程

1. **创建分支**: `git checkout -b feature/my-feature`
2. **编写代码**: 遵循 Rust 编码规范
3. **编写测试**: 在 `#[cfg(test)]` 模块中添加单元测试
4. **运行测试**: `cargo test`
5. **代码审查**: 确保 `cargo clippy` 和 `cargo fmt` 通过
6. **提交**: `git commit -m "feat: description"`
7. **推送**: `git push -u origin feature/my-feature`

### Commit Message 规范

```
<type>: <short description>

[optional body]
```

类型（type）:
- `feat`: 新功能
- `fix`: 错误修复
- `docs`: 文档变更
- `refactor`: 重构
- `test`: 测试相关
- `perf`: 性能优化
- `chore`: 构建/工具变更

### 权限规则（开发时）

开发 agent 需要在 `configs/permissions.json` 中配置相应权限。规则示例：

```json
{
  "name": "dev-agent-file-read",
  "subject": { "agent": "dev-*", "match": "glob" },
  "effect": "allow",
  "actions": [
    { "type": "file", "operation": "read", "paths": ["src/**", "tests/**"] },
    { "type": "file", "operation": "write", "paths": ["src/**"] }
  ]
}
```

### 常见错误处理

```rust
// SkillError 变体
pub enum SkillError {
    NotFound(String),
    MethodNotFound { skill: String, method: String },
    ExecutionFailed(String),
    InvalidArgs(String),
    PermissionDenied(String),
}

// 使用 ? 操作符
async fn my_method(&self) -> Result<JsonValue, SkillError> {
    let result = some_operation()?;
    Ok(serde_json::json!({ "result": result }))
}
```

## Examples

### 实现一个新的 Skill

```rust
use async_trait::async_trait;
use crate::skills::{Skill, SkillManifest, SkillError};

pub struct MySkill;

impl MySkill {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Skill for MySkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "my_skill".to_string(),
            version: "1.0.0".to_string(),
            description: "描述 skill 功能".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["do_something", "check_status"]
    }

    async fn execute(&self, method: &str, args: Value) -> Result<Value, SkillError> {
        match method {
            "do_something" => {
                let input = args.get("input")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("input required".to_string()))?;
                // 执行逻辑
                Ok(serde_json::json!({ "output": format!("processed: {}", input) }))
            }
            _ => Err(SkillError::MethodNotFound {
                skill: "my_skill".to_string(),
                method: method.to_string(),
            })
        }
    }
}
```

### 添加单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_my_skill() {
        let skill = MySkill::new();
        let result = skill.execute("do_something", serde_json::json!({ "input": "test" })).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["output"], "processed: test");
    }
}
```

### 使用 RuleBuilder 构建权限规则

```rust
use crate::permission::rules::{RuleBuilder, RuleSetBuilder};
use crate::permission::actions::ActionBuilder;

let rule = RuleBuilder::new()
    .name("allow-read-src")
    .subject_agent("dev-agent-01")
    .allow()
    .action(ActionBuilder::file("read", vec!["src/**".to_string()]).build().unwrap())
    .build()
    .unwrap();
```
