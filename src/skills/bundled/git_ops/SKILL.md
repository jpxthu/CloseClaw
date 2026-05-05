---
name: git_ops
description: |
  Git operations: status, commit, push, pull, log.
  View repository state, commit changes, and synchronize with remotes.
allowed_tools:
  - status
  - commit
  - push
  - pull
  - log
when_to_use: |
  Use when you need to check the current git status, commit changes,
  push to or pull from a remote, or view recent commit history.
context: Inline
effort: Small
user_invocable: true
paths: []
---

# Git Operations Skill

## Overview

Executes git commands in the current repository. All operations run
in the repository where the agent is currently working.

## Methods

| Method | Description | Required Args |
|--------|-------------|---------------|
| `status` | Show working tree status (porcelain) | — |
| `log` | Show recent commits (last 10, one line each) | — |
| `commit` | Create a commit with the given message | `message` |
| `push` | Push current branch to remote | — |
| `pull` | Pull from remote to current branch | — |

## Examples

```json
// Check status
{ "method": "status", "args": {} }

// View recent commits
{ "method": "log", "args": {} }

// Commit changes
{ "method": "commit", "args": { "message": "fix: resolve panic in handler" } }

// Push to remote
{ "method": "push", "args": {} }

// Pull from remote
{ "method": "pull", "args": {} }
```