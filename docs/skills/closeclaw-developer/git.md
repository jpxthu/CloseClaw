# Git Operations Guide

## Available Commands
Use the `git_ops` skill with these methods:

### status
Check repository status.
```rust
git_ops.execute("status", serde_json::json!({})).await
```

### commit
Create a commit with message.
```rust
git_ops.execute("commit", serde_json::json!({"message": "feat: add feature"})).await
```

### push
Push commits to remote.
```rust
git_ops.execute("push", serde_json::json!({})).await
```

### pull
Pull from remote.
```rust
git_ops.execute("pull", serde_json::json!({})).await
```

### log
View recent commits.
```rust
git_ops.execute("log", serde_json::json!({})).await
```

## Branch Management
```bash
# Create and switch to new branch
git checkout -b feature-branch

# Switch back to master
git checkout master

# Delete merged branch
git branch -d feature-branch
```

## Workflow
1. Always create a new branch for features
2. Commit frequently with clear messages
3. Push before merging
4. Use `--force-with-lease` instead of `--force` when pushing to shared branches
