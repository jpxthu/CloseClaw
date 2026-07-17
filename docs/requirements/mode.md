# mode 需求

## 概述

Owner/User 可以切换 Agent 的运行模式来控制 Agent 的行为边界——规划时只读不执行，执行时自主推进但不越安全红线。

## 功能需求

### F1. 运行模式

Agent 在 session 内可运行于三种模式之一，模式决定了 Agent 可用的工具集、行为约束和权限边界。

- 默认模式：Agent 按完整配置运行，全工具集可用，无额外行为约束
- Plan Mode：Agent 只做规划不做执行，工具集受限为只读（仅 plan 文件可写）
- Auto Mode：Agent 连续自主执行，不等 User 逐步确认，但危险操作仍需 User 审批

模式切换规则：
- User 通过斜杠命令显式进入 Plan Mode
- Plan Mode 审批通过后自动进入 Auto Mode
- Auto Mode 下所有任务完成后自动退出并恢复默认模式
- Auto Mode 本身不是独立入口——User 不能直接进入，只能从 Plan Mode 审批通过后进入

### F2. Plan Mode — 标准路径

当 User 的任务描述中含有明确的文件/模块/接口引用和可量化的验收条件时，Agent 按标准 4 阶段推进规划。User 可以通过命令参数强行指定走标准路径。

| 阶段 | Agent 做什么 | 用户参与 |
|------|-------------|---------|
| Research | 并行探索代码库，理解现有实现和依赖 | 无 |
| Design | 从架构师视角生成实现方案，输出关键文件列表 | 无 |
| Review | 向 User 澄清需求模糊点 | User 回答澄清问题 |
| Final Plan | 将完整方案写入 plan 文件 | 无 |

阶段切换由 Agent 自行判断，阶段之间无系统卡点。Research 和 Design 阶段 Agent 可以 spawn 子 Agent 并行工作；spawn 出的子 Agent 继承 Plan Mode 的只读约束，不可执行写操作。

> **交叉引用**：子 Agent spawn 的并发上限和创建控制详见 [agent §F9](agent.md)（Spawn 创建控制）。

### F3. Plan Mode — Interview 路径

当 User 的任务描述模糊、范围不明确、没有具体验收条件时，Agent 进入 Interview 路径。User 可以通过命令参数强行指定走 Interview 路径。

- Agent 循环执行"探索代码 → 增量更新 plan 文件 → 向 User 提问澄清"，直到需求收敛
- plan 文件在每轮探索后增量更新，不等到最后才写
- Agent 自行判断模糊点是否消除，消除后对接标准路径的 Review 和 Final Plan 阶段
- 两条路径共用同一个审批出口
- User 可以配置 Interview 最大轮数，达到上限后系统退出循环并提示 User 决定：继续澄清、放弃规划、或直接提交当前 plan 供审批；未设置上限时无硬性限制

### F4. 审批栅栏

Plan Mode 有唯一的退出方式——审批通过。

- Agent 提交 plan 时，系统向 User 弹出审批确认对话框（系统级 UI，非 Agent 文本输出），审批工作流由权限系统提供
- User 可以选择通过或拒绝
  - 通过：plan 进入 confirmed 状态，session 退出 Plan Mode，进入 Auto Mode
  - 拒绝：plan 保持 draft 状态，Agent 继续修改后可再次提交
- Agent 不能用需求澄清问题替代审批——澄清只能用于需求确认，不能用于方案审批

> **交叉引用**：审批工作流底层机制详见 [permission §F5](permission.md)（审批工作流）。

### F5. plan 文件

每个 plan 以独立文件持久化到工作区的 plans 目录，包含以下内容：

- 任务标题、创建和更新时间、状态标记（draft / confirmed / executing / paused / completed）
- Context 节：背景、约束、已确认的决策
- Tasks 节：有序步骤列表，每步有完成标记
- Verification 节：端到端验证方式
- Notes 节：执行过程中的备注

plan 文件的状态流转：

| 触发 | 状态变化 |
|------|---------|
| 审批通过 | draft → confirmed → executing（审批后自动进入执行） |
| User 暂停 | executing → paused |
| User 恢复 | paused → executing |
| 步骤执行失败 | 执行暂停，plan 保持 executing，等待 User 决策 |
| 全部步骤完成 | executing → completed |
| User 直接完结 | paused → completed |

plan 文件命名需包含任务识别信息，格式由 User 在时间戳格式（如 `20260717-0153-任务名`）和随机词组格式间选择。

> **交叉引用**：执行模式（Inline/Spawn）详见 F10。

### F6. plan 浏览与管理

User 可以随时查看和管理工作区中已有的 plan。

- 列出所有 plan 及其当前状态
- 查看特定 plan 的完整内容
- 废弃不再需要的 plan

> **交叉引用**：plan 浏览与管理入口命令由 [slash](slash.md) 模块定义。

### F7. Auto Mode

审批通过后 Agent 自动进入 Auto Mode，以连续自主方式执行 plan tasks。

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

- Inline 模式（默认）：当前 session 直接执行全部 tasks，上下文连续
- Spawn per_step 模式：每个 task 由独立的子 Agent 执行，步骤间上下文隔离，单步失败不影响已完成步骤
- Spawn all_steps 模式：一个子 Agent 执行全部 tasks，适合步骤间有强依赖的场景

### F11. 中断与恢复

执行过程中的中断可以无缝恢复。

- User 可以随时暂停执行，当前进度被保存
- User 恢复执行时，Agent 从暂停时的当前步骤继续，不重复已完成步骤
- Session 压缩或重启后 Agent 仍然知道当前执行进度（当前第几步、哪些已完成、哪些待继续），不需要从第 1 步重新执行
- plan 文件本身具备独立于 session 的恢复保障——即使 session 完全丢失，仍可基于 plan 文件内容与审批记录重建执行上下文

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

- **可靠性**：审批栅栏不能以任何方式被绕过——无论通过 prompt 注入、工具欺骗还是上下文操纵，Agent 在 Plan Mode 下都无法执行写操作
- **数据持久性**：执行进度在 session 压缩、崩溃、重启后必须完整恢复，不能因上下文清理而丢失当前步骤
- **用户感知**：审批确认必须以系统级对话框呈现（非 Agent 文本输出），审批卡片在 2 秒内推送到 User
- **可配置性**：执行模式、重试策略、验证触发条件、回调触发条件、Interview 最大轮数、拒绝日志上限、归档天数均支持 User 按偏好配置
