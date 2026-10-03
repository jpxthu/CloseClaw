# Execution Engine

## 概述

Execution Engine 是 workflow 的运行时核心，按定义驱动状态机转换、注入三阶段协议消息、评估跳转条件。

Agent 负责执行步骤内容，Engine 负责流程结构：追踪当前 phase，在验收判定条件满足时注入验收清单，Agent 完成验收后注入跳转问题并评估条件匹配。

## 架构

### 状态机

Engine 管理五个 phase。进入每个 phase 前 Engine 执行相应的注入动作。

1. **executing**
   Agent 正在执行步骤内容。Engine 在进入此 phase 前注入步骤目标消息，之后不干预 Agent 的工具调用。离开转换：
   - Agent turn 结束、且四维活跃维度均否（符合验收判定条件）→ verifying

2. **verifying**
   Engine 已注入验收清单，等待 Agent 响应。离开转换：
   - Agent 继续干活（调工具、spawn 子 session 等）→ 等下次验收判定条件满足再注入验收清单 → 循环回 verifying
   - Agent 完成，调用 workflow_verify → jumping
   - Agent 调用 workflow_blocked（当前步骤允许时）→ blocked
   - pending_verify 计数达到上限 → blocked

3. **jumping**
   Engine 已注入跳转问题，等待 Agent 调用 workflow_jump 回答。离开转换：
   - goto(N) → executing
   - reexecute(N) → executing
   - complete → complete

4. **blocked**
   阻塞状态，等待 Owner 介入。触发来源：
   - Agent 调用 workflow_blocked（当前步骤 allow_blocked 为 true）
   - 验收清单连续注入次数达到上限
   - 恢复时检测到当前步骤在最新定义中已不存在（定义版本变更）——该暂停仅能由 Owner 终止，不可解除续跑

   离开转换：
   - Owner 输入解除 → 解除阻塞（解除动作见「阻塞处理」）→ verifying
   - Owner 终止 workflow → complete

5. **complete**
   Workflow 执行完毕。终止状态，无离开转换。

> 各工具只在特定 phase 合法：workflow_verify 仅在 verifying、workflow_jump 仅在 jumping、workflow_blocked 仅在 verifying 且当前步骤 allow_blocked 为 true。在非允许 phase 调用返回错误、phase 不变（各工具的阶段约束见 workflow-tools.md）。

### 三阶段协议

每个步骤经过三个阶段，由 Engine 驱动转换：

**步骤目标**（executing phase）
触发：Engine 进入 executing phase 时
注入：步骤目标消息（纯文本，role: workflow）
Agent 动作：执行任务
消息抹除：否，步骤目标消息保留在上下文中

**验收**（verifying phase）
触发：验收判定条件成立（四维活跃维度均否；判定见「验收时机」）
注入：验收清单。如果当前步骤 allow_blocked 为 true，末尾附加 "如果确认任务无法继续，调用 workflow_blocked({reason: "原因"})"。
Agent 动作：自查
- 未完成 → 继续执行。Engine 等下次验收判定条件满足，注入新验收清单前先移除上一条验收清单，计数加一
- 完成 → workflow_verify()
- 无法继续且 allow_blocked → workflow_blocked()
消息抹除：Agent 调用 workflow_blocked 后即抹除本轮验收清单交互记录（注入消息 + tool_call + tool_result）；Agent 调用 workflow_verify 进入跳转时不立即抹除，待跳转决策完成后与跳转问题交互记录一并抹除（见「跳转」阶段）

**跳转**（jumping phase）
触发：Agent 调用 workflow_verify
注入：跳转问题（ABCD 选项 + 调用提示，role: workflow）
Agent 动作：workflow_jump({answers})
消息抹除：跳转决策完成后，Engine 一并抹除本轮验收清单与跳转问题的交互记录（两者的注入消息 + tool_call + tool_result）

### 验收时机（Session 活跃维度）

Engine 判定当前步骤执行是否可进入 verifying：在 Agent 当前 turn 结束后，通过 Session 暴露的四维活跃维度是否全部为否来判定可验收。验收判定条件 = 四维活跃维度全部为否——即 Session 的 llm_active、foreground_tool_active、background_tool_active、child_active 均为 false（对应需求 F3 所列 llm 推理 / 同步工具等待结果 / 后台任务 / 子 Session，定义见 [session/session-execution.md](../session/session-execution.md)（四维执行状态））。任一维度为 true 时不进入 verifying，Engine 等待下一次验收判定时机（下一次 Agent turn 结束后重新评估）。

