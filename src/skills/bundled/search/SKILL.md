---
name: search
description: |
  Web search capabilities. Search the web for information,
  news, documentation, and answers to questions.
allowed_tools:
  - search
when_to_use: |
  Use when you need to look up information on the web,
  find documentation, research topics, or get current data.
context: Inline
effort: Small
user_invocable: true
paths: []
---

# Search Skill

## Overview

Provides web search functionality. Send a query and receive relevant
results with titles, URLs, and snippets.

## Methods

| Method | Description | Required Args |
|--------|-------------|---------------|
| `search` | Search the web for the given query | `query` |

## Examples

```json
// Search for information
{ "method": "search", "args": { "query": "Rust async trait best practices" } }
```

## Notes

This is a stub implementation. Connect to a search API (e.g., DuckDuckGo,
Brave Search) for full functionality.