# 命令执行

## 概述

`/exec` 指令用于执行 Shell 命令，经 Permission 模块评估后执行。权限引擎对非 Owner 默认 Deny（可通过规则授权）；Owner 按 [permission 命令执行豁免](../permission/permission-dimensions.md#命令行) 默认放行、不进入评估，黑名单命令仍拒绝。

## 架构

ExecHandler 本身不做权限判断——仅构造 SlashResult::Exec，权限验证由 Gateway 调用 Permission 模块负责。

```
/exec <command>
  ↓
ExecHandler 返回 Exec { command }
  ↓
Gateway 提交 Permission 模块
  ├── 权限引擎评估 → Deny → 审批流程（见 [permission 审批工作流](../permission/approval-workflow.md#审批分流)）
  └── 权限引擎评估 → Allow → 执行命令 → 回复输出
```

权限判断完全由 Permission 模块处理，ExecHandler 不感知权限逻辑。

## 数据流

- **输入**：Shell 命令字符串
- **处理**：ExecHandler 构造 Exec 结果 → Gateway 调用 Permission 模块 → 允许或审批通过后执行
- **输出**：命令执行结果或权限拒绝提示

## 模块关系

- **上游**：Gateway → Dispatcher → ExecHandler
- **下游**：Shell 执行环境（命令执行）
- **间接下游**（通过 Gateway 调用）：Permission 模块（Gateway 在收到 Exec SlashResult 后调用权限引擎评估）
- **无关**：WorkdirHandler（`/exec` 和 `/cd` 独立，不共享工作目录上下文）
