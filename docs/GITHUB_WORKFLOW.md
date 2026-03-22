# GitHub Issue Workflow

## Overview
This workflow defines how Vibe虾 handles GitHub issues during development.

## Rules

### Issue Processing
1. **Read** all new issues
2. **Reply** to issues as needed
3. **Tag** issues appropriately
4. **Close/Open** issues as resolved

### Authorization Rules

| Issue 作者 | 操作 |
|-----------|------|
| **任何人** | 发现 → 评估 → 发给你 review → 确认后才实施 |

### Agent Identity
- All agent replies must be signed: `— Vibe虾 🦐` or `— CloseClaw Bot`
- Never impersonate the user

## Required Setup

### GitHub Token
```bash
# Set GH_TOKEN environment variable for GitHub CLI
export GH_TOKEN=your_github_token_here

# Or configure gh CLI
gh auth login
```

### GitHub CLI (gh)
Required for issue management. Install if not available:
```bash
# Check if installed
gh --version

# Install if needed (requires network access)
```

## Commands

### Read Issues
```bash
# List open issues
gh issue list --state open

# View specific issue
gh issue view <issue-number>
```

### Reply to Issue
```bash
gh issue comment <issue-number> --body "Reply text"
```

### Add Tags/Labels
```bash
gh issue edit <issue-number> --add-label "bug,priority-high"
```

### Close/Open Issue
```bash
gh issue close <issue-number>
gh issue reopen <issue-number>
```

### Create Issue
```bash
gh issue create --title "Issue title" --body "Description" --label "bug"
```

## Workflow (On Heartbeat)

1. Fetch open issues from GitHub
2. For each issue:
   - **评估**：值不值得做、对现有计划的影响
   - **需求待细化**：发计划给你 review，等确认后再实施
   - **需求已明确**：可直接实施，实施完要加 Labels、Close
   - Tag appropriately (bug/enhancement/question/documentation)
3. Update issue status as resolved

## Feature Development Workflow

### 开发流程总则

> **需求先对清楚，对清楚后直接开工。**

| 情况 | 处理方式 |
|------|---------|
| **需求待细化/待明确** | 发计划给你确认，等回复后再实施 |
| **需求已明确**（包括心跳中按 TODO 优先级推进） | 直接开工，无需额外确认 |

每个功能改动都必须经过测试，不能只写实现：

```
需求对清楚（chat 或 heartbeat）
   ↓
测试用例设计（test agent 或开发者）
   ↓
实现代码
   ↓
写自动化测试（UT + 集成测试）
   ↓
运行自动化测试，确保通过
   ↓
测试专员 agent **手动验收测试**（按测试用例操作、观察 stdout/log/调试接口、以用户方式体验）
   ↓
添加测试项目（将手动测试用例也沉淀为可自动化运行的测试脚本）
   ↓
提交 + Close issue
```

### 测试类型说明

| 测试类型 | 执行者 | 方式 |
|---------|--------|------|
| 单元测试（UT） | 开发者 / sub-agent | 跑 `cargo test` |
| 集成测试 | 开发者 / sub-agent | 跑 `cargo test` |
| **用户验收测试** | **测试专员 agent** | **按测试用例手动操作、看 stdout/log/调试接口、以用户方式体验** |

> ⚠️ 测试专员 agent 不仅手动验证，还要将测试用例沉淀为自动化脚本，两者缺一不可。

测试按模块分散到对应目录，不允许全部堆在 `tests/` 根目录：

```
src/
  ├── permission/
  │   ├── mod.rs
  │   └── tests.rs      ← permission 模块的单元测试（含 engine + rules validation）
  ├── agent/
  │   ├── mod.rs
  │   └── tests.rs      ← agent 模块的单元测试
  ├── config/
  │   └── mod.rs
  │   └── tests.rs      ← config 模块的单元测试
  └── skills/
      └── builtin.rs
          └── tests.rs  ← skill 模块的单元测试（含 builtin skills + registry）

tests/
  └── integration_tests.rs  ← 跨模块集成测试（占位）
```

> 禁止在 `tests/` 根目录创建新的针对单个模块的测试文件。
> 单元测试应放在 `src/<module>/tests.rs` 或 `src/<module>/<file>.rs` 的 `#[cfg(test)] mod tests` 中。

### 文档同步原则（必须遵守）

> **文档与代码对齐，是必须的开发原则，不可省略。**

- **Commit 前必检查**：每次代码 commit 前，必须检查对应文档是否需要同步更新
- **改动必更新文档**：功能、架构、流程的任何改动，必须先更新文档再提交 commit
- **定期巡检**：不定期运行 sub-agent 扫描代码库，验证文档与代码是否一致（方案 A：并行分块扫；方案 B：专用 survey skill）
- **文档即代码**：模块的 `docs/<module>/README.md` 是该模块的档案，扫代码时同步更新

违反此原则的代码提交，不符合开发规范。

## Issue Reply Rules

- **署名**：`— Vibe虾 🦐` 或 `— CloseClaw Bot`
- **Labels 必须加**：每个 issue 都要打标签（enhancement / bug / documentation / question 等）
- **Close 时机**：需求已实施 → Close；不打算现在改 → 只回复不 Close
