---
name: skill_discovery
description: |
  Search, install, and manage skills from ClawHub marketplace.
  Find skills by keyword, install new ones, list installed skills,
  and update existing skills.
allowed_tools:
  - find
  - install
  - list
  - update
when_to_use: |
  Use when you need to discover new capabilities, install a skill
  from the marketplace, see what skills are available, or upgrade
  an existing skill to a newer version.
context: Inline
effort: Medium
user_invocable: true
paths: []
dependencies:
  - clawhub
---

# Skill Discovery Skill

## Overview

Interfaces with the ClawHub marketplace to search for, install, list,
and update skills. Agents need `spawn` permission to install or update skills.

## Methods

| Method | Description | Required Args |
|--------|-------------|---------------|
| `find` | Search the marketplace by keyword | `query` |
| `install` | Install a skill from the marketplace | `agent_id`, `skill` |
| `list` | List all installed skills | — |
| `update` | Update a specific skill or all skills | — |

## Examples

```json
// Search for a skill
{ "method": "find", "args": { "query": "web scraping" } }

// Install a skill
{ "method": "install", "args": { "agent_id": "agent-1", "skill": "web_scraper" } }

// List installed skills
{ "method": "list", "args": {} }

// Update a specific skill
{ "method": "update", "args": { "skill": "web_scraper" } }

// Update all skills
{ "method": "update", "args": {} }
```

## Permissions

Installing or updating skills requires `spawn` permission, as it may
invoke external processes (clawhub CLI).