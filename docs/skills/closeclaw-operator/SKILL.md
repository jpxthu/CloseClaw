---
name: closeclaw-operator
description: |
  CloseClaw 框架运维指南，包括 CLI 使用、agent 配置、权限规则编写和监控排障。
---

# CloseClaw Operator

## Overview

本 skill 为负责部署、配置和管理 CloseClaw 框架的 operator 提供指导。CloseClaw 是一个轻量级多 agent 执行框架，采用规则驱动的权限引擎和模块化配置系统。

## Quick Reference

| 意图 | CLI / 操作 | 配置文件 |
|------|-----------|---------|
| 启动服务 | `cargo run` / `closeclaw agent start` | - |
| 查看 agent 列表 | `closeclaw agent list` | configs/agents.json |
| 验证配置文件 | `closeclaw config validate <file>` | agents.json / permissions.json |
| 检查权限规则 | `closeclaw rule check <rule>` | configs/permissions.json |
| 查看 skill 列表 | `closeclaw skill list` | configs/skills.json |
| 热重载配置 | 发送 SIGHUP 或调用 reload API | configs/*.json |
| 查看日志 | `RUST_LOG=debug cargo run` | - |
| 备份配置 | `closeclaw config backup` | configs/backup/ |

## Detailed Usage

### CLI 命令行工具

CloseClaw 提供 `closeclaw` CLI 用于管理框架：

```bash
# 查看所有可用命令
closeclaw --help

# 启动 closeclaw 服务
closeclaw agent start

# 列出所有已配置的 agent
closeclaw agent list

# 创建新 agent
closeclaw agent create <name> --model <model> --persona <persona>

# 验证配置文件
closeclaw config validate configs/agents.json
closeclaw config validate configs/permissions.json

# 检查权限规则
closeclaw rule check "dev-agent-file-read"

# 备份当前配置
closeclaw config backup

# 查看 skill 列表
closeclaw skill list

# 安装新 skill
closeclaw skill install <skill-name>
```

### Agent 配置（agents.json）

每个 agent 在 `configs/agents.json` 中定义：

```json
{
  "version": "1.0",
  "agents": [
    {
      "name": "guide",
      "model": "minimax/MiniMax-M2.7",
      "persona": "你是 CloseClaw 的引导助手...",
      "max_iterations": 100,
      "timeout_minutes": 30
    },
    {
      "name": "dev-agent-01",
      "model": "claude-3-opus",
      "parent": "root",
      "persona": "开发助手，负责编写高质量 Rust 代码",
      "max_iterations": 500,
      "timeout_minutes": 120,
      "skills": ["file_ops", "git_ops", "rust_analyzer"]
    }
  ]
}
```

字段说明：
- `name`: agent 的唯一标识符
- `model`: 使用的 LLM 模型
- `parent`: 父 agent（用于权限继承）
- `persona`: agent 的人设描述（system prompt）
- `max_iterations`: 最大迭代次数限制
- `timeout_minutes`: 任务超时时间
- `skills`: 该 agent 可使用的 skill 列表

### 权限规则（permissions.json）

权限规则定义在 `configs/permissions.json`，采用声明式 JSON 格式：

```json
{
  "version": "1.0",
  "rules": [
    {
      "name": "dev-agent-file-read",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "file",
          "operations": ["read"],
          "paths": ["src/**", "tests/**", "docs/**"]
        }
      ]
    },
    {
      "name": "dev-agent-git",
      "subject": { "agent": "dev-agent-01" },
      "effect": "allow",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "allowed": ["status", "log", "diff", "add", "commit", "push", "pull"] }
        }
      ]
    },
    {
      "name": "dev-agent-forbidden-git-force",
      "subject": { "agent": "dev-agent-01" },
      "effect": "deny",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": { "blocked": ["reset", "rebase", "push", "--force"] }
        }
      ]
    }
  ],
  "defaults": {
    "file": "deny",
    "command": "deny",
    "network": "deny",
    "inter_agent": "deny",
    "config": "deny"
  }
}
```

#### Subject 匹配类型

```json
// 精确匹配
{ "agent": "dev-agent-01" }

// Glob 模式匹配（支持 * 和 **）
{ "agent": "dev-*", "match": "glob" }
{ "agent": "**", "match": "glob" }
```

Glob 规则：
- `*` 匹配任意字符（不含 `/`）
- `**` 匹配任意路径

#### Action 类型

| type | 用途 | 关键字段 |
|------|------|---------|
| `file` | 文件系统操作 | `operations`（read/write/delete），`paths` |
| `command` | shell 命令执行 | `command`，`args`（allowed/blocked） |
| `network` | 网络访问 | `hosts`，`ports` |
| `tool_call` | Skill 方法调用 | `skill`，`methods` |
| `inter_agent` | agent 间通信 | `agents` |
| `config_write` | 配置文件写入 | `files` |

#### Effect 优先级

**Deny 优先于 Allow**（类似 AWS IAM 风格）：
1. 任意一条规则返回 `deny` → 立即拒绝
2. 所有匹配规则都是 `allow` → 允许
3. 无匹配规则 → 使用 `defaults` 设置的默认行为

#### Defaults 默认权限

```json
{
  "defaults": {
    "file": "deny",
    "command": "deny",
    "network": "deny",
    "inter_agent": "deny",
    "config": "deny"
  }
}
```

**建议**：生产环境全部设为 `deny`，显式配置允许的权限。

### 添加新 Skill

Skill 列表在 `configs/skills.json` 中管理：

```json
{
  "version": "1.0",
  "skills": [
    {
      "name": "file_ops",
      "enabled": true,
      "config": {}
    },
    {
      "name": "git_ops",
      "enabled": true,
      "config": {}
    },
    {
      "name": "search",
      "enabled": false,
      "config": { "api_key": "required" }
    }
  ]
}
```

### 配置热重载

CloseClaw 支持运行时热重载配置，无需重启服务：

1. **SIGHUP 触发**: `kill -HUP <pid>`
2. **API 调用**: 通过管理接口触发 reload
3. **自动备份**: 重载前自动备份上一版本到 `configs/backup/`

热重载支持的文件：
- `agents.json` — agent 定义变更
- `permissions.json` — 权限规则变更
- `skills.json` — skill 启用/禁用

**不**支持热重载的文件（需要重启）：
- `gateway.json` — 网关核心配置
- `im.json` — IM adapter 配置

### 监控与日志

#### 启动日志

```bash
# Info 级别（默认）
cargo run

# Debug 级别
RUST_LOG=debug cargo run

# Trace 级别（最详细）
RUST_LOG=trace cargo run
```

#### 日志格式

CloseClaw 使用 `tracing` 库，输出格式：
```
2026-03-21T13:00:00.000Z INFO  closeclaw::agent: Agent dev-agent-01 state changed: idle -> running
2026-03-21T13:00:01.000Z DEBUG closeclaw::permission: Evaluating request: FileOp { agent: "dev-agent-01", path: "src/main.rs", op: "read" }
2026-03-21T13:00:01.001Z DEBUG closeclaw::permission: Rule matched: dev-agent-file-read (allow)
2026-03-21T13:00:01.002Z INFO  closeclaw::permission: Request allowed, token: perm_1711011600_0123456789abcdef
```

#### 关键日志关键字

| 关键字 | 含义 |
|--------|------|
| `Agent state changed` | agent 状态机转换 |
| `Evaluating request` | Permission Engine 开始评估请求 |
| `Rule matched` | 规则匹配结果 |
| `Request allowed` | 请求被允许 |
| `Request denied` | 请求被拒绝 |
| `Config reloaded` | 配置热重载完成 |
| `Backup created` | 配置备份完成 |

### 故障排查

#### Agent 无响应

1. 检查 agent 状态: `closeclaw agent list`
2. 查看心跳超时日志
3. 检查 LLM API 连通性
4. 重启 agent: `closeclaw agent restart <name>`

#### 权限被意外拒绝

1. 检查 `permissions.json` 中对应规则是否存在
2. 确认 `subject.match` 类型是否正确（exact vs glob）
3. 确认 `defaults` 设置是否符合预期
4. 使用 `closeclaw rule check` 验证规则
5. 查看 `Request denied` 日志中的 reason 和 rule 字段

#### 配置文件校验失败

```bash
# 详细错误信息
closeclaw config validate configs/agents.json --verbose

# 常见错误：
# - "version cannot be empty" — version 字段必填
# - "Agent name cannot be empty" — name 字段必填
# - "Duplicate agent name" — agent 名称重复
# - "references unknown parent" — parent 引用了不存在的 agent
```

#### 权限规则校验失败

常见错误：
- `"rule name cannot be empty"` — 规则必须有名称
- `"rule subject agent cannot be empty"` — subject.agent 必填
- `"rule must have at least one action"` — 规则至少要有一个 action

### 配置备份与回滚

```bash
# 手动备份
closeclaw config backup

# 列出可用备份
ls configs/backup/

# 回滚到指定版本
closeclaw config restore configs/backup/agents.json.bak.20260321
```

自动备份触发条件：
- 配置热重载前
- 服务启动时
- 手动触发 backup 命令

### 安全建议

1. **最小权限原则**: 所有 `defaults` 设为 `deny`，显式配置允许的权限
2. **定期审查规则**: 检查权限规则是否符合最小权限原则
3. **分离配置文件**: 敏感配置（如 API key）使用独立文件，通过 `config_write` 规则控制写入权限
4. **日志审计**: 定期审查 Permission Engine 的 `allowed` 和 `denied` 日志
5. **禁止 glob 根路径**: 避免使用 `"paths": ["**"]`，精确指定允许的路径
6. **禁止 `allow` 覆盖 `deny`**: deny 规则放在 allow 规则之前

## Examples

### 完整权限配置示例

```json
{
  "version": "1.0",
  "rules": [
    {
      "name": "guide-read-only",
      "subject": { "agent": "guide" },
      "effect": "allow",
      "actions": [
        { "type": "file", "operations": ["read"], "paths": ["src/**", "tests/**", "docs/**", "*.md", "*.rs", "*.toml"] },
        { "type": "file", "operations": ["read"], "paths": ["README*", "LICENSE*"] }
      ]
    },
    {
      "name": "guide-use-search",
      "subject": { "agent": "guide" },
      "effect": "allow",
      "actions": [
        { "type": "tool_call", "skill": "search", "methods": ["search"] }
      ]
    },
    {
      "name": "guide-deny-write",
      "subject": { "agent": "guide" },
      "effect": "deny",
      "actions": [
        { "type": "file", "operations": ["write", "delete"] }
      ]
    },
    {
      "name": "guide-deny-sensitive-files",
      "subject": { "agent": "guide" },
      "effect": "deny",
      "actions": [
        { "type": "file", "operations": ["read"], "paths": ["configs/**", ".env*", "**/secrets*", "**/api_key*", "**/*.key"] }
      ]
    },
    {
      "name": "guide-deny-command",
      "subject": { "agent": "guide" },
      "effect": "deny",
      "actions": [
        { "type": "command", "command": "**" }
      ]
    },
    {
      "name": "guide-deny-network",
      "subject": { "agent": "guide" },
      "effect": "deny",
      "actions": [
        { "type": "network" }
      ]
    }
  ],
  "defaults": {
    "file": "deny",
    "command": "deny",
    "network": "deny",
    "inter_agent": "deny",
    "config": "deny"
  }
}
```

### 创建只读报告 agent

```json
{
  "name": "readonly-reports",
  "subject": { "agent": "readonly-reports-*", "match": "glob" },
  "effect": "allow",
  "actions": [
    { "type": "file", "operations": ["read"], "paths": ["reports/**", "data/**"] },
    { "type": "tool_call", "skill": "search", "methods": ["search"] }
  ]
}
```
