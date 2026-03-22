# Permission Rules Guide

## Overview
Permissions use deny-take-precedence (AWS IAM style). If any rule denies, the request is denied.

## Rule Structure

```json
{
  "version": "1.0",
  "rules": [
    {
      "name": "rule-name",
      "subject": { "agent": "agent-name" },
      "effect": "allow",
      "actions": [...]
    }
  ],
  "defaults": { "effect": "deny" }
}
```

## Subject Matching

### Exact Match
```json
"subject": { "agent": "vibe" }
```

### Glob Pattern
```json
"subject": { "agent": "dev-*", "match": "glob" }
```

Glob patterns: `*` (single segment), `**` (multi-segment), `?` (single char)

## Action Types

### File Operation
```json
{
  "type": "file",
  "operations": ["read", "write", "delete"],
  "paths": ["src/**", "tests/**"]
}
```

### Command Execution
```json
{
  "type": "command",
  "command": "git",
  "args": { "allowed": ["status", "log"] }
}
```

### Network Request
```json
{
  "type": "network",
  "hosts": ["api.github.com"],
  "ports": ["80", "443"]
}
```

### Tool Call
```json
{
  "type": "tool_call",
  "tools": ["file_ops", "git_ops"]
}
```

### Inter-Agent Message
```json
{
  "type": "inter_agent",
  "agents": ["parent-agent"]
}
```

### Config Write
```json
{
  "type": "config_write",
  "files": ["configs/agents.json"]
}
```

## Rule Order
1. All matching rules are collected
2. If ANY rule is `deny`, request is denied
3. If at least one rule is `allow` and none are `deny`, request is allowed
4. If no rules match, `defaults` applies

## Default Rule (Recommended)
```json
{
  "name": "default-deny",
  "subject": { "agent": "*" },
  "effect": "deny",
  "actions": [{ "type": "file", "operations": ["write", "delete"], "paths": ["**"] }]
}
```
