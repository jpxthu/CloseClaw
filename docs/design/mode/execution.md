# Plan 执行引擎

## 概述

执行引擎将 User 满意的 plan 逐步骤落地执行。核心职责：执行触发、进度追踪、中断恢复、失败处理。

## 架构

### 执行触发

plan 写完后，User 通过两个等价的途径进入执行——斜杠指令 `/execute` 或执行触发工具。两者入参与执行落点一致，权威语义见需求 [mode F4](../../requirements/mode.md)。进入执行本身不属于需 Owner 审批的危险操作：它只把会话推进到执行态，不触达系统边界（本工具与审批链的分界见下文「执行触发工具」）。

**斜杠指令**：通过 `/execute <plan名称> [附加指令]` 命令触发执行。`plan名称` 为必选参数（即 plan 文件 identifier，命名见 [plan-mode.md](plan-mode.md)），指定要执行的 plan；`附加指令` 可选，空格后的内容作为一条用户消息注入 Auto Mode 初始对话。若当前处于 Plan Mode 则先退出。

**自然语言**：Agent 调用执行触发工具（权威定义见下节）。执行触发工具是模式的统一入口，供自然语言路径调用，参数与 `/execute` 相同（`plan名称` + 可选附加指令）。该工具自动向 User 发起确认交互——支持交互的通道弹出确认卡片，不支持的通道以自然语言让 User 回复确认。确认后进入 Auto Mode 开始执行。

**执行路径**：

- **同 session 执行**：当前 session 进入 Auto Mode（若处于 Plan Mode 则先退出），继承规划上下文
- **新 session 执行**：创建新 session，注入 plan 文件内容作为初始上下文，新 session 直接进入 Auto Mode

同一 plan 的并发执行不做系统级锁定，由 User 自行管理。

#### 执行触发工具（权威定义）

执行触发工具是模式域在进入 Auto Mode 执行时提供的入口能力，等价于 `/execute` 命令——它使自然语言成为合法的执行启动途径。本工具的全部行为在此唯一定义，`/execute` 斜杠指令与它是同一执行语义的两个触发通道：

- **触发通道**：自然语言触发——Agent 调用本工具发起执行；斜杠 `/execute` 由 Gateway 直接处理，不经由此工具调用，二者结果等价。
- **入参**：必选 `plan名称`（plan 文件 identifier，命名见 [plan-mode.md](plan-mode.md)）+ 可选附加指令。
- **执行效应**：确认后会话进入 Auto Mode 开始执行（若处于 Plan Mode 则先退出），随后按下文「进度管理」与「数据流」推进 plan 步骤。
- **启动确认 ≠ 审批**：本工具在启动执行时请求 User 确认，确认对象是「是否开始实施（切换/进入 Auto Mode）」，不是对某项具体系统调用的审批，也不写入审批记录。执行过程中触达系统边界的危险操作是否需 Owner 审批，由 Permission 审批链独立判定（详见 [permission 审批工作流](../permission/approval-workflow.md)）——两者确认/审批对象、记录与通道不同，互不混淆。

### Auto Mode 行为原则

Agent 在 Auto Mode 下以连续自主方式执行 plan 步骤。行为原则：

- 低风险操作直接执行，不等 User 逐步确认
- 常规决策自主做出，不升级给 User
- 不在执行中途主动重新进入 Plan Mode（User 在失败后显式选择回退修改 plan 的情形除外）
- 接受 User 随时发来的修正建议
- 危险操作（删数据、改生产配置、向外部平台发消息）必须经 User 确认
- 不擅自向外部平台发送消息

> **事实源**：本行为原则源自需求 F7。同一套原则的英文指令形式——主 Agent 见 [references/prompts.md](references/prompts.md) §4「Auto Mode 指令」，executor 子 Agent 见 §7「Execution Principles」。

### 进度管理

执行进度由 Agent 自行管理——Agent 在 plan 文件中以约定的格式标记步骤完成状态。系统不介入进度判断，Agent 是步骤完成与否的唯一判断者。**进度推进不依赖任何系统级的「进度工具」**——Agent 直接用写入 plan 文件的方式标记步骤状态，不存在逐步骤触发的系统介入或审批栅栏（步骤状态流转本身不产生审批；Auto Mode 下是否需审批只由单步操作是否触达系统边界决定，见 [permission 审批工作流](../permission/approval-workflow.md)）。

