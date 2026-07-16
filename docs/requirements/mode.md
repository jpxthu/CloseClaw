# mode 需求

## 概述

Owner/User 可以切换 Agent 的运行模式来控制 Agent 的行为边界——规划时只读不执行，执行时自主推进但不越安全红线。

## 功能需求

### F1. 运行模式

Agent 的 session 支持三种运行模式，模式决定了 Agent 的工具可用性、行为指令和权限边界。

- 默认模式：Agent 按完整配置运行，全工具集可用，无额外行为约束
- Plan Mode：Agent 只做规划不做执行，工具集受限为只读（仅 plan 文件可写）
- Auto Mode：Agent 连续自主执行，不等用户逐步确认，但危险操作仍需用户审批

模式切换规则：
- User 通过斜杠命令显式进入 Plan Mode
- Plan Mode 审批通过后自动进入 Auto Mode
- Auto Mode 下所有任务完成后自动退出并恢复默认模式
- Auto Mode 本身不是独立入口——User 不能直接 /auto 进入，只能从 Plan Mode 审批通过后进入

### F2. Plan Mode — 标准路径

当 User 的任务描述中含有明确的文件/模块/接口引用和可量化的验收条件时，Agent 按标准 4 阶段推进规划。User 可以通过命令参数显式强制走标准路径，此时系统跳过自动判断直接采用标准路径。

| 阶段 | Agent 做什么 | 用户参与 |
|------|-------------|---------|
| Research | 并行探索代码库，理解现有实现和依赖 | 无 |
| Design | 从架构师视角生成实现方案，输出关键文件列表 | 无 |
| Review | 向 User 澄清需求模糊点 | User 回答澄清问题 |
| Final Plan | 将完整方案写入 plan 文件 | 无 |

阶段切换由 Agent 自行判断，User 不干预。阶段之间无代码层硬性校验。Research 和 Design 阶段 Agent 可以 spawn 子 Agent 并行工作，有并发上限约束（User 可配置上限值）。

### F3. Plan Mode — Interview 路径

当 User 的任务描述模糊、范围不明确、没有具体验收条件时，Agent 进入 Interview 路径。User 可以通过命令参数显式强制走 Interview 路径，此时系统跳过自动判断直接采用 Interview 路径。

- Agent 循环执行"探索代码 → 增量更新 plan 文件 → 向 User 提问澄清"，直到需求收敛
- Plan 文件在每轮探索后增量更新，不等到最后才写
- Agent 自行判断 ambiguities 是否消除，消除后对接标准路径的 Review 和 Final Plan 阶段
- 两条路径共用同一个审批出口
- User 可以配置最大 Interview 轮数上限，达到上限后框架强制退出循环并提示 User 确认是否继续审批；未设置上限时无硬性限制

### F4. 审批栅栏

Plan Mode 有唯一的退出方式——审批通过。

- Agent 提交 plan 时，框架向 User 弹出审批确认对话框（系统级，非文本"请审批"）
- User 可以选择通过或拒绝
  - 通过：plan 进入 confirmed 状态，session 退出 Plan Mode，进入 Auto Mode
  - 拒绝：plan 保持 draft 状态，Agent 继续修改后可再次提交
- Agent 不能用需求澄清问题替代审批——AskUserQuestion 只能用于需求澄清，不能问"方案行不行"

### F5. Plan 文件

每个 plan 以独立文件持久化到 workspace 的 plans 目录，包含以下内容：

- 任务标题、创建和更新时间、状态标记（draft / confirmed / executing / paused / completed）
- Context 节：背景、约束、已确认的决策
- Tasks 节：有序步骤列表，每步有完成标记
- Verification 节：端到端验证方式
- Notes 节：执行过程中的备注

Plan 文件的状态流转：
- draft → confirmed（审批通过）
- confirmed → executing（User 触发执行）
- executing → paused（User 暂停）/ completed（全部完成）
- paused → executing（User 恢复）/ completed

Plan 文件命名需直观可读，支持按时间戳或随机词组两种格式。

### F6. Plan 浏览与管理

User 可以随时查看和管理工作区中已有的 plan。

- 列出所有 plan 及其当前状态（如 `/plans` 命令）
- 查看特定 plan 的完整内容（如 `/plan <slug>` 命令）
- 废弃不再需要的 plan（如 `/delete <plan-file>` 命令）

### F7. Auto Mode

审批通过后 Agent 自动进入 Auto Mode，以连续自主方式执行 plan tasks。

