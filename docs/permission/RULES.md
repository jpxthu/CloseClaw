# 权限规则参考

## 概述

权限规则定义在 JSON 文件中（通常为 `permissions.json`），在 `PermissionEngine` 启动时加载。引擎按照规则在文件中出现的顺序对每个操作进行评估。

## 文件格式

```json
{
  "version": "1.0",
  "defaults": {
    "file": "deny",
    "command": "deny",
    "network": "deny",
    "inter_agent": "deny",
    "config": "deny"
  },
  "rules": [
    { /* 规则对象 */ }
  ]
}
```

## 顶层字段

| 字段 | 类型 | 必填 | 描述 |
|---|---|---|---|
| `version` | `string` | 是 | 规则文件格式版本。当前为 `1.0`。 |
| `rules` | `array` | 是 | 规则对象列表（可以为空）。 |
| `defaults` | `object` | 否 | 没有规则匹配时各类操作的默认效果。默认为全部 `deny`。 |

## `defaults`

每个键是一个操作类别。有效值为 `"allow"` 和 `"deny"`。

| 类别 | 应用于 |
|---|---|
| `file` | `PermissionRequest::FileOp` |
| `command` | `PermissionRequest::CommandExec` |
| `network` | `PermissionRequest::NetOp` |
| `inter_agent` | `PermissionRequest::InterAgentMsg` |
| `config` | `PermissionRequest::ConfigWrite` |

**示例** — 默认允许所有网络请求，拒绝其他：

```json
{
  "defaults": {
    "file": "deny",
    "command": "deny",
    "network": "allow",
    "inter_agent": "deny",
    "config": "deny"
  }
}
```

## 规则对象

```json
{
  "name": "rule-unique-name",
  "subject": {
    "agent": "agent-id-or-glob",
    "match": "exact"
  },
  "effect": "allow",
  "actions": [
    { /* 操作对象 */ }
  ]
}
```

### 字段

| 字段 | 类型 | 必填 | 描述 |
|---|---|---|---|
| `name` | `string` | 是 | 唯一规则名称。用于拒绝消息中。 |
| `subject` | `object` | 是 | 定义此规则适用于哪些 agents。 |
| `effect` | `string` | 是 | `"allow"` 或 `"deny"`。 |
| `actions` | `array` | 是 | 此规则覆盖的操作对象列表。 |

### `subject`

```json
{
  "agent": "dev-agent-*",
  "match": "glob"
}
```

| 字段 | 类型 | 默认值 | 描述 |
|---|---|---|---|
| `agent` | `string` | — | Agent 标识符或 glob 模式。 |
| `match` | `string` | `"exact"` | `"exact"`（精确匹配）或 `"glob"`（glob 匹配）。Glob 支持 `*`（单路径段）和 `**`（递归）。 |

### Glob 模式

| 模式 | 匹配 | 不匹配 |
|---|---|---|
| `dev-agent-01` | `dev-agent-01` | `dev-agent-02` |
| `dev-agent-*` | `dev-agent-01`, `dev-agent-42` | `dev-agent` |
| `**` | 任意内容 | — |
| `/home/admin/**` | `/home/admin/code/main.rs` | `/home/other/file` |

## 操作类型

### `file` — 文件操作

```json
{
  "type": "file",
  "operation": "read",
  "paths": ["/home/admin/code/**"]
}
```

| 字段 | 类型 | 必填 | 描述 |
|---|---|---|---|
| `type` | string | 是 | 必须为 `"file"`。 |
| `operation` | `string` | 是 | 之一：`read`、`write`、`list`、`delete`、`execute`。 |
| `paths` | `array<string>` | 否 | 允许路径的 glob 模式列表。空表示所有路径（在允许范围内）。 |

### `command` — Shell 命令

允许特定参数的示例：

```json
{
  "type": "command",
  "command": "git",
  "args": {
    "allowed": ["status", "log", "diff", "--*"]
  }
}
```

阻止特定参数的示例：

```json
{
  "type": "command",
  "command": "rm",
  "args": {
    "blocked": ["-rf", "--no-preserve-root"]
  }
}
```

**`args` 变体：**