- Agent 按 plan 文件的 Tasks 节顺序执行步骤
- Tasks 节每个步骤以序号（Tasks 节顺序）与标题共同标识；User 可通过 /execute 附加指令或自然语言以此标识指定步骤或步骤子集
- 每步完成后 Agent 在 plan 文件中更新对应步骤标记
- 步骤状态由 Agent 自行判断：未开始 → 进行中 → 已完成 / 失败 / 已跳过
- 部分执行时本次执行范围（步骤子集）以步骤标识随进度一并记录在 plan 文件，供中断/压缩后恢复重建，子集外步骤不纳入执行

### 步骤状态

| 状态 | 含义 | 标记格式 |
|------|------|---------|
| 未开始 | 步骤尚未执行 | `[ ]` |
| 进行中 | 步骤正在执行 | `[-]` |
| 已完成 | 步骤成功完成 | `[x]` |
| 失败 | 步骤执行失败，需 User 介入 | `[!]` |
| 已跳过 | User 或 Agent 显式跳过 | `[~]` |

状态流转单向：未开始 → 进行中 → 已完成 / 失败 / 已跳过。失败后 User 可决定重试（失败 → 进行中）。已完成、已跳过不允许回退。

### 执行方式

执行方式完全由 User 通过自然语言指令决定，没有固定的模式约束：

- User 可指定在同 session 或新 session 中执行
- User 可指定执行全部步骤或部分步骤
- User 可要求 Agent spawn 子 Agent 来执行特定步骤

### 中断恢复

执行过程中的中断可无缝恢复，且暂停/恢复均通过自然语言触发，无专用斜杠指令：

- User 可随时暂停执行，当前进度被保存
- User 恢复执行时，Agent 从暂停时的当前步骤继续，不重复已完成步骤
- session 压缩或重启后 Agent 仍然知道当前执行进度（当前第几步、哪些已完成、哪些待继续），不需要从第 1 步重新执行
- plan 文件本身具备独立于 session 的恢复保障——即使 session 完全丢失，仍可基于 plan 文件内容重建执行上下文

### 失败处理

步骤是否失败由 Agent 自行判断，不存在系统级的重试次数限制或自动重试机制。Agent 认为某步骤失败后，User 可自由决定下一步操作——重试、显式选择回到 Plan Mode 修改后续步骤、跳过该步骤、或放弃。

### 审计日志

Auto Mode 下触发审批的危险操作会生成审计日志，User 可查看。日志记录操作内容及最终处置。

- 危险操作范围：删除数据、修改生产配置、向外部平台发送消息，以及 Permission 模块标记为需审批的操作
- 每条被审批的操作记录：工具名、操作描述、最终处置（批准/拒绝）、时间戳
- 按时间倒序排列
- 持久化（本地文件），跨 session 可见
- User 可配置日志存储上限

### 配置

审计日志存储上限和 plan 归档天数由 User 配置，详见 [config](../config/README.md) 模块。

## 数据流

### 同 session 执行（斜杠指令）

1. User `/execute <plan名称> [附加指令]`
2. 若处于 Plan Mode → 退出 Plan Mode
3. session 标记 Auto Mode（切换不立即生效，下一条用户消息前才应用约束）
4. 注入 Auto Mode 指令 + plan 文件内容；若含附加指令，其内容作为一条用户消息注入 Auto Mode 初始对话
5. Agent 按 plan Tasks 节顺序逐步执行
6. 每步完成后 Agent 更新 plan 文件步骤标记
7. 全部步骤完成 → session 退出 Auto Mode → 恢复默认模式

### 同 session 执行（自然语言触发）

