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

| Issue Author | Action Allowed |
|--------------|---------------|
| **jpxthu** (owner) | Full: evaluate, reply, tag, close/open, **make code changes** |
| **Anyone else** | Reply only. For code changes: must @mention jpxthu for review |

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
   - If from jpxthu and has code change request → implement, test, push
   - If from others → reply (with @jpxthu mention if code change needed)
   - Tag appropriately (bug/feature/question/docs)
3. Update issue status as resolved

## Security Constraint
**Code changes only for jpxthu's issues.** All other issues: reply only, @jpxthu for review.
