# Execution Engine

## 概述

Execution Engine 是 workflow 的运行时核心，按定义驱动状态机转换、注入三阶段协议消息、评估跳转条件。

Agent 负责执行步骤内容，Engine 负责流程结构：追踪当前 phase，在验收判定条件满足时注入验收清单，Agent 完成验收后注入跳转问题并评估条件匹配；同时管理 blocked 暂停（含定义版本变更的终止性暂停）与恢复时的状态重建。

## 架构

### 状态机

Engine 管理五个 phase。进入需要注入的 phase 时，Engine 执行对应注入动作。

1. **executing**
   Agent 正在执行步骤内容。Engine 在进入此 phase 时注入步骤目标消息，之后不干预 Agent 的工具调用。离开转换：
   - Agent turn 结束、且四维活跃维度均否（符合验收判定条件）→ verifying

2. **verifying**
   Engine 已注入验收清单，等待 Agent 响应。离开转换：
   - Agent 继续干活（调工具、spawn 子 session 等）→ phase 保持 verifying，等下次验收判定条件满足时移除上一条验收清单后重新注入
   - Agent 完成，调用 workflow_verify → jumping
   - Agent 调用 workflow_blocked（当前步骤允许时）→ blocked
   - pending_verify 计数达到上限 → blocked

3. **jumping**
   Engine 已注入跳转问题，等待 Agent 调用 workflow_jump 回答。离开转换：
   - goto(N) → executing
   - reexecute(N) → executing
   - complete → complete

4. **blocked**
   阻塞状态，等待 Owner 介入。按可否解除分为两类：
   - **可解除暂停**：Agent 调用 workflow_blocked（当前步骤 allow_blocked 为 true）；验收清单连续注入次数达到上限
   - **终止性暂停**：Session 重启或归档恢复时检测到当前步骤在最新定义中已不存在（定义版本变更）——仅能由 Owner 终止，不可解除

   离开转换：
   - Owner 输入非斜杠消息（仅可解除暂停）→ 解除阻塞（解除动作见「阻塞处理」）→ verifying
   - Owner 输入 `/stop`（任意 blocked）→ complete（Owner 终止，执行退出清理）

5. **complete**
   Workflow 执行完毕。终止状态，无离开转换。

> 各工具只在特定 phase 合法：workflow_verify 仅在 verifying、workflow_jump 仅在 jumping、workflow_blocked 仅在 verifying 且当前步骤 allow_blocked 为 true。在非允许 phase 调用返回错误、phase 不变（各工具的阶段约束见 workflow-tools.md）。
>
> Owner `/stop` 可从任意 phase 直接进入 complete（见「Owner 终止」），各 phase 的离开转换不再重复列出；恢复期检测到定义版本变更时，运行中的任意 phase 转为 blocked（终止性暂停）。

### 三阶段协议

每个步骤经过三个阶段，由 Engine 驱动转换。三阶段注入的 workflow 控制消息均经 session 统一消息队列注入（role: workflow，now 优先级），不经入站 Processor Chain，也不进出站渲染链路。

**步骤目标**（executing phase）
触发：Engine 进入 executing phase 时
注入：步骤目标消息（纯文本，role: workflow）
Agent 动作：执行任务
消息抹除：否，步骤目标消息保留在上下文中

**验收**（verifying phase）
触发：验收判定条件成立（四维活跃维度均否；判定见「验收时机」）；Owner 解除可解除暂停时也会注入验收清单，不经验收判定条件（见「阻塞处理」）
注入：验收清单。如果当前步骤 allow_blocked 为 true，末尾附加 "如果确认任务无法继续，调用 workflow_blocked({reason: "原因"})"。
Agent 动作：自查。Engine 不校验验收条目真伪，workflow_verify 只是完成信号（见 workflow-tools.md）。
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

Engine 判定当前步骤执行是否可进入 verifying：在 Agent 当前 turn 结束后，通过 Session 暴露的四维活跃维度是否全部为否来判定可验收。验收判定条件 = 四维活跃维度全部为否——即 Session 的 llm_active、foreground_tool_active、background_tool_active、child_active 均为 false（对应需求 F3 所列 llm 推理 / 同步工具等待结果 / 后台任务 / 子 Session，定义见 [session-execution.md](../session/session-execution.md)（四维执行状态））。任一维度为 true 时不进入 verifying，Engine 等待下一次验收判定时机（下一次 Agent turn 结束后重新评估）。

此判定有别于 Session 的「idle（消息就绪）」判定：后者仅要求 llm_active 与 foreground_tool_active 为 false，不计入后台任务与子 Session，不用于 workflow 验收时机（详见 session-execution.md 复合状态一段）。

Owner 输入 `/stop` 造成的四维归零不触发验收：`/stop` 走 Owner 终止路径（见「Owner 终止」），Engine 不注入验收清单。

## 数据流

### 步骤目标阶段

1. Engine 经统一消息队列注入步骤目标消息（role: workflow），内容为当前步骤目标描述
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

Engine 收到 workflow_jump({answers}) 后按 transitions 顺序匹配条件。Engine 先按 options 顺序将答案字母映射回内部值，再与 expected_value 比对；第一个全部满足的 transition 生效，都不满足则执行 default。全硬编码，不依赖 LLM。

