# mode 需求

## 概述

Owner/User 可以切换 Agent 的运行模式来控制 Agent 的行为边界——规划时只读不执行，执行时自主推进但不越安全红线。plan 和执行是两个独立的事情：plan 写完后可以在同一 session 内执行（继承规划上下文），也可以新建 session 执行（从 plan 文件读取背景）。

## 功能需求

### F1. 运行模式

Agent 在 session 内可运行于以下模式之一，每种模式决定了 Agent 可用的工具集、行为约束和权限边界。

- 默认模式：Agent 按完整配置运行，全工具集可用，无额外行为约束
- Plan Mode：Agent 只做规划不做执行，工具集受限为只读（仅 plan 文件可写）。User 可以反复要求 Agent 修改 plan，Plan Mode 没有审批栅栏——User 说"执行"时才退出
- Auto Mode（执行模式）：Agent 连续自主执行 plan 步骤，不等 User 逐步确认，但危险操作仍需 User 审批。可直接进入，不需要先经过 Plan Mode

模式切换规则：
- User 通过斜杠指令显式进入 Plan Mode 或 Auto Mode
- Plan Mode 与 Auto Mode 相互独立——从 Plan Mode 退出不自动进入 Auto Mode，进入 Auto Mode 也不需要先经过 Plan Mode
- Plan Mode 下 User 通过斜杠指令或自然语言触发执行时，退出 Plan Mode 并进入 Auto Mode
- Auto Mode 下所有任务完成后自动退出并恢复默认模式

### F2. Plan Mode — 标准路径

当 User 的任务描述中含有明确的文件/模块/接口引用和可量化的验收条件时，Agent 按标准 4 阶段推进规划。

| 阶段 | Agent 做什么 | User 参与 |
|------|-------------|---------|
| Research | 并行探索代码库，理解现有实现和依赖 | 无 |
| Design | 从架构师视角生成实现方案，输出关键文件列表 | 无 |
| Review | 向 User 展示方案并澄清需求模糊点 | User 审阅方案、提出修改意见 |
| Final Plan | 将完整方案写入 plan 文件 | 无 |

阶段切换由 Agent 自行判断，阶段之间无系统卡点。Review 不是一次性审批——User 可以反复审阅并给出修改意见，Agent 不断调整 plan，直到 User 满意并决定执行。Research 和 Design 阶段 Agent 可以 spawn 子 Agent 并行工作；spawn 出的子 Agent 在 Plan Mode 下也只读（权限沿 spawn 链路收窄，详见 [permission §F9](permission.md)（子 Agent 权限继承））。

> **交叉引用**：子 Agent spawn 的并发上限和创建控制详见 [agent §F9](agent.md)（Spawn 创建控制）。

### F3. Plan Mode — Interview 路径

当 User 的任务描述模糊、范围不明确、没有具体验收条件时，Agent 进入 Interview 路径。

- Agent 循环执行"探索代码 → 增量更新 plan 文件 → 向 User 提问澄清"，直到需求收敛
- plan 文件在每轮探索后增量更新，不等到最后才写
- Agent 自行判断模糊点是否消除，消除后对接标准路径的 Review 和 Final Plan 阶段
- User 可以配置 Interview 最大轮数，达到上限后系统退出循环并提示 User 决定：继续澄清、放弃规划、或直接写入当前 plan；未设置上限时无硬性限制

### F4. 执行触发

plan 写完后，User 可以通过以下方式触发执行：

**斜杠指令**：通过 `/execute` 命令触发执行。

> **交叉引用**：`/execute` 命令的完整语法和参数由 [slash](slash.md) 模块定义。执行触发的业务行为（模式切换、上下文注入、执行路径选择）由本模块定义。

**自然语言**：
- Agent 调用执行触发工具，参数同斜杠指令
- 该工具自动发起用户确认交互（支持交互的通道弹出确认卡片，不支持的通道以自然语言让 User 回复 y/n 确认），确认后进入 Auto Mode 开始执行

**执行路径**：
- 同 session 执行：当前 session 进入 Auto Mode（若处于 Plan Mode 则先退出），继承规划上下文
- 新 session 执行：创建新 session，注入 plan 文件内容作为初始上下文，新 session 直接进入 Auto Mode

同一 plan 的并发执行不做系统级锁定，由 User 自行管理。

> **交叉引用**：执行触发命令入口由 [slash](slash.md) 模块定义。

### F5. plan 文件

每个 plan 以独立文件持久化到工作区的 plans 目录，包含以下内容：

- 任务标题、创建和更新时间
- Context 节：背景、约束、已确认的决策
- Tasks 节：有序步骤列表，每步有完成标记
- Verification 节：端到端验证方式
- Notes 节：执行过程中的备注

plan 本身没有 draft/confirmed/completed 等全局状态——只有步骤级别的状态。步骤状态包括：

| 状态 | 含义 |
|------|------|
| 未开始 | 步骤尚未执行 |
| 进行中 | 步骤正在执行 |
| 已完成 | 步骤成功完成 |
| 失败 | 步骤执行失败，需 User 介入 |
| 已跳过 | User 或 Agent 显式跳过 |

