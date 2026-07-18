# 模式切换

## 概述

`/plan`、`/mode`、`/auto` 和 `/execute` 指令用于在 Normal（默认）、Plan（规划）和 Auto（执行）三种会话模式之间切换。模式切换不立即变更 system prompt，仅标记会话状态；下一条用户消息进入 LLM 前由 system prompt builder 根据当前模式重新组装 prompt。

## 架构

模式切换的核心机制是延迟生效：`set_mode()` 仅写入会话状态，system prompt 的实际变更推迟到下一条消息的 prompt 构建阶段。

### /plan — 进入 Plan Mode

```
/plan [任务描述]
  ↓
ModeSwitchHandler 返回 SetMode(Plan)
  ↓
Gateway 调用 session.set_mode(plan)
  ↓
若带任务描述参数 → 转发给 Agent 作为初始输入
  ↓
回复"已切换到 Plan 模式"
  ↓
下一条用户消息进入 LLM 前
  ↓
system prompt builder 检测 mode=Plan
  ↓
注入 Plan 工作流指令（标准 4 阶段 或 Interview 路径）
限制工具集为只读 + plan 文件写
```

`/plan` 后的额外文本作为初始任务描述转发给 Agent。不含参数时仅标记 Plan Mode 状态。

### /auto — 直接进入 Auto Mode

```
/auto
  ↓
ModeSwitchHandler 返回 SetMode(Auto)
  ↓
Gateway 调用 session.set_mode(auto)
  ↓
回复"已切换到 Auto 模式"
  ↓
下一条用户消息进入 LLM 前
  ↓
system prompt builder 检测 mode=Auto
  ↓
注入 Auto Mode 连续执行指令
恢复完整工具集（危险操作受运行时审查）
```

`/auto` 不接受参数。

### /execute — 触发执行

```
/execute
  ↓
ModeSwitchHandler 检查当前模式
  ├── 处于 Plan Mode → 退出 Plan Mode，进入 Auto Mode
  └── 不处于 Plan Mode → 直接进入 Auto Mode
  ↓
Gateway 调用 session.set_mode(auto)
  ↓
回复"开始执行"
  ↓
下一条用户消息进入 LLM 前
  ↓
system prompt builder 检测 mode=Auto → 注入 Auto Mode 指令 + plan 文件上下文
```

## 数据流

- **`/plan`**（无参数）：SetMode(Plan)
- **`/plan <描述>`**：SetMode(Plan) + 转发描述为初始输入
- **`/auto`**：SetMode(Auto)
- **`/execute`**：若处于 Plan Mode 则退出后 SetMode(Auto)；否则直接 SetMode(Auto)
- **`/mode`**（无参数）：从 SlashContext 读取当前模式 → Reply("当前模式：Plan" / "当前模式：Auto" / "当前模式：Normal")
- **`/mode plan`**：等价于 `/plan`
- **`/mode auto`**：等价于 `/auto`
- **`/mode normal`**：SetMode(Normal) → 下一条消息恢复标准 system prompt
- **`/mode` 非法参数**：Reply("无效模式。可用：normal, plan, auto")，模式不变

## 模块关系

- **上游**：Gateway → Dispatcher → ModeSwitchHandler
- **下游**：Session 模块（`set_mode()` 方法）；system prompt builder（读取 mode 决定 prompt 内容）
- **无关**：LLM 对话流程（切换本身不触发 LLM 调用）、ReasoningLevel（`/reasoning` 控制推理深度，模式控制 Agent 行为，两轴独立）、Verbosity（`/verbose` 控制信息展示等级，模式控制 Agent 行为，两轴独立）