### 跳转动作

goto(N)：前进到 Step N，step_history 追加完成记录。目标 phase 为 executing。
reexecute(N)：重入 Step N，不追加完成记录，步骤目标消息注入时附加重新执行提示。目标 phase 为 executing。
complete：Workflow 结束。目标 phase 为 complete。

### 验收重试

Engine 每次注入验收清单后 pending_verify 计数加一。计数达到上限（默认 3，可在 workflow 定义中配置，每个 workflow 一个上限值）→ phase 转为 blocked 并通知 Owner；Owner 解除阻塞后计数归零（重置验收重试次数），转入 blocked 前残留的旧验收清单在 Owner 解除时一并清理。

以下事件使计数归零：goto 到新步骤、reexecute 重入步骤、Agent 调用 workflow_verify 使当前步骤验收通过、Owner 解除可解除暂停。

没有超时机制。Agent 只要还在执行步骤内容，不管多久 Engine 都等——步骤长度由任务复杂度决定，Engine 不设时间上限。

### 阻塞处理

可解除暂停有两种来源（Agent 主动阻塞、验收重试耗尽），转入 blocked 与解除的动作一致：下面分别说明转入条件，解除动作只在「Owner 解除阻塞」一处完整定义，本模块其他文档引用此处。定义版本变更产生的是终止性暂停，不可解除，单独定义如下。

**转入 blocked**

- **Agent 主动阻塞**（当前步骤 allow_blocked 为 true）：Agent 在 verifying 阶段调用 workflow_blocked({reason}) → Engine 将 phase 设为 blocked，通过 Gateway 系统通知接口即时向 Owner 推送 IM 通知（含暂停原因）
- **验收重试耗尽**：pending_verify 计数达到上限 → Engine 将 phase 设为 blocked，通过 Gateway 系统通知接口即时向 Owner 推送 IM 通知（含暂停原因）

**终止性暂停（定义版本变更）**

Session 重启或归档恢复时检测到当前步骤在最新定义中已不存在 → phase 置为 blocked，暂停原因置为「当前步骤在最新定义中已不存在」并通知 Owner。该暂停仅接受 Owner `/stop`；Owner 非斜杠消息不解除阻塞，Engine 经 Gateway 系统通知接口提示仅能由 `/stop` 终止。暂停原因取值见 [session-integration.md](session-integration.md)「持久化数据」。

**Owner 解除阻塞**（解除动作定义处）

仅适用于可解除暂停。Owner 发送非斜杠消息后，Gateway 在消息路由到 Session 前经 [WorkflowControl](../common/core-traits.md#workflowcontrol) 通知 Engine。Engine 保留当前步骤目标消息（若定义变更导致步骤目标更新，按最新定义替换步骤目标消息）、pending_verify 归零、清理残留验收清单、注入验收清单 → verifying。斜杠指令不解除阻塞；其中 `/stop` 按「Owner 终止」处理。终止性暂停不适用本动作（见上）。

**Owner 终止**

Owner 输入 `/stop`（运行中或暂停中均可）时，Gateway 先按 `/stop` 语义强制停止运行（停止入口见 [session-execution.md](../session/session-execution.md) 停止入口节），再经 [WorkflowControl](../common/core-traits.md#workflowcontrol) 通知 Engine 终止。Engine 将 phase 置为 complete，执行退出清理（移除 workflow context、清理控制消息、清空 WorkflowRun、持久化）。非 Owner 的 `/stop` 只停止运行、不终止 workflow；其他停止入口（父 Session 停止、系统关闭）同样只停止运行、不改变 workflow 运行状态，按需求 F7 恢复。

## 模块关系

- **Workflow Definition**（同模块子功能）：提供 Step 定义——Engine 按目标、验收清单、跳转问题、跳转规则驱动执行。
- **Session Integration**（同模块子功能）：Engine 将 WorkflowRun 写入 session checkpoint 持久化。
- **Workflow Tools**（同模块子功能）：Engine 接收并处理 workflow_start（进入模式）与 workflow_verify/jump/blocked 工具调用。
- **Session**（跨模块）：提供四维活跃维度与验收判定相关查询——Engine 据此在 Agent turn 结束后判断是否进入 verifying。
- **Gateway**（跨模块）：blocked 通知 Owner 时经 [Gateway 系统通知接口](../gateway/outbound-flow.md#系统通知接口)推送 IM 通知；Gateway 在 Owner 非斜杠消息路由前与 Owner `/stop` 停止完成后经 [WorkflowControl](../common/core-traits.md#workflowcontrol) 通知 Engine（解除阻塞 / 终止）。
- **LLM Provider**（无关）：Engine 不直接调用 LLM，通过注入 workflow 消息驱动。

### 共享类型 / 核心 trait

- 实现 [WorkflowControl](../common/core-traits.md#workflowcontrol)：供 Gateway 在 WorkflowStart、Owner 非斜杠消息与 Owner `/stop` 入口调用。
- 消费 [SessionLookup](../common/core-traits.md#sessionlookup)：向统一消息队列注入 workflow 控制消息（role: workflow）。
- 消费 [SlashSessionQuery](../common/core-traits.md#slashsessionquery)：查询/持久化 WorkflowRun 与当前活跃 workflow phase。

