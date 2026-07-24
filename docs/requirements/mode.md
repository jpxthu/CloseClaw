# mode 需求

## 概述

Owner/User 可以切换 Agent 的运行模式来控制 Agent 的行为边界——规划时只读不执行，执行时自主推进但不越安全红线。规划与执行是两个独立的事情：plan 写完后可以在同 session 内执行（继承规划上下文），也可以新 session 执行（从 plan 文件读取背景）。

## 功能需求

### F1. 运行模式

Agent 在 session 内可运行于以下模式之一，每种模式决定了 Agent 可用的工具集、行为约束和权限边界。

- 默认模式：Agent 按完整配置运行，全工具集可用，无额外行为约束
- Plan Mode：Agent 只做规划不做执行，工具集受限为只读（仅 plan 文件可写）。User 可以反复要求 Agent 修改 plan，Plan Mode 没有审批栅栏——User 说"执行"时才退出
- Auto Mode（执行模式）：Agent 连续自主执行 plan 步骤，不等 User 逐步确认，但危险操作仍需 User 审批。可直接进入，不需要先经过 Plan Mode

模式切换规则：
- Plan Mode 与 Auto Mode 是独立模式，可分别进入和退出，也可通过 `/execute` 命令切换
- Plan Mode 下 User 通过 `/execute` 命令或自然语言表达执行意图（如"执行吧""开始实行"）时，退出 Plan Mode 并进入 Auto Mode
- Auto Mode 下所有任务完成后自动退出并恢复默认模式

> **交叉引用**：模式切换指令的完整语法和参数由 F14（模式切换指令）定义。指令拦截和分派由 [slash §F1](slash.md)（斜杠指令入口）定义。

### F2. Plan Mode — 标准路径

当 User 的任务描述中含有明确的文件/模块/接口引用和可量化的验收条件时，Agent 按标准 4 阶段推进规划。

| 阶段 | Agent 做什么 | User 参与 |
|------|-------------|---------|
| Research | 并行探索代码库，理解现有实现和依赖 | 无 |
| Design | 从架构师视角生成实现方案，输出关键文件列表 | 无 |
| Review | 向 User 展示方案并澄清需求模糊点 | User 审阅方案、提出修改意见 |
| Final Plan | 将完整方案写入 plan 文件 | 无 |

阶段切换由 Agent 自行判断，阶段之间无系统强制卡点。Review 不是一次性审批——User 可以反复审阅并给出修改意见，Agent 不断调整 plan，直到 User 满意并决定执行。Research 和 Design 阶段 Agent 可以 spawn 子 Agent 并行工作；spawn 出的子 Agent 在 Plan Mode 下也只读（详见 [permission §F9](permission.md)（子 Agent 权限继承））。

> **交叉引用**：子 Agent spawn 的并发上限和创建控制详见 [agent §F9](agent.md)（Spawn 创建控制）。

### F3. Plan Mode — Interview 路径

当 User 的任务描述模糊、范围不明确、没有具体验收条件时，Agent 进入 Interview 路径。

- Agent 循环执行"探索代码 → 增量更新 plan 文件 → 向 User 提问澄清"，直到需求收敛
- plan 文件在每轮探索后增量更新，不等到最后才写
- Agent 自行判断模糊点是否消除，消除后对接标准路径的 Review 和 Final Plan 阶段


### F4. 执行触发

plan 写完后，User 可以通过以下方式触发执行：

**斜杠指令**：通过 `/execute <plan名称> [附加指令]` 命令触发执行。`plan名称` 为必选参数。`附加指令` 可选——空格后的所有内容作为一条用户消息注入 Auto Mode 的初始对话。若当前处于 Plan Mode 则先退出再进入 Auto Mode，否则直接进入 Auto Mode。

> **交叉引用**：`/execute` 的指令注册、Immediate 标记和确认豁免由 [slash §F2](slash.md)（模式切换）与 [slash §F1](slash.md)（斜杠指令入口）定义。

