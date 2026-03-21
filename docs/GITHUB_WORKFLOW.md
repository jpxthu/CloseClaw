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
   - **发计划给你 review**：等确认后再实施
   - **确认后**才实施，实施完要加 Labels、Close
   - Tag appropriately (bug/enhancement/question/documentation)
3. Update issue status as resolved

## Issue Reply Rules

- **署名**：`— Vibe虾 🦐` 或 `— CloseClaw Bot`
- **Labels 必须加**：每个 issue 都要打标签（enhancement / bug / documentation / question 等）
- **Close 时机**：需求已实施 → Close；不打算现在改 → 只回复不 Close
