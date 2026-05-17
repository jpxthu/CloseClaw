# 命令执行

## 概述

`/exec` 指令用于以 owner 身份执行 Shell 命令，经 Permission 模块审批后执行。非 owner 用户调用直接拒绝。

## 架构

ExecHandler 本身不做权限判断——仅构造 SlashResult::Exec，权限验证由 Permission 模块负责。

```
/exec <command>
  ↓
ExecHandler 返回 Exec { command }
  ↓
Gateway 提交 Permission 模块
  ├── 非 owner → Permission 返回拒绝 → Reply("权限不足")
  └── owner → Permission 返回通过 → 执行命令 → 回复输出
```

权限判断完全由 Permission 模块处理，ExecHandler 不感知权限逻辑。

## 数据流

- **输入**：Shell 命令字符串
- **处理**：ExecHandler 构造 Exec 结果 → Gateway 调用 Permission 模块 → 审批通过后执行
- **输出**：命令执行结果或权限拒绝提示

## 模块关系

- **上游**：Gateway → Dispatcher → ExecHandler
- **下游**：Permission 模块（权限审批）；Shell 执行环境（命令执行）
- **无关**：WorkdirHandler（`/exec` 和 `/cd` 独立，不共享工作目录上下文）