**自然语言**：Agent 提供执行触发工具，参数与 `/execute` 相同（`plan名称` + 可选附加指令）。该工具调用时自动向 User 发起确认（y/n）——系统直接拦截并展示确认卡片。User 确认后进入 Auto Mode 开始执行。自然语言路径需要确认，因为该方式由对话上下文触发，无法像显式命令那样排除误触。

> **交叉引用**：执行触发工具的注册通过工具扩展接入机制完成，详见 [tools §F9](tools.md)（工具扩展接入）。

**执行路径**：
- 同 session 执行：当前 session 进入 Auto Mode（若处于 Plan Mode 则先退出），继承规划上下文
- 新 session 执行：创建新 session，注入 plan 文件内容作为初始上下文，新 session 直接进入 Auto Mode

同一 plan 的并发执行不做系统级锁定，由 User 自行管理。

### F5. plan 文件

每个 plan 以独立文件持久化到工作区的 plans 目录，包含以下内容：

- 任务标题、创建和更新时间
- Context 节：背景、约束、已确认的决策
- Tasks 节：有序步骤列表，每步有完成标记
- Verification 节：端到端验证方式
- Notes 节：执行过程中的备注

plan 本身没有 draft/confirmed/completed 等 plan 级别的状态——只有步骤级别的状态。步骤状态包括：

| 状态 | 含义 |
|------|------|
| 未开始 | 步骤尚未执行 |
| 进行中 | 步骤正在执行 |
| 已完成 | 步骤成功完成 |
| 失败 | 步骤执行失败，需 User 介入 |
| 已跳过 | User 或 Agent 显式跳过 |

步骤的状态流转：未开始 → 进行中 → 已完成 / 失败 / 已跳过。失败后 User 可决定重试（失败 → 进行中）。已完成不允许回退。已完成若干步后 User 发现设计有问题，可以回 Plan Mode 修改未完成的步骤，不影响已完成步骤。

plan 文件命名需包含任务识别信息。命名格式由 User 在配置中一次性指定——时间戳格式（如 `20260718-2006-任务名`）或随机词组格式（如 `ancient-forest-mist`），创建新 plan 时自动套用所选格式。

> **交叉引用**：执行方式详见 F10。

### F6. plan 浏览与管理

User 可以随时查看和管理工作区中已有的 plan。通过 `/plans` 命令或自然语言触发。

`/plans` 不接受参数时列出所有 plan 及其步骤完成情况。`/plans <plan名称>` 查看特定 plan 的完整内容。

> **交叉引用**：`/plans` 的指令注册、Immediate 标记和分派由 [slash §F12](slash.md)（plan 浏览）管理。

通过自然语言时（如"看看有哪些 plan""废弃上次的 plan"），Agent 执行对应操作：

- 列出所有 plan 及其步骤完成情况
- 查看特定 plan 的完整内容
- 废弃不再需要的 plan

### F7. Auto Mode（执行模式）

Agent 在 Auto Mode 下以连续自主方式执行 plan 步骤。

Agent 在 Auto Mode 下的行为原则：
- 低风险操作直接执行，不等 User 逐步确认
- 常规决策自主做出，不上报给 User
- 不在执行中途主动重新进入 Plan Mode（User 在失败后显式选择回退修改 plan 的情形除外，详见 F12）
- 接受 User 随时发来的修正建议
- 删除数据、修改生产配置等危险操作必须经 User 确认
- 不擅自向外部平台发送消息

Auto Mode 下 Agent 的工具集恢复到完整状态（写工具可见）。

> **交叉引用**：运行时危险操作审查由权限系统提供，详见 [permission §F2](permission.md)（权限维度）、[permission §F5](permission.md)（审批工作流）。

### F8. 执行拒绝日志

Auto Mode 下触发审批的危险操作会生成审计日志，User 可查看。日志记录操作内容及最终处置（批准/拒绝）。

