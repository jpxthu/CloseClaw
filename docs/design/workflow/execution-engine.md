# Execution Engine

## 概述

Execution Engine 是 workflow 的运行时核心，按定义驱动状态机转换、注入三阶段协议消息、硬编码评估跳转条件。

Agent 负责执行步骤内容，Engine 负责流程结构：追踪当前 phase，在 session idle 时注入验收清单，Agent 完成验证后注入跳转问题并评估条件匹配。

## 架构

### 状态机

Engine 管理五个 phase：

1. **executing**
   Agent 正在执行步骤内容，Engine 不干预。
   - 创建 workflow 时进入
   - goto 或 reexecute 后进入
   - Agent turn 结束、session idle → verifying

2. **verifying**
   Engine 已注入验收清单，等待 Agent 响应。
   - Agent 继续干活（调工具、spawn 子 session 等）→ 等下次 idle 重新注入 verify → 循环
   - Agent 完成，调用 workflow_verify → jumping
   - Agent 调用 workflow_blocked（当前步骤允许时）→ blocked
   - 注入次数超过重试上限 → blocked

3. **jumping**
   Engine 已注入跳转问题，等待 Agent 调用 workflow_jump 回答。
   - goto → executing
   - reexecute → executing
   - complete → complete

4. **blocked**
   阻塞状态，等待 owner 介入。触发来源：
   - Agent 调用 workflow_blocked（当前步骤 allow_blocked 为 true）
   - verify 连续注入次数超过上限

   离开转换：
   - Owner 输入解除 → Engine 移除旧 goal，立即注入 verify
   - Owner 终止 workflow → complete

5. **complete**
   Workflow 执行完毕。终止状态，无离开转换。

### 三阶段协议

每个步骤经过三个阶段，由 Engine 驱动转换：

1. **Goal**
   触发：Engine 进入步骤时
   注入：步骤目标描述（纯文本）
   Agent 动作：执行任务
   消息抹除：否

2. **Verify**
   触发：session idle（判定逻辑见 Session 模块）
   注入：验收清单。如果当前步骤 allow_blocked 为 true，末尾附加 "如果确认任务无法继续，调用 workflow_blocked({reason: "原因"})"。否则只注入验收清单 + "如果全部满足，调用 workflow_verify；否则继续完成步骤"。
   Agent 动作：自查
   - 未完成 → 继续执行。Engine 等下次 idle 重新注入。重新注入前先移除上一条 verify 消息（旧消息不再有意义），注入新消息，pending_verify 计数 +1。
   - 完成 → workflow_verify()
   - 无法继续且 allow_blocked → workflow_blocked()
   消息抹除：Agent 调用 workflow_verify 或 workflow_blocked 后，Engine 抹除 verify 注入消息 + tool_call + tool_result 三条消息。Agent 未调任何 workflow 工具而继续干活 → 旧 verify 消息保留，下轮注入前移除。

3. **Jump**
   触发：Agent 调用 workflow_verify
   注入：跳转问题（ABCD 选项 + 调用提示）
   Agent 动作：workflow_jump({answers})
   消息抹除：是。Engine 抹除 jump 注入消息 + tool_call + tool_result 三条消息。效果等同于这次提问没有发生过，直接跳转。

### Session 空闲

Engine 依赖 Session 的空闲判定决定何时进入 verify 阶段。Session 空闲的定义由 Session 模块统一管理——LLM 不在请求中、无前台阻塞工具、子 session 均已完成。多个功能（spawn、archive 等）共用此判定。

## 数据流

### 正常执行流程

**Goal 阶段**

1. Engine 注入 goal 消息（role: workflow, type: goal），内容为当前步骤目标描述
2. Agent 收到后连续工具调用完成步骤
3. Agent turn 结束、session idle → 进入 Verify 阶段

**Verify 阶段**

1. Engine 注入 verify 消息（role: workflow, type: verify），内容为验收清单。如果当前步骤 allow_blocked，末尾附加 blocked 提示
2. Agent 自查：
   - 继续干活 → Engine 等下次 idle。注入新 verify 前先移除上一条 verify 消息。pending_verify +1
   - 完成 → workflow_verify() → Engine 抹除 verify 消息对 → 进入 Jump 阶段
   - 无法继续且 allow_blocked → workflow_blocked() → Engine 抹除 verify 消息对 → 进入 Blocked 阶段

**Jump 阶段**

1. Engine 注入 jump 消息（role: workflow, type: jump），内容为跳转问题
2. Agent 调用 workflow_jump({answers})
3. Engine 按 transitions 顺序匹配条件，执行对应 action
4. Engine 抹除 jump 注入消息 + tool_call + tool_result
5. Engine 更新 WorkflowRun 状态
6. Engine 注入下一步 goal（或结束）

### 跳转评估

Engine 收到 workflow_jump({answers}) 后按 transitions 顺序匹配条件（布尔比对、枚举匹配、字符串比对），第一个全部满足的 transition 生效，都不满足则执行 default。硬编码，不依赖 LLM。

### 跳转动作

goto(N)
: 前进到 Step N。清空 step_data。step_history 追加当前步骤完成记录。

reexecute(N)
: 重入 Step N。保留 step_data。不追加完成记录。goal 注入时附加重新执行提示。

complete
: Workflow 结束。Engine 清理上下文，session 回到普通模式。

### 验证重试

Engine 每次注入 verify 后 pending_verify 计数 +1。Agent 调了 verify → 计数归零。计数 ≥ 上限（默认 3，可在 workflow 定义中配置，每个 workflow 一个上限值）→ phase 转为 blocked，通知 owner。

pending_verify 在以下情况下归零：Agent 调用 workflow_verify、goto 到新步骤、reexecute 重入步骤。

没有超时机制。Agent 只要还在执行步骤内容，不管多久 Engine 都等——步骤长度由任务复杂度决定，Engine 不设时间上限。

### 阻塞处理

**Agent 主动阻塞**（当前步骤 allow_blocked 为 true）：

1. Agent 在 verify 阶段调用 workflow_blocked({reason})
2. Engine：phase = blocked
3. Engine 通过 Gateway 通知 owner（含 reason）
4. Owner 回复后 Engine 解除阻塞 → 移除旧 goal → 立即注入 verify（不等 idle）→ verifying

如果不允许 blocked（allow_blocked 为 false）而 Agent 调了 workflow_blocked → Engine 返回错误，Agent 继续 verify 循环。

**verify 重试耗尽**：

1. pending_verify 计数 ≥ 上限
2. Engine：phase = blocked
3. Engine 通过 Gateway 通知 owner
4. Owner 回复后 Engine 解除阻塞 → 移除旧 goal → 立即注入 verify → verifying

## 模块关系

- **Workflow Definition**（同模块）：提供 Step 定义——Engine 按目标、验收清单、跳转问题、跳转规则驱动执行。
- **Session Integration**（同模块）：Engine 将 WorkflowRun 写入 session checkpoint 持久化。
- **Workflow Tools**（同模块）：Engine 接收并处理 workflow_verify/jump/blocked 工具调用。
- **Session**（跨模块）：提供空闲判定——Engine 在 session idle 时触发 verify。
- **Gateway**（跨模块）：blocked 通知 owner 时通过 Gateway 发送。
- **LLM Provider**（无关）：Engine 不直接调用 LLM，通过注入 workflow 消息驱动。
