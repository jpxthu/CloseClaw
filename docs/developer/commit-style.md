# Commit Message Style Guide

## Format
```
<type>(<scope>): <description>

[optional body]

Source: <source>
Type: <type>
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

## Example
```
feat(permission): add glob matching for subject patterns

- Support * for single path segment
- Support ** for multiple segments
- Support ? for single character

Source: issue #123
Type: feat
```

## Rules

1. Use imperative mood ("add" not "added")
2. First line under 72 characters
3. **Source footer is required** — one of:
   - `issue #N` — traced to a GitHub issue
   - `CI` — driven by CI failure or workflow change
   - `user` — from user demand or feedback
   - `Fixes #N` / `Refs #N` / `Closes #N` — legacy format, equivalent to `issue #N`
4. **Type footer is required** — same value as `<type>` in the title
5. One logical change per commit
6. Include context for non-trivial changes

## CI Gate

A CI step validates every commit on `main` for:
- Presence of `Source:` footer with value `issue #N`, `Fixes #N`, `Refs #N`, `Closes #N`, `CI`, or `user`
- Presence of `Type:` footer with a valid type

Commits failing this check will block merge.