1. User 自然语言要求执行
2. Agent 调用执行触发工具 → User 确认
3. 若处于 Plan Mode → 退出 Plan Mode
4. session 标记 Auto Mode（切换不立即生效，下一条用户消息前才应用约束）
5. 注入 Auto Mode 指令 + plan 文件内容
6. Agent 按 plan Tasks 节顺序逐步执行
7. 每步完成后 Agent 更新 plan 文件步骤标记
8. 全部步骤完成 → session 退出 Auto Mode → 恢复默认模式

### 新 session 执行

1. User 指定新 session 执行（通过 /execute 或自然语言）
2. 创建新 session
3. 注入 plan 文件内容作为初始上下文；若含附加指令，其内容一并注入
4. 注入 Auto Mode 指令
5. 新 session 直接进入 Auto Mode
6. Agent 按 plan Tasks 节顺序逐步执行（若为部分执行，界定并持久化执行范围，见「部分步骤执行」）
7. 每步完成后 Agent 更新 plan 文件步骤标记
8. 全部步骤完成 → session 退出 Auto Mode → 恢复默认模式

### 部分步骤执行

1. User 指定只执行 Tasks 节中部分步骤（通过 /execute 或自然语言）
2. 若处于 Plan Mode → 退出 Plan Mode
3. session 标记 Auto Mode（切换不立即生效，下一条用户消息前才应用约束）
4. 注入 Auto Mode 指令 + plan 文件内容；若通过 /execute 指定，附加指令（含步骤子集）作为一条用户消息注入
5. Agent 界定执行范围：User 指定的步骤子集（按 Tasks 节标识，见「进度管理」），该子集随进度一并写入 plan 文件，供中断/压缩后恢复重建
6. Agent 按序执行该子集内步骤，每步完成后更新 plan 文件对应步骤标记
7. 子集内全部步骤完成 → session 退出 Auto Mode → 恢复默认模式；子集外步骤保持原状态不变

> 本流程的执行范围界定与持久化同样适用于新 session 执行部分步骤的场景（见「新 session 执行」步骤 6）；执行范围以步骤标识记录于 plan 文件，见「进度管理」。

### Spawn 子 Agent 执行

1. User 指定 spawn 执行特定步骤
2. 父 session 处于 Auto Mode
3. spawn executor 子 Agent（传入步骤描述 + plan 上下文）
4. 子 Agent 执行 → 结果通知父 session
5. 父 session 更新 plan 文件对应步骤标记

### 中断恢复

1. User 以自然语言暂停（如「停一下」「暂停」）→ Agent 停止当前步骤，记录进度（含本次执行范围，若为部分执行）到 plan 文件
2. session 标记保持 Auto Mode
3. User 以自然语言恢复（如「继续」「继续执行」） → Agent 按记录的进度与执行范围，从当前步骤继续执行

### session 压缩后恢复

1. compaction 触发 → plan 文件不受压缩影响，独立于 session 持久化
2. session 续活时重新读取 plan 文件 Tasks 节
3. Agent 识别最后完成的步骤（或仍处于进行中的未完成步骤）与记录的本次执行范围，从相应步骤继续，不越出原执行范围

### 失败处理

1. Agent 判定步骤失败 → 标记 plan 文件对应步骤为失败
2. 停止后续步骤 → 通知 User：失败步骤 + 原因
3. User 决策：重试 / 回到 Plan Mode 修改后续步骤 / 跳过 / 放弃

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Plan Mode | User 满意 plan 后触发执行（触发途径之一，非唯一入口） |
| Slash Command | `/execute` 命令入口 |
| User | 自然语言触发执行、暂停/恢复、失败决策 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Agent | spawn executor 子 Agent |
| Session | 会话模式（normal/plan/auto）持久化、compaction 保护 |
| System Prompt | 注入 Auto Mode 指令 |
| Permission | Auto Mode 下运行时审查危险操作 |
| Tools | 执行触发工具注册和调用 |

### 模块内关系

- Plan 文件由 Plan Mode 阶段创建，执行引擎读取和更新步骤状态
- 失败后 User 可显式选择回到 Plan Mode 修改后续步骤（追加或修改未完成步骤，不改已完成的）

### 无关

| 模块 | 说明 |
|------|------|
| LLM Provider | 不直接调用 |
| Processor Chain / Renderer | 无关 |
| IM Adapter | 无关 |
