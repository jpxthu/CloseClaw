# Git Hooks

本目录包含项目推荐的本地 Git hooks，与 CI 中的检查保持一致。

## 目录结构

```
.githooks/
├── commit-msg    # 提交信息格式检查（Source: + Type:）
├── pre-commit   # 代码风格检查（fmt + clippy + 行数）
├── pre-push     # 禁止 force push 到保护分支
└── README.md    # 本文件
```

## 配置

本项目使用 `core.hooksPath` 指向 `.githooks` 目录。配置已写入 `.git/config`：

```bash
git config core.hooksPath .githooks
```

**注意**：`core.hooksPath` 是 per-repo 配置，新克隆仓库后需重新执行上方命令。

## Hooks 说明

### pre-commit

在每次 `git commit` 前自动运行以下检查，全部通过才允许提交：

| 步骤 | 检查内容 | 失败时的提示 |
|---|---|---|
| 1 | `cargo fmt --check` | 格式不对，运行 `cargo fmt` 修复 |
| 2 | `cargo clippy --all -- -D warnings` | clippy 报错，修复 lint 问题 |
| 3 | `taplo fmt --check`（如已安装） | TOML 格式错误 |
| 4 | 文件行数 ≤ 500 | 文件超过 500 行，必须先拆分 |

**依赖安装（一次性）：**
```bash
cargo install clippy
cargo install taplo
```

### commit-msg

检查 commit message 是否包含 `Source:` 和 `Type:` footer。

```
feat(auth): add OAuth2 support

Source: issue #123
Type: feat
```

Source 支持：`issue #N`、`Fixes #N`、`Refs #N`、`Closes #N`、`CI`、`user`  
Type 支持：`feat` `fix` `docs` `refactor` `test` `perf` `chore`

### pre-push

禁止 force push 到保护分支（`master` `main` `develop`）。

检测原理：push 前 fetch remote sha，通过 `git merge-base --is-ancestor` 判断 remote sha 是否为 local sha 的祖先——若不是，则为 force push，拦截并报错。

## 团队同步

`.githooks/` 目录已纳入版本控制，团队成员 pull 后需执行配置命令：

```bash
git config core.hooksPath .githooks
```

或者在克隆时通过 `git clone --config core.hooksPath=.githooks` 自动配置。
