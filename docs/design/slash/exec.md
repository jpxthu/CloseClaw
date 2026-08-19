# 命令执行

## 概述

`/exec` 指令用于执行 Shell 命令，经 Permission 模块评估后执行。权限引擎对非 Owner 默认 Deny（可通过规则授权），Owner 默认 Allow。

## 架构

ExecHandler 本身不做权限判断——仅构造 SlashResult::Exec 并携带**操作描述**（要执行的命令），权限评估由 Gateway 读取该描述后提交 Permission 模块负责。执行流程：

1. ExecHandler 返回 Exec 变体（携带操作描述）
2. Gateway 读取操作描述并提交 Permission 模块评估
3. 权限引擎按描述评估：Allow → 执行命令并回复输出；Deny → 回复"权限不足"

权限判断完全由 Permission 模块处理，ExecHandler 不感知权限逻辑——只负责在操作描述中声明本次操作的内容，Exec 恒为携带操作描述的需评估操作。

## 数据流

- **输入**：Shell 命令字符串
- **处理**：ExecHandler 构造 Exec 结果（携带操作描述）→ Gateway 读取操作描述并调用 Permission 模块 → 权限评估通过后执行
- **输出**：命令执行结果或权限拒绝提示

## 模块关系

- **上游**：Gateway → Dispatcher → ExecHandler
- **下游**：Shell 执行环境（命令执行）
- **间接下游**（通过 Gateway 调用）：Permission 模块（Gateway 读取 Exec SlashResult 携带的操作描述后，统一提交权限引擎评估）
- **无关**：WorkdirHandler（`/exec` 和 `/cd` 独立，不共享工作目录上下文）
