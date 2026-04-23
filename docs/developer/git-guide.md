# Git 工作流

## 核心规则

- **禁止直接 push master**——所有变更必须通过 PR（squash merge）
- **禁止 force push master**——无论什么情况
- 一个 PR = 一个 squash commit，master 保持线性历史
- commit message 遵守 [commit-style.md](commit-style.md)

## 标准开发流程

```bash
# 1. 拉最新 master，创建功能分支
git checkout master && git pull --rebase
git checkout -b feat/xxx

# 2. 开发
# ...写代码...

# 3. 提交前检查（本地 hook 会自动跑，这里手动列出供参考）
cargo fmt && cargo clippy -- -D warnings && cargo test

# 4. 提交（遵守 commit-style）
git add -A
git commit -m "feat(scope): 简短描述

Source: issue #N
Type: feat"

# 5. 开发过程中 master 有更新？rebase 而非 merge
git fetch origin
git rebase origin/master

# 6. 推送并创建 PR
git push -u origin feat/xxx
gh pr create --fill
```

## PR 创建与合并

> ⚠️ **PR merge 时不要加 `--subject` 和 `--body` 参数**。squash merge 会把 PR title 作为 commit subject、PR body 作为 commit body，保留完整的变更信息。
>
> ```bash
> gh pr merge --squash --delete-branch
> ```

开发 agent 的 code review 由自己 spawn sub-agent 完成，不需要等外部 review。

```bash
# 创建 PR 后 squash merge（不覆盖 subject 和 body）
gh pr merge --squash --delete-branch
```

## 分支命名

| 前缀 | 用途 |
|------|------|
| `feat/` | 新功能 |
| `fix/` | Bug 修复 |
| `docs/` | 文档变更 |
| `refactor/` | 重构 |
| `test/` | 测试 |
| `chore/` | 构建/工具 |

## master 保护（本地 hook）

项目自带 `pre-push` hook，阻止直接 push 到 master。

### 安装

```bash
git config core.hooksPath .githooks
```

### Hook 内容

`.githooks/pre-push`：

```bash
#!/bin/bash
while read local_ref local_sha remote_ref remote_sha; do
    branch=$(echo "$remote_ref" | sed 's|refs/heads/||')
    if [ "$branch" = "master" ]; then
        echo "ERROR: 直接 push 到 master 被禁止。请通过 PR squash merge。"
        exit 1
    fi
done
```

## 常用命令速查

```bash
# 创建并切换分支
git switch -c feat/xxx

# 删除已合并的分支
git branch -d feat/xxx

# 推送时用 --force-with-lease（替代 --force）
git push --force-with-lease

# 查看当前状态
git status
git log --oneline -10
```
