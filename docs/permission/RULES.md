# Permission Rules Reference

## Overview

Permission rules are defined in a JSON file (conventionally `permissions.json`) and loaded by the `PermissionEngine` at startup. The engine evaluates every action against the rules in the order they appear in the file.

## File Format

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

## Top-Level Fields

| Field | Type | Required | Description |
|---|---|---|---|
| `version` | `string` | Yes | Rule file format version. Currently `1.0`. |
| `rules` | `array` | Yes | List of rule objects (may be empty). |
| `defaults` | `object` | No | Default effect per action category when no rule matches. Defaults to all `deny`. |

## `defaults`

Each key is an action category. Valid values are `"allow"` and `"deny"`.

| Category | Applies to |
|---|---|
| `file` | `PermissionRequest::FileOp` |
| `command` | `PermissionRequest::CommandExec` |
| `network` | `PermissionRequest::NetOp` |
| `inter_agent` | `PermissionRequest::InterAgentMsg` |
| `config` | `PermissionRequest::ConfigWrite` |

**Example** — allow all network by default but deny everything else:

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

## Rule Object

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

### Fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | `string` | Yes | Unique rule name. Used in denial messages. |
| `subject` | `object` | Yes | Defines which agents this rule applies to. |
| `effect` | `string` | Yes | `"allow"` or `"deny"`. |
| `actions` | `array` | Yes | List of action objects that this rule covers. |

### `subject`

```json
{
  "agent": "dev-agent-*",
  "match": "glob"
}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `agent` | `string` | — | Agent identifier or glob pattern. |
| `match` | `string` | `"exact"` | `"exact"` or `"glob"`. Glob supports `*` (single path segment) and `**` (recursive). |

### Glob Patterns

| Pattern | Matches | Does not match |
|---|---|---|
| `dev-agent-01` | `dev-agent-01` | `dev-agent-02` |
| `dev-agent-*` | `dev-agent-01`, `dev-agent-42` | `dev-agent` |
| `**` | anything | — |
| `/home/admin/**` | `/home/admin/code/main.rs` | `/home/other/file` |

## Action Types

### `file` — File Operations

```json
{
  "type": "file",
  "operation": "read",
  "paths": ["/home/admin/code/**"]
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string | Yes | Must be `"file"`. |
| `operation` | `string` | Yes | One of: `read`, `write`, `list`, `delete`, `execute`. |
| `paths` | `array<string>` | No | Glob patterns for allowed paths. Empty means all paths (within the allowed scope). |

### `command` — Shell Commands

```json
{
  "type": "command",
  "command": "git",
  "args": {
    "allowed": ["status", "log", "diff", "--*"]
  }
}
```

Or to block specific arguments:

```json
{
  "type": "command",
  "command": "rm",
  "args": {
    "blocked": ["-rf", "--no-preserve-root"]
  }
}
```

**`args` variants:**

| Variant | Meaning |
|---|---|
| `{}` or omitted | Any arguments allowed |
| `{"allowed": ["a", "b"]}` | Only these arguments (and their glob suffixes) allowed |
| `{"blocked": ["x", "y"]}` | These arguments are denied; all others allowed |

### `network` — Network Connections

```json
{
  "type": "network",
  "hosts": ["*.internal.corp", "localhost"],
  "ports": [80, 443, 8000]
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `hosts` | `array<string>` | No | Allowed host glob patterns. Empty = allow all. |
| `ports` | `array<number>` | No | Allowed port numbers. Empty = allow all. |

### `tool_call` — Skill / Tool Invocation

```json
{
  "type": "tool_call",
  "skill": "code-editor",
  "methods": ["read_file", "write_file"]
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `skill` | `string` | Yes | Skill identifier. |
| `methods` | `array<string>` | No | Allowed method names. Empty = all methods allowed. |

### `inter_agent` — Inter-Agent Messaging

```json
{
  "type": "inter_agent",
  "agents": ["admin-agent", "monitor-agent"]
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `agents` | `array<string>` | No | Allowed recipient agent IDs (glob patterns). Empty = allow all. |

### `config_write` — Configuration File Modifications

```json
{
  "type": "config_write",
  "files": ["/home/admin/.closeclaw/config.json"]
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `files` | `array<string>` | No | Allowed file paths (glob patterns). Empty = allow all. |

## Complete Example

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

## Rule Evaluation Algorithm

1. Extract the **agent ID** from the incoming `PermissionRequest`.
2. Look up rule indices for that agent in O(1) via the pre-built HashMap index.
3. If no exact match, fall back to scanning all rules for glob pattern matches.
4. If still no match, apply the **default** for the request's category.
5. Among matching rules, the first **deny** wins (AWS IAM style).
6. If only allows are found, the action is allowed and a short-lived token is returned.

## Loading a Ruleset

```rust
use closeclaw::permission::{PermissionEngine, RuleSet};

let json = std::fs::read_to_string("permissions.json")?;
let rules: RuleSet = serde_json::from_str(&json)?;
let engine = PermissionEngine::new(rules);
```

## Hot Reload

To reload rules without restarting the engine process:

```rust
let json = std::fs::read_to_string("permissions.json")?;
let rules: RuleSet = serde_json::from_str(&json)?;
sandbox.reload_rules(rules).await?;
```
