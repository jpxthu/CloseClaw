# Commit Message Style Guide

## Specification: CI Commit Message Validation

### Issue Reference
- **Issue**: #157 - CI 规则修改: 统一使用英文关键词, 增加 Closes 支持

### Current State
- CI workflow (`.github/workflows/ci.yml` line 52) currently accepts: `Fixes|Refs|关联|关闭`
- No formal commit style guide document exists

### Target State
1. CI regex updated to: `(Fixes|Refs|Closes) #?[0-9]+`
2. This document created to formalize commit message conventions

---

## Commit Message Format

### Required Footer Format
All commits must include a footer in one of these formats:

```
Fixes #<issue_number>
Refs #<issue_number>
Closes #<issue_number>
```

### Examples

**Valid:**
```
feat: add user authentication module

Fixes #123
```

```
fix: resolve memory leak in connection pool

Refs #456
```

```
docs: update API documentation

Closes #789
```

**Invalid:**
```
feat: add new feature  # Missing footer
```

```
fix: bug fix

关闭 #123  # Chinese keyword not allowed
```

---

## Implementation

### File: `.github/workflows/ci.yml`
- **Line 52** regex changed from: `(Fixes|Refs|关联|关闭)`
- **Line 52** regex changed to: `(Fixes|Refs|Closes)`

### File: `docs/commit-style.md`
- This document created to establish commit style standard

---

## Acceptance Criteria

1. ✅ CI regex accepts `Fixes`, `Refs`, and `Closes`
2. ✅ CI regex rejects Chinese keywords (`关联`, `关闭`)
3. ✅ Commit style guide document exists at `docs/commit-style.md`
4. ✅ All examples use English keywords only

---

## Notes
- Use `Fixes` when the commit **resolves** the issue
- Use `Refs` when the commit **references** the issue (but doesn't resolve it)
- Use `Closes` as an alternative to `Fixes` (semantically equivalent)