| 变体 | 含义 |
|---|---|
| `{}` 或省略 | 允许任意参数 |
| `{"allowed": ["a", "b"]}` | 仅允许这些参数（及其 glob 后缀） |
| `{"blocked": ["x", "y"]}` | 这些参数被拒绝；其他允许 |

### `network` — 网络连接

```json
{
  "type": "network",
  "hosts": ["*.internal.corp", "localhost"],
  "ports": [80, 443, 8000]
}
```

| 字段 | 类型 | 必填 | 描述 |
|---|---|---|---|
| `hosts` | `array<string>` | 否 | 允许的主机 glob 模式列表。空 = 允许所有。 |
| `ports` | `array<number>` | 否 | 允许的端口号列表。空 = 允许所有。 |

### `tool_call` — Skill / 工具调用

```json
{
  "type": "tool_call",
  "skill": "code-editor",
  "methods": ["read_file", "write_file"]
}
```

| 字段 | 类型 | 必填 | 描述 |
|---|---|---|---|
| `skill` | `string` | 是 | Skill 标识符。 |
| `methods` | `array<string>` | 否 | 允许的方法名列表。空 = 允许所有方法。 |

### `inter_agent` — 跨 Agent 消息

```json
{
  "type": "inter_agent",
  "agents": ["admin-agent", "monitor-agent"]
}
```

| 字段 | 类型 | 必填 | 描述 |
|---|---|---|---|
| `agents` | `array<string>` | 否 | 允许的目标 Agent ID 列表（glob 模式）。空 = 允许所有。 |

### `config_write` — 配置文件修改

```json
{
  "type": "config_write",
  "files": ["/home/admin/.closeclaw/config.json"]
}
```

| 字段 | 类型 | 必填 | 描述 |
|---|---|---|---|
| `files` | `array<string>` | 否 | 允许的文件路径（glob 模式）。空 = 允许所有。 |

## 完整示例

```json
{
  "version": "1.0",
  "defaults": {
    "file": "deny",
    "command": "deny",
    "network": "deny",
    "inter_agent": "deny",
    "config": "deny"
  },
  "rules": [
    {
      "name": "dev-agents-read-code",
      "subject": {
        "agent": "dev-*",
        "match": "glob"
      },
      "effect": "allow",
      "actions": [
        {
          "type": "file",
          "operation": "read",
          "paths": ["/home/admin/code/**"]
        }
      ]
    },
    {
      "name": "dev-agents-run-git",
      "subject": {
        "agent": "dev-*",
        "match": "glob"
      },
      "effect": "allow",
      "actions": [
        {
          "type": "command",
          "command": "git",
          "args": {}
        }
      ]
    },
    {
      "name": "admin-full-network",
      "subject": {
        "agent": "admin-agent",
        "match": "exact"
      },
      "effect": "allow",
      "actions": [
        {
          "type": "network",
          "hosts": ["*"],
          "ports": []
        }
      ]
    },
    {
      "name": "monitor-agent-communicate",
      "subject": {
        "agent": "monitor-agent",
        "match": "exact"
      },
      "effect": "allow",
      "actions": [
        {
          "type": "inter_agent",
          "agents": []
        }
      ]
    }
  ]
}
```

## 规则评估算法

1. 从传入的 `PermissionRequest` 中提取 **Agent ID**。
2. 通过预建的 HashMap 索引在 O(1) 时间内查找该 Agent 的规则索引。
3. 如果没有精确匹配，回退到扫描所有规则进行 glob 模式匹配。
4. 如果仍然没有匹配，应用请求类别的 **默认规则**。
5. 在匹配的规则中，第一个 **deny** 获胜（AWS IAM 风格）。
6. 如果只找到 allow，则操作被允许并返回一个短生命周期 token。

## 加载规则集

```rust
use closeclaw::permission::{PermissionEngine, RuleSet};

let json = std::fs::read_to_string("permissions.json")?;
let rules: RuleSet = serde_json::from_str(&json)?;
let engine = PermissionEngine::new(rules);
```

## 热重载

无需重启引擎进程即可重载规则：

```rust
let json = std::fs::read_to_string("permissions.json")?;
let rules: RuleSet = serde_json::from_str(&json)?;
sandbox.reload_rules(rules).await?;
```