步骤的状态流转：未开始 → 进行中 → 已完成 / 失败 / 已跳过。失败后 User 可决定重试（失败 → 进行中）。已完成不允许回退。已完成若干步后 User 发现设计有问题，可以回 Plan Mode 修改未完成的步骤，不影响已完成步骤。

plan 文件命名需包含任务识别信息，格式由 User 在时间戳格式（如 `20260717-0153-任务名`）和随机词组格式间选择。

> **交叉引用**：执行模式（Inline/Spawn）详见 F10。

### F6. plan 浏览与管理

User 可以随时查看和管理工作区中已有的 plan。

- 列出所有 plan 及其步骤完成情况
- 查看特定 plan 的完整内容
- 废弃不再需要的 plan

> **交叉引用**：plan 浏览与管理入口命令由 [slash](slash.md) 模块定义。

### F7. Auto Mode（执行模式）

Agent 在 Auto Mode 下以连续自主方式执行 plan 步骤。

Agent 在 Auto Mode 下的行为原则：
- 低风险操作直接执行，不等 User 逐步确认
- 常规决策自主做出，不升级给 User
- 不在执行中途主动重新进入 Plan Mode（User 在失败后显式选择回退修改 plan 的情形除外，详见 F12）
- 接受 User 随时发来的修正建议
- 删除数据、修改生产配置等危险操作必须经 User 确认
- 不擅自向外部平台发送消息

Auto Mode 下 Agent 的工具集恢复到完整状态（写工具可见）。

> **交叉引用**：运行时危险操作审查由权限系统提供，详见 [permission §F2](permission.md)（权限维度）、[permission §F5](permission.md)（审批工作流）。

### F8. 执行拒绝日志

Auto Mode 下被拦截的危险操作会生成拒绝日志，User 可查看。

- 每条被拒绝的操作记录：工具名、操作描述、拒绝原因、时间戳
- 按时间倒序排列
- 持久化，跨 session 可见
- User 可以配置日志存储上限

### F9. 执行进度管理

执行阶段的每步完成由系统自动追踪，Agent 不需要手动修改 plan 文件的 checkbox。

- 步骤按顺序逐一推进——当前一步成功完成前，后续步骤不能被执行
- 已完成的步骤不会意外回退
- plan 文件中的完成标记随进度自动同步（步骤状态变更时立即同步，确保中断不丢失最新进度）
- 步骤有明确的生命周期：未开始 → 进行中 → 已完成 / 失败 / 已跳过（失败为终结态，到达后停止执行等待 User 决策）

### F10. 执行模式

User 可以配置执行策略来控制任务如何被执行。

- Inline 模式（默认）：当前 session 直接执行全部步骤，上下文连续
- Spawn per_step 模式：每个步骤由独立的子 Agent 执行，步骤间上下文隔离，单步失败不影响已完成步骤
- Spawn all_steps 模式：一个子 Agent 执行全部步骤，适合步骤间有强依赖的场景

### F11. 中断与恢复

执行过程中的中断可以无缝恢复。

- User 可以随时暂停执行，当前进度被保存
- User 恢复执行时，Agent 从暂停时的当前步骤继续，不重复已完成步骤
- session 压缩或重启后 Agent 仍然知道当前执行进度（当前第几步、哪些已完成、哪些待继续），不需要从第 1 步重新执行
- plan 文件本身具备独立于 session 的恢复保障——即使 session 完全丢失，仍可基于 plan 文件内容重建执行上下文

> **交叉引用**：暂停/恢复执行的命令入口由 [slash](slash.md) 模块定义。

### F12. 失败处理

步骤执行失败时，系统暂停后续步骤，通知 User 失败步骤和原因。

- User 可以决定：重试当前步骤、回到 Plan Mode 修改 plan（追加新步骤，不修改已完成的步骤）、跳过该步骤、放弃整个 plan
- User 可以配置单步最大重试次数和重试方式

### F13. 步骤完成回调

每步完成后系统自动触发后续操作，Agent 不需要自己记得"该做下一步了"。

- 验证回调：涉及多文件修改或有外部影响的步骤完成后，以独立视角审视实现结果（以发现缺陷为目标，而非仅确认"能跑通"），输出 PASS / FAIL / PARTIAL 裁决
- 通知回调：向 User 发送进度更新
- 自定义回调：User 可以配置自定义命令在步骤完成时执行

User 可以配置每个回调类型的触发条件。验证回调的默认行为是"涉及多文件/外部影响的步骤自动触发"，User 可改为始终触发或不触发。

### F14. plan 归档

已完成的 plan 文件在最后访问超过一定天数后自动归档，避免 plans 目录无限增长。User 可以配置归档天数。

## 关联设计文档

- [✓] mode/README.md
- [✓] mode/plan-mode.md
- [✓] mode/execution.md

## 非功能需求

- **可靠性**：Plan Mode 下的写保护不能以任何方式被绕过——无论通过 prompt 注入、工具欺骗还是上下文操纵，Agent 在 Plan Mode 下都无法执行写操作（plan 文件除外）
- **数据持久性**：执行进度在 session 压缩、崩溃、重启后必须完整恢复，不能因上下文清理而丢失当前步骤
- **可配置性**：执行模式、重试策略、验证触发条件、回调触发条件、Interview 最大轮数、拒绝日志上限、归档天数均支持 User 按偏好配置
