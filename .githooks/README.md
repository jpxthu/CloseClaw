# Git Hooks

本目录包含项目推荐的本地 Git hooks，与 CI 中的检查保持一致。

## 目录结构

```
.githooks/
├── commit-msg         # 提交信息格式检查（Source: + Type:）
├── pre-commit         # 代码风格检查（行数 + fmt）+ 角色规则
├── pre-push           # 禁止 force push 到保护分支 + 角色规则
├── roles/             # 角色规则目录
│   ├── dev.pre-commit # dev 角色：禁止改动 docs/design/ 和 CONTRIBUTING.md
│   └── dev.pre-push   # dev 角色：push 前兜底检查
└── README.md          # 本文件
```

## 配置

本项目使用 `core.hooksPath` 指向 `.githooks` 目录。配置已写入 `.git/config`：

```bash
git config core.hooksPath .githooks
```

**注意**：`core.hooksPath` 是 per-repo 配置，新克隆仓库后需重新执行上方命令。

## 角色系统

Hooks 支持基于角色的扩展检查。Base 检查对所有人生效，角色规则在此基础上增加额外约束。

### 设置角色

```bash
git config hooks.role dev
```

取消角色（回到纯 base 检查）：

```bash
git config --unset hooks.role
```

### 可用角色

| 角色 | 说明 | 限制 |
|------|------|------|
| `dev` | 普通开发者 | 禁止修改 `docs/design/` 和 `CONTRIBUTING.md` |

### 工作原理

Base hook 完成通用检查后，读取 `git config hooks.role`，如果存在对应的角色脚本（`roles/<role>.<hook-name>`），则加载并执行。

```
pre-commit 触发
  → 1. Base 检查（行数、env 禁令、fmt）
  → 2. 读取 hooks.role 配置
  → 3. 执行 roles/<role>.pre-commit（如有）
  → 4. 全部通过 → 放行
```

角色规则 **不允许绕过**（`--no-verify` 会跳过所有 hook，包括 base，不建议使用）。

### 添加新角色

在 `roles/` 目录下创建 `<role>.pre-commit` 和/或 `<role>.pre-push` 脚本即可。脚本以 bash 执行，退出非零则拦截。

## Hooks 说明

### pre-commit

在每次 `git commit` 前自动运行以下检查，全部通过才允许提交：

| 步骤 | 检查内容 | 阈值 | 失败时的提示 |
|---|---|---|---|
| 1 | 文件行数 | ≤ 1000 行 | `ERROR: <file>: <N> 行（上限 1000）` |
| 2 | `cargo fmt --check` | 行宽 100 等 | 格式不符合规范，运行 `cargo fmt` 修复 |
| 3 | 角色规则 | 取决于角色 | 见上方角色说明 |

**注意**：行数和 fmt 检查仅针对 staged 的 `.rs` 文件，不背负历史技术债。

### commit-msg

检查 commit message 是否包含 `Source:` 和 `Type:` footer。校验口径宽松：行内包含指定关键词即可，不苛求格式。

```
feat(auth): add OAuth2 support

Source: issue #123
Type: feat
```

- 必须存在 `Source:` 与 `Type:` 两行 footer（可重复多行，取包含关键词的行判定）
- `Source:` 行内包含任一来源关键词：`issue` `design-doc` `requirement` `Fixes` `Refs` `Closes` `CI` `user`
- `Type:` 行内包含任一类型关键词：`feat` `fix` `docs` `refactor` `test` `perf` `chore`
- 其余格式（`#N` 写法、路径、逗号列表等）不作要求；多 issue 合批可写 `Source: issue #A, #B` 或逐行重复 `Source: issue #N`

### pre-push

禁止 force push 到保护分支（`master` `main` `develop`）。

## 团队同步

`.githooks/` 目录已纳入版本控制，团队成员 pull 后需执行配置命令：

```bash
git config core.hooksPath .githooks
```

开发者还需设置自己的角色：

```bash
git config hooks.role dev
```

或者在克隆时一步到位：

```bash
git clone --config core.hooksPath=.githooks --config hooks.role=dev git@github.com:jpxthu/CloseClaw.git
```
