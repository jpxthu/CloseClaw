---
name: skill_creator
description: |
  Helps agents understand how to create new skills for CloseClaw.
  Provides a guide for the skill creation process, a SKILL.md
  template, and a code validator for skill implementations.
allowed_tools:
  - guide
  - template
  - validate
when_to_use: |
  Use when you need to create a new skill for CloseClaw and want
  guidance on the correct structure, or when you want to validate
  that a skill implementation follows the proper conventions.
context: Inline
effort: Small
user_invocable: true
paths: []
---

# Skill Creator Skill

## Overview

Assists in creating new CloseClaw skills by providing a built-in
guide, a SKILL.md template, and a code validator that checks for
required trait implementations.

## Methods

| Method | Description | Required Args |
|--------|-------------|---------------|
| `guide` | Return the built-in skill creation guide | — |
| `template` | Return a SKILL.md template | — |
| `validate` | Validate that Rust code implements the Skill trait | `code` |

## Examples

```json
// Get the skill creation guide
{ "method": "guide", "args": {} }

// Get the SKILL.md template
{ "method": "template", "args": {} }

// Validate a skill implementation
{ "method": "validate", "args": { "code": "use async_trait::async_trait; ..." } }
```

## Validation Checks

The `validate` method checks for:

- `#[async_trait]` — async trait derive macro present
- `fn manifest` — manifest method implemented
- `fn methods` — methods method implemented
- `async fn execute` — execute method implemented

## Creating a Skill

1. Create `src/skills/your_skill_name.rs` implementing the `Skill` trait
2. Register in `src/skills/mod.rs`
3. Create `src/skills/bundled/your_skill_name/SKILL.md`
4. Write tests
5. Run `cargo test`