# Execution Engine

## 概述

Execution Engine 是 workflow 的运行时核心，管理步骤执行的完整生命周期：按定义驱动状态转换、注入协议消息、评估跳转条件。Agent 负责执行步骤内容，Engine 负责流程结构。

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
   - Agent 未完成 → 继续执行，等下次 idle 时 Engine 重新注入 → executing（循环）
   - Agent 完成，调用 workflow_verify → jumping
   - 超时（多次注入无响应）→ blocked

3. **jumping**
   Engine 已注入跳转问题，等待 Agent 调用 workflow_jump 回答。
   - goto → executing
   - reexecute → executing
   - complete → complete

4. **blocked**
   阻塞状态。触发来源：blocking 步骤等待 owner / Agent 调用 workflow_blocked / 验证超时。
   - Owner 输入解除 → verifying
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
   触发：session idle
   注入：验收清单 + "如果全部满足，调用 workflow_verify；否则继续完成步骤"
   Agent 动作：自查 → verify() 或继续执行
   消息抹除：是（含 tool_call 和 tool_result）

3. **Jump**
   触发：Agent 调用 verify
   注入：跳转问题（ABCD 选项）
   Agent 动作：回答 → jump({answers})
   消息抹除：是（含 tool_call 和 tool_result）

### 消息角色

所有 Engine 注入的消息使用独立的 workflow 角色，与 user、assistant、system、tool 并列。Agent 通过消息角色识别流程控制消息。

### 空闲判定

Engine 在 session 空闲时触发 verify。空闲需同时满足：

1. LLM 不在请求中
2. 没有前台阻塞的工具调用
3. 所有子 session 已完成

后台工具执行中不影响空闲判定。

## 数据流

### 正常执行流程

**Goal 阶段**

1. Engine 注入 goal 消息（role: workflow, type: goal），内容为当前步骤目标描述
2. Agent 收到后连续工具调用完成步骤，可 spawn 子 session、多轮思考
3. Agent turn 结束、session idle → 进入 Verify 阶段

**Verify 阶段**

1. Engine 注入 verify 消息（role: workflow, type: verify），内容为验收清单条目 + 引导语
2. Agent 自查：
   - 未完成 → 继续执行 → 等下次 idle → Engine 重新注入（回到步骤 1）
   - 完成 → 调用 workflow_verify()
3. Engine 收到 verify → 抹除 verify 交互记录（注入消息 + tool_call + tool_result）→ 进入 Jump 阶段

**Jump 阶段**

1. Engine 注入 jump 消息（role: workflow, type: jump），内容为跳转问题（ABCD 选项 + 调用提示）
2. Agent 调用 workflow_jump({answers})
3. Engine 按 transitions 顺序匹配条件，执行对应 action
4. Engine 抹除 jump 交互记录（注入消息 + tool_call + tool_result）
5. Engine 更新 WorkflowRun 状态
6. Engine 注入下一步 goal（或结束）

### 跳转评估

Engine 收到 workflow_jump({answers}) 后的处理：

1. 取第一条 transition
2. 检查 when 中所有条件是否与 answers 匹配
3. 全部匹配 → 执行该 transition 的 action
4. 有任一不匹配 → 取下一个 transition
5. 所有 when transition 都不匹配 → 执行 default

匹配是纯硬编码的——布尔比对、枚举匹配、字符串比对。Engine 不做语义理解。

### 跳转动作

goto(N)
: 前进到 Step N。清空 step_data，step_history 追加当前步骤完成记录。

reexecute(N)
: 重入 Step N。保留 step_data，不追加完成记录。goal 注入时附加"重新执行（已保留数据：step_data 摘要）"提示。

complete
: Workflow 结束。Engine 清理 workflow 上下文，session 可继续作为普通 session 使用。

### 验证重试

正常情况：Agent 未完成验证时可继续执行，Engine 等下次 session idle 自动重新注入 verify。

超时兜底：
1. Engine 等待超时（默认 5 分钟）
2. 超时 → 重新注入 verify（pending_verify 计数 +1）
3. 超过重试上限（默认 3 次）
4. → phase = blocked
5. → 通知 owner

### 阻塞处理

Blocking 类型步骤：
1. Engine 注入 goal
2. Engine 标记为 blocked
3. Owner 输入到达 → Engine 评估 → 转为 verifying → 正常 verify → jump 流程

Agent 主动阻塞：
1. Agent 调用 workflow_blocked({ reason })
2. Engine 标记为 blocked，通知 owner
3. Owner 回复后 → Engine 评估 → 转为 verifying → 正常 verify → jump 流程

超时死锁导致的 blocked 同样走 owner 决策路径。

## 模块关系

### 上游

- **Workflow Definition**：提供 Workflow 结构体，Engine 按 Step 定义驱动执行。
- **Session**：提供空闲判定，Engine 在 session idle 时触发 verify。
- **Gateway**：blocked 状态通知 owner 时通过 Gateway 发送消息。

### 下游

- **Session**：WorkflowRun 状态随 session checkpoint 持久化，由 Engine 写入。
- **Workflow Tools**：Engine 接收 workflow_verify/jump/blocked 工具调用并处理。

### 无关

- **LLM Provider**（无调用关系）：Engine 不直接调用 LLM，通过注入 workflow 消息间接驱动。
