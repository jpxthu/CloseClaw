# 模式系统

## 概述

- 关联需求文档：[requirements/mode.md](../../requirements/mode.md)
- 模式系统管理 session 的运行模式，通过切换模式改变 Agent 的工具可用性、系统提示词和权限边界。plan 和执行是两个独立的事情——plan 写完后可以在同 session 内执行（继承规划上下文），也可以新 session 执行（从 plan 文件读取背景）。

## 架构

### 模式类型

| 模式 | 说明 |
|------|------|
| 默认模式 | 无模式标记时的行为状态——Agent 按完整配置运行，全工具集可用，无额外行为约束 |
| Plan Mode | Agent 只做规划不做执行——工具集受限为只读（仅 plan 文件可写）。User 可反复要求 Agent 修改 plan。Plan Mode 没有审批栅栏——User 说"执行"时才退出 |
| Auto Mode | Agent 连续自主执行 plan 步骤，不等 User 逐步确认，但危险操作仍需 User 审批。可直接进入，不需要先经过 Plan Mode |

### 模式切换规则

- User 通过斜杠指令进入 Plan Mode，通过 `/execute` 或自然语言触发进入 Auto Mode
- Plan Mode 与 Auto Mode 相互独立——进入 Auto Mode 也不需要先经过 Plan Mode；Plan Mode 下 User 通过斜杠指令或自然语言触发执行时，退出 Plan Mode 并进入 Auto Mode，其余退出不自动进入 Auto Mode
- Auto Mode 下全部步骤到达终态（含失败后 User 放弃）时自动退出并恢复默认模式（失败处理详见 [execution.md](execution.md)）
- `/mode`（无参数）查询当前模式，`/mode <非法值>` 回复「无效模式」类错误提示且模式不变；指令解析与回复由 [slash/mode-switching.md](../slash/mode-switching.md) 承载

### 模式生效机制

每种模式通过两个层面影响 Agent 行为：

- **工具过滤**：按模式规则限制可用工具集。Plan Mode 下仅 plans/ 目录可写，其余写工具不可见；Auto Mode 下完整工具集可见，但危险操作需运行时审查。切换由模式系统自身执行
- **系统提示词注入**：根据模式注入特定的行为指令。Plan Mode 注入双路径工作流指引，Auto Mode 注入连续执行指令

两种约束随模式切换动态生效——进入每种模式时应用该模式的约束集，退出时释放。切换不立即生效：模式标记在下一条用户消息到达前应用，因此切换到实际生效存在一条消息的延迟。Auto Mode 下危险操作的运行时审查由 Permission 层负责。

### plan 文件

每个 plan 以独立文件持久化到 `workspace/plans/`。plan 本身无全局状态，只有步骤级状态（未开始/进行中/已完成/失败/已跳过），由 Agent 自行管理。文件内容与命名格式（User 在时间戳格式和随机词组格式间选择）详见 [plan-mode.md](plan-mode.md)。

全部步骤处于终态（已完成 `[x]`、失败 `[!]` 或已跳过 `[~]`）的 plan，其访问时间戳超过配置天数后，由 Daemon 后台任务 PlanArchiveSweeper 定时扫描并自动归档到 `workspace/plans/archive/`（任务调度见 [daemon/README.md](../daemon/README.md)，停止机制见 [daemon/shutdown.md](../daemon/shutdown.md)）。访问时间戳由应用层在 Agent 触达 plan 时记录——Agent 读取 plan 内容、写步骤状态、或基于 plan 重建执行上下文会刷新；`/plans` 查看等纯浏览不刷新。归档天数由 User 配置（承载见 [config](../config/README.md)）。

### 子功能索引

| 子功能 | 说明 |
|--------|------|
| [plan-mode.md](plan-mode.md) | Plan Mode 专项：标准路径 4 阶段、Interview 路径、Agent 类型、安全机制 |
| [execution.md](execution.md) | 执行引擎：执行触发、进度管理、中断恢复、失败处理、审计日志 |
| [plan-browse.md](plan-browse.md) | plan 浏览与管理：`/plans` 命令和自然语言列/看/废弃 plan |
| [references/prompts.md](references/prompts.md) | 各模式注入指令的固定文案（参考资料，非子功能文档） |

## 数据流

### 进入 Plan Mode

1. User `/plan [任务描述]`（描述可选；不带描述时仅切换模式）
2. session 设置 plan_mode 标记（切换不立即生效，下一条用户消息前才应用约束）
3. 若带任务描述 → 作为下一条用户消息注入对话
4. 完整路径（含工具过滤、系统提示词注入、标准/Interview 路径自选）见 [plan-mode.md](plan-mode.md) 数据流

### 进入 Auto Mode

1. User `/execute <plan名称>`，或自然语言触发（Agent 调用执行触发工具）。`<plan名称>` 即 plan 文件 identifier（命名见 [plan-mode.md](plan-mode.md)）
2. session 设置 auto_mode 标记（切换不立即生效，下一条用户消息前才应用约束）
3. 恢复完整工具集（危险操作受运行时审查）
4. 注入 Auto Mode 指令 + plan 文件上下文
5. Agent 开始执行 plan 步骤（详见 [execution.md](execution.md) 数据流）

### 退出 Plan Mode → 进入 Auto Mode

1. Plan Mode 下 User 触发执行（`/execute` 或自然语言）
2. session 清除 plan_mode 标记 → 设置 auto_mode 标记（切换不立即生效，下一条用户消息前才应用约束）
3. 恢复完整工具集（危险操作受运行时审查）
4. 注入 Auto Mode 指令 + plan 文件上下文
5. Agent 开始执行 plan 步骤

### 退出 Auto Mode

1. 全部步骤到达终态（全部完成，或失败后 User 决定放弃）
2. session 清除 auto_mode 标记
3. 恢复默认模式

### 退出 Plan Mode

1. User `/mode normal`（不触发执行）
2. session 清除 plan_mode 标记
3. 恢复完整工具集与默认模式

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Slash Command | `/plan`、`/mode`、`/execute`、`/plans` 命令入口 |
| User | 自然语言触发执行 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Session | 存储和持久化模式标记，压缩保护 |
| System Prompt | 接收模式指令，拼入最终 system prompt |
| Agent | session 读取模式标记，按模式生成对应类型子 Agent（详见 [plan-mode.md](plan-mode.md) Agent 类型表） |
| Permission | Auto Mode 下运行时审查危险操作 |
| Tools | 注册执行触发工具 |

### 无关

| 模块 | 说明 |
|------|------|
| LLM Provider | 不直接调用 |
| Processor Chain / Renderer | 无关 |
| IM Adapter | 无关 |
