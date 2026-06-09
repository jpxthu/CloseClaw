# 模式切换

## 概述

`/plan` 和 `/mode` 指令用于在 Normal（普通）和 Plan（规划）两种会话模式之间切换。模式切换不立即变更 system prompt，仅标记会话状态；下一条用户消息进入 LLM 前由 system prompt builder 根据当前模式重新组装 prompt。

## 架构

模式切换的核心机制是延迟生效：`set_mode()` 仅写入会话状态，system prompt 的实际变更推迟到下一条消息的 prompt 构建阶段。

```
/plan 或 /mode plan
  ↓
ModeSwitchHandler 返回 SetMode(Plan)
  ↓
Gateway 调用 session.set_mode(Plan)
  ↓
回复"已切换到 Plan 模式"
  ↓
下一条用户消息进入 LLM 前
  ↓
system prompt builder 检测 mode=Plan
  ↓
注入 Plan 工作流指令（Research → Design → Review → Confirm）
限制工具集为只读 + plan 文件写
```

`/plan` 后的额外文本（如 `/plan 设计缓存`）被丢弃，不传递到 Session。

## 数据流

- **`/plan`**：无参数 → SetMode(Plan)
- **`/mode`**（无参数）：从 SlashContext 读取当前模式 → Reply("当前模式：Plan")
- **`/mode plan`**：等价于 `/plan` → SetMode(Plan)
- **`/mode normal`**：SetMode(Normal) → 下一条消息恢复标准 system prompt
- **`/mode` 非法参数**：Reply("无效模式。可用：normal, plan")，模式不变

## 模块关系

- **上游**：Gateway → Dispatcher → ModeSwitchHandler
- **下游**：Session 模块（`set_mode()` 方法）；system prompt builder（读取 mode 决定 prompt 内容）
- **无关**：LLM 对话流程（切换本身不触发 LLM 调用）、ReasoningLevel（`/reasoning` 控制推理深度，Plan 模式控制 Agent 行为，两轴独立）、Verbosity（`/verbose` 控制信息展示等级，Plan 模式控制 Agent 行为，两轴独立）