Agent 在 Auto Mode 下的行为原则：
- 低风险操作直接执行，不等 User 逐步确认
- 常规决策自主做出，不升级给 User
- 不在执行中途重新进入规划模式
- 接受 User 随时发来的修正建议
- 删除数据、修改生产配置等危险操作必须经 User 确认
- 不擅自向外部平台发送消息

Auto Mode 下 Agent 的工具集恢复到完整状态（写工具可见），但危险操作受运行时审查。

### F8. 执行拒绝日志

Auto Mode 下被拦截的危险操作形成可查看的拒绝日志，确保 User 对安全事件的可见性。

- 每条被拒绝的操作记录：工具名、操作描述、拒绝原因、时间戳
- User 可以查看按时间倒序排列的拒绝日志
- 日志持久化，跨 session 可见
- User 可以配置日志存储上限

### F9. 执行进度管理

执行阶段的进度由框架管理，而非 Agent 手动修改 plan 文件中的 checkbox。

- Agent 通过进度工具上报步骤状态变化（开始执行 / 完成 / 失败 / 跳过）
- 框架校验状态流转的合法性：不能跳步（前一步未完成则拒绝标记下一步）、不能回退（已完成不能改回进行中）
- 框架同步更新 plan 文件中的 checkbox，保持文件视图与内部状态一致
- 步骤有明确生命周期：pending → in_progress → completed / failed / skipped

### F10. 执行模式

User 可以配置执行策略来控制任务如何被执行。

- Inline 模式（默认）：主 session 直接执行全部 tasks，上下文连续
- Spawn per_step 模式：每个 task 由独立的子 Agent 执行，步骤间上下文隔离，单步失败不影响已完成步骤
- Spawn all_steps 模式：一个子 Agent 执行全部 tasks，适合步骤间有强依赖的场景

### F11. 中断与恢复

执行过程中的中断可以无缝恢复。

- User 可以随时暂停执行（如 `/pause` 命令），当前进度被保存
- User 恢复执行（如 `/continue` 命令）时，Agent 从暂停时的当前步骤继续，不重复已完成步骤
- Session 压缩不会丢失执行进度——进度状态被独立保护，不经过 LLM 总结
- Session 续活时，Agent 在上下文中看到当前执行位置（当前第几步、哪些已完成、哪些待执行）
- Plan 内容在 session 崩溃或持久化异常时有多条恢复路径保障，按优先级依次：框架维护的持久化进度状态 → 磁盘上的 plan 文件 → 审批工具调用记录 → 消息历史中的 plan 引用

### F12. 失败处理

步骤执行失败时执行流程暂停，由 User 决定后续处理。

- 失败步骤及其原因被清晰通知给 User
- User 可以决定：重试当前步骤、回到 Plan Mode 修改 plan（追加新步骤，不修改已完成的步骤）、跳过该步骤、放弃整个 plan
- User 可以配置单步最大重试次数和重试方式

### F13. 步骤完成回调

每步完成后系统自动触发后续操作，Agent 不需要自己记得"该做下一步了"。

- 验证回调：非平凡任务完成后自动触发独立验证（详见 F12 行为描述）。验证 Agent 以独立视角审视实现结果，输出 PASS / FAIL / PARTIAL 裁决
- 通知回调：向 User 发送进度更新
- 自定义回调：User 可以配置自定义脚本在步骤完成时执行

User 可以配置每个回调类型的触发条件：非平凡任务触发 / 始终触发 / 不触发。

### F14. Plan 归档

已完成的 plan 文件在最后访问超过一定天数后自动归档，避免 plans 目录无限增长。User 可以配置归档天数。

## 关联设计文档

- [✓] mode/README.md
- [✓] mode/plan-mode.md
- [✓] mode/execution.md

## 非功能需求

- **可靠性**：审批栅栏必须由代码层保证，不能依赖 prompt 约束——Plan Mode 下的写操作应被权限系统硬拦截，Agent 无法绕过
- **数据持久性**：执行进度在 session 压缩、崩溃、续活后必须完整恢复，不依赖 Agent 记忆或上下文完整性
- **用户感知**：审批确认必须以系统级对话框呈现（非 Agent 文本输出），确保 User 明确感知到审批动作
- **可配置性**：执行模式、重试策略、验证触发条件、回调触发条件、Interview 最大轮数、拒绝日志上限、归档天数均支持 User 按偏好配置
