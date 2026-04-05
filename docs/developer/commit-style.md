# Commit Message Style Guide

## Format
```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

## Types
| Type | Description |
|------|-------------|
| `feat` | New feature |
| `fix` | Bug fix |
| `docs` | Documentation changes |
| `refactor` | Code refactoring (no functional change) |
| `test` | Adding or updating tests |
| `perf` | Performance improvement |
| `chore` | Maintenance tasks (deps, build, CI) |

## Examples

### Feature
```
feat(permission): add glob matching for subject patterns

- Support * for single path segment
- Support ** for multiple segments
- Support ? for single character

Closes #123
```

### Bug Fix
```
fix(engine): correct args_match Blocked semantics

Previously, Blocked rules matched when NO args were blocked.
Now they match when ANY arg is blocked.

Before: git status matched "dev-agent-forbidden-git-reset" incorrectly
After: Only git reset matches the forbidden rule
```

### Documentation
```
docs: update SPEC.md with Phase 3 completion status

- Add Phase 3 status table
- Update architecture diagram
- Fix typos in Scheduler section
```

### Refactor
```
refactor(config): extract ConfigProvider trait

Split monolithic config.rs into separate provider modules.
Each provider now implements the ConfigProvider trait.
```

### Chore
```
chore: update dependencies in Cargo.toml

- tokio: 1.35.0 -> 1.40.0
- serde: 1.0 -> 1.0
- Add notify crate for file watching
```

## Rules
1. Use imperative mood ("add" not "added")
2. First line under 72 characters
3. **Reference issues with `Fixes #N` or `Refs #N` in the footer** — all commits on `main` branch must be traceable to a GitHub issue
   - Exception: reviewer audit checkpoint commits (footer contains `— Reviewer: [name]`) may omit issue reference

   **Body 引用宽容**：issue 引用也可以出现在 body 正文中（如 `关联 #23988456494`），不限于 footer 行。
4. One logical change per commit
5. Include context for non-trivial changes

## Issue Reference Format

Every non-exception commit on `main` **must** include a footer reference:

```
Fixes #N      #N is the GitHub issue number this commit resolves
Refs #N       #N is the GitHub issue number this commit relates to
```

Examples:
```
fix(permission): correct strip_thinking_tags to remove thinking content

Fixes #95
```

```
chore: run cargo fmt on entire codebase

Refs #86
```

### CI Gate
A CI step validates that commits on `main` contain `Fixes #N` or `Refs #N` (or the Reviewer exception). Commits failing this check will block merge.