此判定有别于 Session 的「idle（消息就绪）」判定：后者仅要求 llm_active 与 foreground_tool_active 为 false，不计入后台任务与子 Session，不用于 workflow 验收时机（详见 session-execution.md 复合状态一段）。

## 数据流

### 步骤目标阶段

1. Engine 注入步骤目标消息（role: workflow），内容为当前步骤目标描述
2. Agent 收到后连续工具调用完成步骤
3. Agent turn 结束，且四维活跃维度均否（符合验收判定条件）→ 进入验收阶段

### 验收阶段

1. Engine 注入验收清单（role: workflow）。如当前步骤 allow_blocked，末尾附加 blocked 提示
2. Agent 自查：
   - 继续干活 → Engine 等下次验收判定条件满足，注入新验收清单前先移除上一条验收清单，pending_verify 加一
   - 完成 → workflow_verify() → 进入跳转阶段（本轮验收交互记录暂留，待跳转决策完成后统一抹除）
   - 无法继续且 allow_blocked → workflow_blocked() → Engine 抹除三条消息 → 进入阻塞阶段

### 跳转阶段

1. Engine 注入跳转问题（role: workflow）。选项来自当前步骤定义中的 jump 配置，与 transitions 的 when 条件对应
2. Agent 调用 workflow_jump({answers})
3. Engine 按 transitions 顺序匹配条件，执行对应 action（goto/reexecute/complete）
4. Engine 一并抹除本轮验收清单与跳转问题的交互记录（两者的注入消息 + tool_call + tool_result），更新 WorkflowRun 状态
5. 注入下一步的步骤目标消息或结束

### 跳转评估

Engine 收到 workflow_jump({answers}) 后按 transitions 顺序匹配条件。boolean 用原生布尔值直接比对；enum 先按 options 顺序将答案字母映射回内部值，再与 expected_value 比对。第一个全部满足的 transition 生效，都不满足则执行 default。全硬编码，不依赖 LLM。

### 跳转动作

goto(N)：前进到 Step N，step_history 追加完成记录。目标 phase 为 executing。
reexecute(N)：重入 Step N，不追加完成记录，步骤目标消息注入时附加重新执行提示。目标 phase 为 executing。
complete：Workflow 结束。目标 phase 为 complete。

### 验收重试

Engine 每次注入验收清单后 pending_verify 计数加一。计数达到上限（默认 3，可在 workflow 定义中配置，每个 workflow 一个上限值）→ phase 转为 blocked 并通知 Owner；Owner 解除阻塞后计数归零（重置验收重试次数），转入 blocked 前残留的旧验收清单在 Owner 解除时一并清理。

进入新步骤时计数归零：goto 到新步骤、reexecute 重入步骤，以及 Agent 调用 workflow_verify 使当前步骤验收通过。

没有超时机制。Agent 只要还在执行步骤内容，不管多久 Engine 都等——步骤长度由任务复杂度决定，Engine 不设时间上限。

### 阻塞处理

两种阻塞来源（Agent 主动阻塞、验收重试耗尽）转入 blocked 与解除的动作一致：下面分别说明转入条件，解除动作只在「Owner 解除阻塞」一处完整定义，本模块其他文档引用此处。

**转入 blocked**

- **Agent 主动阻塞**（当前步骤 allow_blocked 为 true）：Agent 在 verifying 阶段调用 workflow_blocked({reason}) → Engine 将 phase 设为 blocked，通过 Gateway 即时通知 Owner（含暂停原因）
- **验收重试耗尽**：pending_verify 计数达到上限 → Engine 将 phase 设为 blocked，通过 Gateway 即时通知 Owner（含暂停原因）

**Owner 解除阻塞**（解除动作定义处）

Owner 回复后 Engine 保留当前步骤目标消息、pending_verify 归零、清理残留验收清单、注入验收清单 → verifying。

**Owner 终止**

phase 转为 complete，Engine 执行退出清理。

## 模块关系

- **Workflow Definition**（同模块）：提供 Step 定义——Engine 按目标、验收清单、跳转问题、跳转规则驱动执行。
- **Session Integration**（同模块）：Engine 将 WorkflowRun 写入 session checkpoint 持久化。
- **Workflow Tools**（同模块）：Engine 接收并处理 workflow_verify/jump/blocked 工具调用。
- **Session**（跨模块）：提供四维活跃维度与验收判定相关查询——Engine 据此在 Agent turn 结束后判断是否进入 verifying。
- **Gateway**（跨模块）：blocked 通知 Owner 时通过 Gateway 发送。
- **LLM Provider**（无关）：Engine 不直接调用 LLM，通过注入 workflow 消息驱动。
