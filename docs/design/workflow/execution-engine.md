# Execution Engine

## 概述

Execution Engine 是 workflow 的运行时核心，按定义驱动状态机转换、注入三阶段协议消息、硬编码评估跳转条件。

Agent 负责执行步骤内容，Engine 负责流程结构：追踪当前 phase（executing/verifying/jumping/blocked/complete），在 session idle 时注入验收清单，Agent 完成验证后注入跳转问题并评估条件匹配。

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
   - Agent 继续干活（调工具、spawn 子 session 等）→ 等下次 idle Engine 重新注入 → executing（循环）
   - Agent 完成，调用 workflow_verify → jumping
   - 注入次数超过重试上限 → blocked

3. **jumping**
   Engine 已注入跳转问题，等待 Agent 调用 workflow_jump 回答。
   - goto → executing
   - reexecute → executing
   - complete → complete

4. **blocked**
   阻塞状态。触发来源：blocking 步骤等待 owner / Agent 调用 workflow_blocked / 验证重试耗尽。
   - Owner 输入解除 → Engine 移除旧 goal，转为 verifying
   - Owner 终止 → complete

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
   注入：验收清单 + "如果全部满足，调用 workflow_verify；否则继续完成步骤"
   Agent 动作：自查
   - 未完成 → 继续执行（Engine 不干预，等下次 idle 重新注入）
   - 完成 → workflow_verify()
   消息抹除：条件性——如果 Agent 紧接着调了 workflow_verify，Engine 抹除 verify 注入消息 + tool_call + tool_result 三条消息。如果 Agent 没调 verify 而是继续干活，只保留注入的验收消息（无 tool call 可抹），等下一轮。

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

Engine 注入验收清单（role: workflow, type: verify）。Agent 自查：

- Agent 继续干活（调工具等）：Engine 等下次 idle 重新注入。上一条 verify 注入消息保留在 context 中（无 tool call 可抹除）。此循环计入 pending_verify 计数。
- Agent 完成，调用 workflow_verify()：Engine 抹除 verify 注入消息 + tool_call + tool_result，进入 Jump 阶段。

**Jump 阶段**

1. Engine 注入 jump 消息（role: workflow, type: jump），内容为跳转问题。选项直接来自当前步骤定义中的 jump 问题，与 transitions 的 when 条件对应。
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

Engine 每次注入 verify 后 pending_verify 计数 +1。Agent 继续干活未调 verify → 等下次 idle 重新注入（注入前先移除上一条 verify 消息，避免堆积），计数继续累加。Agent 调了 verify → 计数归零。

计数 ≥ 上限（默认 3，可在 workflow 定义中配置）→ phase 转为 blocked，通知 owner。

pending_verify 在以下情况下归零：Agent 调用 workflow_verify、goto 到新步骤、reexecute 重入步骤。

没有超时机制。Agent 只要还在执行步骤，不管多久 Engine 都等——步骤本身的长度由任务复杂度决定，Engine 不设时间上限。

### 阻塞处理

Blocking 类型步骤等待 owner 输入：

1. Engine 注入 goal（含"提交给 owner 确认，等待回复"的指引）
2. Engine 标记为 blocked
3. Owner 回复后，Engine 转为 verifying
4. Agent 收到 verify（检查清单："owner 已提供输入"）→ 调用 workflow_verify → jump

Agent 主动阻塞同理——调用 workflow_blocked({reason}) → Engine 标记 blocked 并通知 owner → owner 回复后 verifying。

## 模块关系

- **Workflow Definition**（同模块）：提供 Step 定义——Engine 按目标、验收清单、跳转问题、跳转规则驱动执行。
- **Session Integration**（同模块）：Engine 将 WorkflowRun 写入 session checkpoint 持久化。
- **Workflow Tools**（同模块）：Engine 接收并处理 workflow_verify/jump/blocked 工具调用。
- **Session**（跨模块）：提供空闲判定——Engine 在 session idle 时触发 verify。
- **Gateway**（跨模块）：blocked 通知 owner 时通过 Gateway 发送。
- **LLM Provider**（无关）：Engine 不直接调用 LLM，通过注入 workflow 消息驱动。
