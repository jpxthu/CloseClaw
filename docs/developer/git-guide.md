# Git Workflow Guide

> 如何使用 CloseClaw 进行 Git 版本控制工作流。

## 基础工作流

1. **创建新分支** — 每次新功能或修复都从 `master`/`main` 创建新分支
2. **频繁提交** — 用清晰的 commit message 记录每步进展
3. **推送前检查** — 确认 `git status` 无意外文件
4. **推送到远程** — 推送到共享分支时使用 `--force-with-lease` 而非 `--force`

## 分支管理

```bash
# 从 master 创建并切换到新分支
git checkout -b feature/xxx

# 从 master 创建并切换到新分支（等价）
git switch -c feature/xxx

# 切换回 master
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

## 与 closeclaw 项目协作

CloseClaw 团队约定：
- 所有变更通过 PR 合并
- commit 署名格式：`— 角色名: 描述`
- 大改动先开 GitHub Issue 讨论

命令 Reference 见 [git-reference.md](git-reference.md)。
