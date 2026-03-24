# Git 命令参考

> CloseClaw 项目常用 Git 命令清单。操作步骤见 [git-guide.md](git-guide.md)。

## 日常命令

| 命令 | 说明 |
|------|------|
| `git status` | 查看工作区状态 |
| `git add <file>` | 暂存文件 |
| `git commit -m "msg"` | 提交 |
| `git push` | 推送到远程 |
| `git push --force-with-lease` | 安全强制推送（推荐替代 `--force`） |
| `git pull --rebase` | 拉取并变基 |
| `git log --oneline -10` | 查看最近 10 条提交 |
| `git branch -a` | 查看所有分支 |
| `git diff` | 查看未暂存的变更 |
| `git diff --cached` | 查看已暂存的变更 |

## 分支操作

```bash
# 创建并切换到新分支
git checkout -b feature/xxx
git switch -c feature/xxx

# 切换分支
git checkout master
git switch master

# 删除已合并的分支
git branch -d feature/xxx
```

## 提交规范

推荐格式：`type: 简短描述`

常见 type：
- `feat:` 新功能
- `fix:` bug 修复
- `docs:` 文档变更
- `refactor:` 重构（不影响功能）
- `test:` 测试相关
- `chore:` 构建/工具变更

## CloseClaw 团队约定

- 所有变更通过 PR 合并
- commit 署名格式：`— 角色名: 描述`
- 大改动先开 GitHub Issue 讨论
