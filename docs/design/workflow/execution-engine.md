# Execution Engine

## 概述

Execution Engine 是 workflow 的运行时核心，按定义驱动状态机转换、注入三阶段协议消息、评估跳转条件。

Agent 负责执行步骤内容，Engine 负责流程结构：追踪当前 phase，在 session idle 时注入验收清单，Agent 完成验证后注入跳转问题并评估条件匹配。

## 架构

### 状态机

Engine 管理五个 phase。进入每个 phase 前 Engine 执行相应的注入动作。

1. **executing**
   Agent 正在执行步骤内容。Engine 在进入此 phase 前注入 goal 消息，之后不干预 Agent 的工具调用。离开转换：
   - Agent turn 结束、session idle → verifying

2. **verifying**
   Engine 已注入验收清单，等待 Agent 响应。离开转换：
   - Agent 继续干活（调工具、spawn 子 session 等）→ 等下次 idle 重新注入 verify → 循环回 verifying
   - Agent 完成，调用 workflow_verify → jumping
   - Agent 调用 workflow_blocked（当前步骤允许时）→ blocked
   - pending_verify 计数超过上限 → blocked

3. **jumping**
   Engine 已注入跳转问题，等待 Agent 调用 workflow_jump 回答。离开转换：
   - goto(N) → executing
   - reexecute(N) → executing
   - complete → complete

4. **blocked**
   阻塞状态，等待 owner 介入。触发来源：
   - Agent 调用 workflow_blocked（当前步骤 allow_blocked 为 true）
   - verify 连续注入次数超过上限

   离开转换：
   - Owner 输入解除 → Engine 移除旧 goal，pending_verify 归零，清理残留 verify 消息，立即注入 verify → verifying
   - Owner 终止 workflow → complete

5. **complete**
   Workflow 执行完毕。终止状态，无离开转换。

### 三阶段协议

每个步骤经过三个阶段，由 Engine 驱动转换：

**Goal**（executing phase）
触发：Engine 进入 executing phase 时
注入：步骤目标描述（纯文本，role: workflow）
Agent 动作：执行任务
消息抹除：否，goal 消息保留在上下文中

**Verify**（verifying phase）
触发：session idle（判定逻辑见 Session 模块）
注入：验收清单。如果当前步骤 allow_blocked 为 true，末尾附加 "如果确认任务无法继续，调用 workflow_blocked({reason: "原因"})"。
Agent 动作：自查
- 未完成 → 继续执行。Engine 等下次 idle，注入新 verify 前先移除上一条 verify 消息，计数加一
- 完成 → workflow_verify()
- 无法继续且 allow_blocked → workflow_blocked()
消息抹除：Agent 调用 workflow_verify 或 workflow_blocked 后，Engine 抹除 verify 注入消息 + tool_call + tool_result 三条消息

**Jump**（jumping phase）
触发：Agent 调用 workflow_verify
注入：跳转问题（ABCD 选项 + 调用提示，role: workflow）
Agent 动作：workflow_jump({answers})
消息抹除：Engine 抹除 jump 注入消息 + tool_call + tool_result 三条消息

### Session 空闲

Engine 依赖 Session 的空闲判定决定何时进入 verifying phase。Session 空闲的定义由 Session 模块统一管理——LLM 不在请求中、无前台阻塞工具、子 session 均已完成。多个功能（spawn、archive 等）共用此判定。

## 数据流

### Goal 阶段

1. Engine 注入 goal 消息（role: workflow），内容为当前步骤目标描述
2. Agent 收到后连续工具调用完成步骤
3. Agent turn 结束、session idle → 进入 Verify 阶段

### Verify 阶段

1. Engine 注入 verify 消息（role: workflow），内容为验收清单。如当前步骤 allow_blocked，末尾附加 blocked 提示
2. Agent 自查：
   - 继续干活 → Engine 等下次 idle。注入新 verify 前先移除上一条 verify 消息，pending_verify 加一
   - 完成 → workflow_verify() → Engine 抹除三条消息（注入消息 + tool_call + tool_result）→ 进入 Jump 阶段
   - 无法继续且 allow_blocked → workflow_blocked() → Engine 抹除三条消息 → 进入 Blocked 阶段

### Jump 阶段

1. Engine 注入 jump 消息（role: workflow），内容为跳转问题。选项来自当前步骤定义中的 jump 配置，与 transitions 的 when 条件对应
2. Agent 调用 workflow_jump({answers})
3. Engine 按 transitions 顺序匹配条件，执行对应 action（goto/reexecute/complete）
4. Engine 抹除三条消息（jump 注入消息 + tool_call + tool_result），更新 WorkflowRun 状态
5. 注入下一步 goal 或结束

### 跳转评估

Engine 收到 workflow_jump({answers}) 后按 transitions 顺序匹配条件（布尔比对、枚举匹配、字符串比对）。第一个全部满足的 transition 生效，都不满足则执行 default。全硬编码，不依赖 LLM。

### 跳转动作

goto(N)：前进到 Step N，清空 step_data，step_history 追加完成记录。目标 phase 为 executing。
reexecute(N)：重入 Step N，保留 step_data，不追加完成记录，goal 注入时附加重新执行提示。目标 phase 为 executing。
complete：Workflow 结束。目标 phase 为 complete。

### 验证重试

Engine 每次注入 verify 后 pending_verify 计数加一。Agent 调用 workflow_verify 后计数归零。

计数超过上限（默认 3，可在 workflow 定义中配置，每个 workflow 一个上限值）→ phase 转为 blocked。转入 blocked 时，pending_verify 数值保留不动，owner 解除阻塞后归零。转入 blocked 前残留的旧 verify 消息在 owner 解除时一并清理。

pending_verify 在以下情况下归零：Agent 调用 workflow_verify、goto 到新步骤、reexecute 重入步骤、owner 解除 blocked。

没有超时机制。Agent 只要还在执行步骤内容，不管多久 Engine 都等——步骤长度由任务复杂度决定，Engine 不设时间上限。

### 阻塞处理

**Agent 主动阻塞**（当前步骤 allow_blocked 为 true）：

1. Agent 在 verify 阶段调用 workflow_blocked({reason})
2. Engine 将 phase 设为 blocked，通过 Gateway 通知 owner
3. Owner 回复后 Engine 解除阻塞 → pending_verify 归零，移除旧 goal，清理残留 verify 消息，立即注入 verify → verifying
4. Owner 终止 → complete，Engine 执行退出清理

**verify 重试耗尽：**

1. pending_verify 计数超过上限
2. Engine 将 phase 设为 blocked，通过 Gateway 通知 owner
3. Owner 回复后 Engine 解除阻塞 → pending_verify 归零，移除旧 goal，清理残留 verify 消息，立即注入 verify → verifying
4. Owner 终止 → complete，Engine 执行退出清理

## 模块关系

- **Workflow Definition**（同模块）：提供 Step 定义——Engine 按目标、验收清单、跳转问题、跳转规则驱动执行。
- **Session Integration**（同模块）：Engine 将 WorkflowRun 写入 session checkpoint 持久化。
- **Workflow Tools**（同模块）：Engine 接收并处理 workflow_verify/jump/blocked 工具调用。
- **Session**（跨模块）：提供空闲判定——Engine 在 session idle 时触发 verify。
- **Gateway**（跨模块）：blocked 通知 owner 时通过 Gateway 发送。
- **LLM Provider**（无关）：Engine 不直接调用 LLM，通过注入 workflow 消息驱动。
