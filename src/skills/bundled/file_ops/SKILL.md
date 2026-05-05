---
name: file_ops
description: |
  File system operations: read, write, list, delete, exists.
  Direct file manipulation for agents with appropriate permissions.
allowed_tools:
  - read
  - write
  - delete
  - list
  - exists
when_to_use: |
  Use when you need to read from or write to the file system,
  list directory contents, check if files exist, or delete files.
  Requires `file_read` or `file_write` permission depending on operation.
context: Inline
effort: Small
user_invocable: true
paths: []
---

# File Operations Skill

## Overview

Provides direct file system operations including read, write, list, delete, and exists checks.
Agents must hold the appropriate permission (`file_read` or `file_write`) before executing.

## Methods

| Method | Description | Required Args |
|--------|-------------|---------------|
| `read` | Read file contents as string | `path` |
| `write` | Write string content to a file (overwrites) | `path`, `content` |
| `delete` | Delete a single file | `path` |
| `exists` | Check if a path exists and is a file | `path` |
| `list` | List directory entries | `path` (defaults to `.`) |

## Permissions

| Action | Required Permission |
|--------|---------------------|
| read, exists, list | `file_read` |
| write, delete | `file_write` |

## Examples

```json
// Read a file
{ "method": "read", "args": { "path": "/tmp/example.txt" } }

// Write a file
{ "method": "write", "args": { "path": "/tmp/out.txt", "content": "hello" } }

// Check existence
{ "method": "exists", "args": { "path": "/tmp/example.txt" } }

// List directory
{ "method": "list", "args": { "path": "/home/admin" } }

// Delete file
{ "method": "delete", "args": { "path": "/tmp/out.txt" } }
```