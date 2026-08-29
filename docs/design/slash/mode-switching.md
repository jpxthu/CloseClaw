# 模式切换

## 概述

`/plan`、`/mode` 和 `/execute` 指令用于在 Normal（默认）、Plan（规划）和 Auto（执行）三种会话模式之间切换。模式切换不立即变更 system prompt，仅标记会话状态；下一条用户消息进入 LLM 前由 system prompt builder 根据当前模式重新组装 prompt。

## 架构

模式切换的核心机制是延迟生效：指令执行时仅向会话记录模式状态，system prompt 的实际变更推迟到下一条消息的 prompt 构建阶段。

### /plan — 进入 Plan Mode

1. User 发送 `/plan [任务描述]`（描述可选）
2. ModeSwitchHandler 返回 SetMode(Plan)，将会话模式标记为 Plan
3. 若带任务描述 → 描述作为下一条用户消息注入对话
4. 回复「已切换到 Plan 模式」
5. 下一条用户消息进入 LLM 前，system prompt builder 检测到 Plan 模式，注入 Plan 工作流指令（标准 4 阶段或 Interview 路径），限制工具集为只读 + plan 文件写

`/plan` 后的额外文本作为初始任务描述转发给 Agent。不含参数时仅标记 Plan Mode 状态。

### /execute — 触发执行

1. User 发送 `/execute <plan名称> [附加指令]`（`<plan名称>` 即 plan 文件 identifier，命名见 [mode/plan-mode.md](../mode/plan-mode.md)）
2. ModeSwitchHandler 检查当前模式：处于 Plan Mode → 先退出 Plan Mode；不处于 Plan Mode → 直接进入
3. 将会话模式标记为 Auto
4. 回复「开始执行」
5. 下一条用户消息进入 LLM 前，system prompt builder 检测到 Auto 模式，注入 Auto Mode 指令 + plan 文件上下文

`plan名称` 为必选参数，指定要执行的 plan（即 plan 文件 identifier），缺少时提示用法错误。`附加指令` 可选，空格后的内容作为一条用户消息注入 Auto Mode 初始对话。

## 数据流

- **`/plan`**（无参数）：标记为 Plan 模式
- **`/plan <描述>`**：标记为 Plan 模式，描述作为下一条用户消息注入
- **`/execute <plan名称> [附加指令]`**：若处于 Plan Mode 则先退出，标记为 Auto 模式，注入 plan 文件上下文
- **`/mode`**（无参数，Immediate）：读取当前模式 → 回复「当前模式：Plan / Auto / Normal」
- **`/mode plan [描述]`**：等价于 `/plan [描述]`
- **`/mode normal`**：标记为 Normal，下一条消息恢复标准 system prompt
- **`/mode` 非法参数**：回复「无效模式。可用：normal, plan」，模式不变。显示状态可能为 Auto，但 Auto 模式不在 `/mode` 的切换参数内——进入 Auto 只能通过 `/execute`（见架构 /execute）

## 模块关系

- **上游**：Gateway（入站消息处理，`/` 前缀拦截，经 SlashDispatcher 分派给 ModeSwitchHandler）
- **下游**：Session 模块（记录/读取模式状态）；system prompt builder（读取模式决定 prompt 内容）
- **无关**：LLM 对话流程（切换本身不触发 LLM 调用）、ReasoningLevel（`/reasoning` 控制推理强度，模式控制 Agent 行为，两轴独立）、Verbosity（`/verbose` 控制信息展示等级，模式控制 Agent 行为，两轴独立）