- 每条被审批的操作记录：工具名、操作描述、最终处置（批准/拒绝）、时间戳
- 按时间倒序排列
- 持久化，跨 session 可见
- User 可以配置日志存储上限

### F9. 执行进度管理

执行阶段的进度由 Agent 自行管理——Agent 在 plan 文件中以约定的格式标记步骤完成状态。系统不介入进度判断，Agent 是步骤完成与否的唯一判断者。

- Agent 按 plan 文件的 Tasks 节顺序执行步骤
- 每步完成后 Agent 在 plan 文件中更新对应步骤的状态标记。标记与 F5 的步骤状态一一对应：`[ ]` 未开始，`[-]` 进行中，`[x]` 已完成，`[!]` 失败，`[~]` 已跳过

### F10. 执行方式

plan 写完后，执行方式完全由 User 通过自然语言指令决定，没有固定的模式约束。

> **交叉引用**：同 session / 新 session 执行的定义见 [F4](#f4-执行触发)（执行触发）。

- User 可以指定在继承规划上下文的同 session 中执行，或在新 session 中从 plan 文件读取背景后执行
- User 可以指定执行全部步骤或部分步骤
- User 可以要求 Agent spawn 子 Agent 来执行特定步骤

### F11. 中断与恢复

执行过程中的中断可以无缝恢复。暂停和恢复通过自然语言触发，无专用斜杠指令——User 说"停一下""暂停"即暂停执行，说"继续""继续执行""继续 xx 步"即恢复执行。

- User 暂停执行时，Agent 停止当前步骤并将进度保存到 plan 文件
- User 恢复执行时，Agent 从暂停时的当前步骤继续，不重复已完成步骤
- 对话被压缩或重启后 Agent 仍然知道当前执行进度（当前第几步、哪些已完成、哪些未完成），不需要从第 1 步重新执行
- plan 文件本身具备独立于 session 的恢复保障——即使 session 完全丢失，仍可基于 plan 文件内容重建执行上下文

### F12. 失败处理

步骤是否失败由 Agent 自行判断，不存在系统级的重试次数限制或自动重试机制。Agent 认为某步骤失败后，User 可以自由决定下一步操作——重试、回到 Plan Mode 修改后续步骤、跳过该步骤、或放弃。

### F13. plan 归档

当 plan 的全部步骤处于终态（已完成 `[x]`、失败 `[!]` 或已跳过 `[~]`）时，若该 plan 文件最后访问时间超过配置天数，则自动归档，避免 plans 目录无限增长。User 可以配置归档天数。

### F14. 模式切换指令

User 通过以下斜杠指令查询或切换会话运行模式：

- `/plan [描述]`：切换到 Plan Mode。可选描述参数在模式切换后作为下一条用户消息注入对话——效果等价于先执行 `/plan`、再发送该描述文本。不带描述时仅切换模式
- `/mode`（无参数）：查询当前模式
- `/mode plan [描述]`：等价于 `/plan [描述]`
- `/mode normal`：切换到默认模式
- `/mode <非法值>`：提示错误，模式不变

模式切换不立即生效——切换仅标记会话状态，下一条用户消息前才应用新模式的约束。

> **交叉引用**：Plan Mode 下的 Agent 行为约束见 F2（标准路径）和 F3（Interview 路径）。Auto Mode 的行为约束见 F7。

## 关联设计文档

- [✓] mode/README.md
- [✓] mode/plan-mode.md
- [✓] mode/execution.md

## 非功能需求

- **可靠性**：Plan Mode 下的写保护对所有操作和外部输入均不可绕过，Agent 在 Plan Mode 下无法执行写操作（plan 文件除外）
- **数据持久性**：执行进度在对话被压缩、系统崩溃或重启后必须完整恢复，不能因对话清理而丢失当前步骤
- **可配置性**：审计日志上限、归档天数均支持 User 按偏好配置
