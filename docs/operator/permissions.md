# Permission Rules Guide / 权限规则参考

> This document describes the format of `permissions.json` loaded by `PermissionEngine` at startup.
> 本文档描述 `PermissionEngine` 启动时加载的 `permissions.json` 配置文件格式。

---

## Overview / 概述

Permissions use **deny-take-precedence** (AWS IAM style).
If any rule denies, the request is denied.

权限规则使用 **Deny 优先** 评估策略（AWS IAM 风格）。
若任意规则为 Deny，请求即被拒绝。

---

## File Format / 文件格式

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
    { /* rule object */ }
  ]
}
```

### Top-Level Fields / 顶层字段

| Field 字段 | Type 类型 | Required 必填 | Description 描述 |
|---|---|---|---|
| `version` | `string` | Yes 是 | Format version. Currently `1.0`. 格式版本。当前为 `1.0`。 |
| `rules` | `array` | Yes 是 | List of rule objects. Can be empty. 规则对象列表。可以为空。 |
| `defaults` | `object` | No 否 | Default effect when no rule matches. Defaults to all `deny`. 无规则匹配时的默认效果。默认为全部 `deny`。 |

### `defaults`

Each key is an operation category. Valid values: `"allow"` / `"deny"`.

每个键是一个操作类别。有效值：`"allow"` 和 `"deny"`。

| Category 类别 | Applies to 应用于 |
|---|---|
| `file` | `PermissionRequest::FileOp` |
| `command` | `PermissionRequest::CommandExec` |
| `network` | `PermissionRequest::NetOp` |
| `inter_agent` | `PermissionRequest::InterAgentMsg` |
| `config` | `PermissionRequest::ConfigWrite` |

**Example** — allow all network, deny everything else:
**示例** — 允许所有网络请求，拒绝其他：

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

---

## Rule Structure / 规则结构

```json
{
  "name": "rule-unique-name",
  "subject": {
    "agent": "agent-id-or-glob",
    "match": "exact"
  },
  "effect": "allow",
  "actions": [
    { /* action object */ }
  ]
}
```

| Field 字段 | Type 类型 | Required 必填 | Description 描述 |
|---|---|---|---|
| `name` | `string` | Yes 是 | Unique rule name. Used in denial messages. 唯一规则名称。用于拒绝消息。 |
| `subject` | `object` | Yes 是 | Defines which agents this rule applies to. 定义此规则适用于哪些 agents。 |
| `effect` | `string` | Yes 是 | `"allow"` or `"deny"`。 |
| `actions` | `array` | Yes 是 | List of action objects this rule covers. 此规则覆盖的操作对象列表。 |

### `subject`

```json
{
  "agent": "dev-agent-*",
  "match": "glob"
}
```

| Field 字段 | Type 类型 | Default 默认值 | Description 描述 |
|---|---|---|---|
| `agent` | `string` | — | Agent identifier or glob pattern. Agent 标识符或 glob 模式。 |
| `match` | `string` | `"exact"` | `"exact"` or `"glob"`. Exact: string equality. Glob: pattern matching. `"exact"`（精确匹配）或 `"glob"`（glob 匹配）。 |

### Subject Matching / Subject 匹配

**Exact Match / 精确匹配：**
```json
"subject": { "agent": "vibe" }
```

**Glob Pattern / Glob 模式：**
```json
"subject": { "agent": "dev-*", "match": "glob" }
```

### Glob Patterns / Glob 模式

| Pattern 模式 | Matches 匹配 | Does not match 不匹配 |
|---|---|---|
| `dev-agent-01` | `dev-agent-01` | `dev-agent-02` |
| `dev-agent-*` | `dev-agent-01`, `dev-agent-42` | `dev-agent` |
| `**` | Any content 任意内容 | — |
| `/home/admin/**` | `/home/admin/code/main.rs` | `/home/other/file` |

> ⚠️ `?` (single character) is listed in some references but **not currently supported** by the engine's glob implementation. Stick to `*` and `**`.

---

## Action Types / 操作类型

### `file` — File Operation / 文件操作

```json
{
  "type": "file",
  "operation": "read",
  "paths": ["/home/admin/code/**"]
}
```

| Field 字段 | Type 类型 | Required 必填 | Description 描述 |
|---|---|---|---|
| `type` | `string` | Yes 是 | Must be `"file"`. 必须为 `"file"`。 |
| `operation` | `string` | Yes 是 | One of: `read`, `write`, `list`, `delete`, `execute`. |
| `paths` | `array<string>` | No 否 | Glob patterns for allowed paths. Empty = all paths (within allow scope). 允许路径的 glob 模式列表。空 = 允许所有路径。 |

> ⚠️ Field is `operation` (singular, array value is the operation string). Some older docs incorrectly use `operations` (plural array). Always use `operation` + string.

### `command` — Shell Command / Shell 命令

Allow specific arguments / 允许特定参数：
```json
{
  "type": "command",
  "command": "git",
  "args": {
    "allowed": ["status", "log", "diff", "--*"]
  }
}
```

Block specific arguments / 阻止特定参数：
```json
{
  "type": "command",
  "command": "rm",
  "args": {
    "blocked": ["-rf", "--no-preserve-root"]
  }
}
```

**`args` variants / `args` 变体：**

| Variant 变体 | Meaning 含义 |
|---|---|
| `{}` or omitted 省略 | Allow all arguments 允许任意参数 |
| `{"allowed": ["a", "b"]}` | Only these arguments (and their glob suffixes) are allowed 仅允许这些参数（及其 glob 后缀） |
| `{"blocked": ["x", "y"]}` | These arguments are denied; others allowed 这些参数被拒绝；其他允许 |

### `network` — Network Connection / 网络连接

```json
{
  "type": "network",
  "hosts": ["*.internal.corp", "localhost"],
  "ports": [80, 443, 8000]
}
```

| Field 字段 | Type 类型 | Required 必填 | Description 描述 |
|---|---|---|---|
| `hosts` | `array<string>` | No 否 | Allowed host glob patterns. Empty = allow all. 允许的主机 glob 模式列表。空 = 允许所有。 |
| `ports` | `array<number>` | No 否 | Allowed port numbers. Empty = allow all. 允许的端口号列表。空 = 允许所有。 |

### `tool_call` — Skill / Tool Call / Skill/工具调用

```json
{
  "type": "tool_call",
  "skill": "code-editor",
  "methods": ["read_file", "write_file"]
}
```

| Field 字段 | Type 类型 | Required 必填 | Description 描述 |
|---|---|---|---|
| `skill` | `string` | Yes 是 | Skill identifier. Skill 标识符。 |
| `methods` | `array<string>` | No 否 | Allowed method names. Empty = allow all. 允许的方法名列表。空 = 允许所有方法。 |

> ⚠️ Some older docs use `tools` (array of tool names) instead of `skill` + `methods`. The authoritative schema uses `skill` + `methods`.

### `inter_agent` — Inter-Agent Message / 跨 Agent 消息

```json
{
  "type": "inter_agent",
  "agents": ["admin-agent", "monitor-agent"]
}
```

| Field 字段 | Type 类型 | Required 必填 | Description 描述 |
|---|---|---|---|
| `agents` | `array<string>` | No 否 | Allowed target agent IDs (glob patterns). Empty = allow all. 允许的目标 Agent ID 列表（glob 模式）。空 = 允许所有。 |

### `config_write` — Config File Modification / 配置文件修改

```json
{
  "type": "config_write",
  "files": ["/home/admin/.closeclaw/config.json"]
}
```

| Field 字段 | Type 类型 | Required 必填 | Description 描述 |
|---|---|---|---|
| `files` | `array<string>` | No 否 | Allowed file paths (glob patterns). Empty = allow all. 允许的文件路径（glob 模式）。空 = 允许所有。 |

---

## Complete Example / 完整示例

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

---

## Rule Evaluation Algorithm / 规则评估算法

1. Extract **Agent ID** from the incoming `PermissionRequest`.
2. Look up the agent's rule index in O(1) via pre-built HashMap.
3. Fall back to glob pattern scanning if no exact match.
4. If still no match, apply the **default rule** for the request category.
5. Within matched rules, the first **deny** wins (AWS IAM style).
6. If only allows are found and no deny, the operation is allowed.

1. 从传入的 `PermissionRequest` 中提取 **Agent ID**。
2. 通过预建的 HashMap 索引在 O(1) 时间内查找该 Agent 的规则索引。
3. 若无精确匹配，回退到 glob 模式扫描。
4. 若仍无匹配，应用请求类别的 **默认规则**。
5. 在匹配的规则中，第一个 **deny** 获胜（AWS IAM 风格）。
6. 若仅有 allow 且无 deny，则操作被允许。

**Rule order / 规则顺序：**
1. All matching rules are collected. 收集所有匹配的规则。
2. If **any** rule is `deny`, request is denied. 若**任意**规则为 `deny`，请求被拒绝。
3. If at least one rule is `allow` and none are `deny`, request is allowed. 若至少一个 `allow` 且无 `deny`，请求被允许。
4. If no rules match, `defaults` applies. 若无规则匹配，应用 `defaults`。

---

## Loading & Hot Reload / 加载与热重载

```rust
use closeclaw::permission::{PermissionEngine, RuleSet};

let json = std::fs::read_to_string("permissions.json")?;
let rules: RuleSet = serde_json::from_str(&json)?;
let engine = PermissionEngine::new(rules);
```

Hot reload without restarting the engine process:
无需重启引擎进程即可热重载规则：

```rust
let json = std::fs::read_to_string("permissions.json")?;
let rules: RuleSet = serde_json::from_str(&json)?;
sandbox.reload_rules(rules).await?;
```

---

## Recommended Default Rule / 推荐默认规则

```json
{
  "name": "default-deny",
  "subject": { "agent": "*" },
  "effect": "deny",
  "actions": [{ "type": "file", "operation": "write", "paths": ["**"] }]
}
```
