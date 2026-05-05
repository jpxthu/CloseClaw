---
name: permission_query
description: |
  Query the current agent's permission configuration.
  Check which actions (exec, file_read, file_write, network, spawn,
  tool_call, config_write) are allowed for this agent.
allowed_tools:
  - query
  - list_actions
when_to_use: |
  Use when you need to check whether the current agent is permitted
  to perform a specific action before attempting it.
context: Inline
effort: Trivial
user_invocable: true
paths: []
---

# Permission Query Skill

## Overview

Allows agents to introspect their own permission configuration.
Can query whether a specific action is allowed or list all
supported action types.

## Methods

| Method | Description | Required Args |
|--------|-------------|---------------|
| `query` | Check if an action is allowed for this agent | `agent_id`, `action` |
| `list_actions` | List all supported action types | — |

## Supported Actions

- `exec` — execute shell commands
- `file_read` — read files and directories
- `file_write` — write or delete files
- `network` — make outbound network requests
- `spawn` — spawn sub-agents or child processes
- `tool_call` — invoke tools
- `config_write` — modify configuration files

## Examples

```json
// Check if agent can read files
{ "method": "query", "args": { "agent_id": "agent-1", "action": "file_read" } }

// List all supported actions
{ "method": "list_actions", "args": {} }
```

## Response Shape

```json
// query response (allowed)
{ "allowed": true, "agent_id": "agent-1", "action": "file_read" }

// query response (denied)
{ "allowed": false, "agent_id": "agent-1", "action": "file_write", "reason": "..." }

// query response (no engine)
{ "allowed": null, "agent_id": "agent-1", "action": "file_read", "reason": "permission engine not available" }
```