---
name: coding_agent
description: |
  Delegate complex coding tasks to AI coding agents (OpenCode, Claude Code).
  Suitable for tasks requiring deep code analysis, large refactors, or
  multi-file changes beyond simple single-method edits.
allowed_tools:
  - delegate
  - review
  - refactor
  - test
when_to_use: |
  Use when a task requires deep code understanding, large-scale refactoring,
  writing tests across multiple files, or delegating complex implementation
  work. Not needed for simple single-method edits or read-only analysis.
context: Fork
effort: Large
user_invocable: true
paths: []
---

# Coding Agent Skill

## Overview

Delegates complex coding tasks to external AI coding agents.
The skill returns a `fork` execution signal, instructing the caller
to spawn an isolated sub-agent to perform the actual work.

## Methods

| Method | Description | Required Args |
|--------|-------------|---------------|
| `delegate` | Delegate a coding task to an AI agent | `task`, `language` (opt) |
| `review` | Request a code review | `code` |
| `refactor` | Request refactoring of code | `code`, `goal` (opt) |
| `test` | Generate tests for given code | `code` |

## Execution Mode

This skill uses `context: Fork`. When called, it returns the task
details and signals the caller to spawn a sub-agent for execution.

## Examples

```json
// Delegate a coding task
{ "method": "delegate", "args": { "task": "implement feature X", "language": "rust" } }

// Request code review
{ "method": "review", "args": { "code": "fn old() {}" } }

// Refactor code
{ "method": "refactor", "args": { "code": "fn old() {}", "goal": "reduce complexity" } }

// Generate tests
{ "method": "test", "args": { "code": "fn add(a: i32, b: i32) -> i32 { a + b }" } }
```

## Response Shape

```json
{
  "status": "delegated",
  "task": "implement feature X",
  "language": "rust",
  "model": "minimax/MiniMax-M2.7",
  "message": "Coding task delegated - implementation stub"
}
```