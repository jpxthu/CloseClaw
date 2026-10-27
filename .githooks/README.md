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
  → 1. Base 检查（行数、env 禁令、fmt、行宽）
  → 2. 读取 hooks.role 配置
  → 3. 执行 roles/<role>.pre-commit（如有）
  → 4. 全部通过 → 放行
```

角色规则 **不允许绕过**（`--no-verify` 会跳过所有 hook，包括 base，不建议使用）。

### 添加新角色

在 `roles/` 目录下创建 `<role>.pre-commit` 和/或 `<role>.pre-push` 脚本即可。脚本以 bash 执行，退出非零则拦截。

## Hooks 说明

判定细则以 `CONTRIBUTING.md` 与 `docs/developer/STANDARDS.md` 为准，本目录只放脚本。

- `pre-commit`：文件行数 / env 禁令 / test-audit 静态扫描 / fmt / 行宽
- `commit-msg`：`Source:` / `Type:` footer + 分支前缀与主 type 一致性
- `pre-push`：禁止 push 到保护分支
