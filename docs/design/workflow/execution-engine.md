# Execution Engine

## 概述

Execution Engine 是 workflow 的运行时核心，管理步骤执行的完整生命周期：按定义驱动状态转换、注入协议消息、评估跳转条件。Agent 负责执行步骤内容，Engine 负责流程结构。

## 架构

### 状态机

Engine 管理五个 phase：

- **executing**：Agent 正在执行步骤内容，Engine 不干预。创建时进入此状态，每次 goto/reexecute 后也进入此状态。
- **verifying**：Agent turn 结束 session idle 后 Engine 注入验收清单，等待 Agent 响应。未完成则 Agent 继续执行，等下次 idle Engine 再次注入；完成则 Agent 调用 workflow_verify 进入 jumping。
- **jumping**：Engine 注入跳转问题，等待 Agent 调用 workflow_jump 回答。
- **blocked**：阻塞状态——blocking 步骤等待 owner 输入、Agent 主动请求阻塞、或死锁超时。
- **complete**：workflow 执行完毕。

状态转换路径：创建 → executing。Agent turn 结束 → verifying。验证未完成 → executing（循环）。完成调 verify → jumping。jump 回答后 → executing（goto/reexecute）或 complete。超时 → blocked。Agent 主动阻塞 → blocked。Owner 输入后 blocked → verifying。Owner 终止 → complete。

### 三阶段协议

每个步骤经过三个阶段，由 Engine 驱动转换：

| 阶段 | 触发 | Engine 注入 | Agent 动作 | 消息抹除 |
|------|------|------------|-----------|---------|
| Goal | 进入步骤时 | 步骤目标描述（纯文本） | 执行任务 | 否 |
| Verify | session idle | 验收清单 + "全部满足则调 workflow_verify，否则继续" | 自查 → verify() 或继续 | 是 |
| Jump | Agent 调 verify() | 跳转问题（ABCD 选项） | 回答 → jump({answers}) | 是 |

### 消息角色

所有 Engine 注入的消息使用独立的 workflow 角色，与 user、assistant、system、tool 并列。Agent 通过消息角色识别流程控制消息。

### 空闲判定

Engine 在 session 空闲时触发 verify。空闲条件：LLM 不在请求中、没有前台阻塞的工具调用、所有子 session 已完成。后台工具执行中不影响空闲判定。

## 数据流

### 正常执行流程

Engine 注入 goal 消息（role: workflow，type: goal，内容为当前步骤目标描述）。Agent 收到后连续工具调用完成步骤，可 spawn 子 session、可多轮思考。Agent turn 结束 session idle 时 Engine 注入 verify 消息（role: workflow，type: verify，内容为验收清单条目加引导语"如果全部满足，调用 workflow_verify；否则继续完成步骤"）。

Agent 自查——未完成则继续执行等下次 idle，完成则调用 workflow_verify()。Engine 收到后抹除 verify 交互记录（含 tool_call 和 tool_result），注入 jump 消息（role: workflow，type: jump，内容为跳转问题——问题和选项以自然语言呈现，选项渲染为 ABCD 标签加文字解释，底部附调用提示）。

Agent 回答 workflow_jump({answers})。Engine 按 transitions 顺序匹配条件，执行对应的 action，抹除 jump 交互记录（含 tool_call 和 tool_result），更新 WorkflowRun 状态，注入下一步 goal 或结束。

### 跳转评估

Engine 收到 workflow_jump({answers}) 后按 transitions 顺序匹配：取第一条 transition 检查 when 中所有条件是否与 answers 匹配，全部匹配则执行该 transition 的 action，否则取下一个。全部 when transition 都不匹配则执行 default transition。匹配是纯硬编码的——布尔比对、枚举匹配、字符串比对。

### 跳转动作

goto(N)：前进到 Step N，清空 step_data，step_history 追加当前步骤完成记录。

reexecute(N)：重入 Step N，保留 step_data，不追加完成记录，goal 注入时附加"重新执行（已保留数据：{step_data 摘要}）"提示。

complete：Workflow 结束，Engine 清理 workflow 上下文，session 可继续作为普通 session 使用。

### 验证重试

Agent 未完成验证时可继续执行，Engine 等下次 session idle 自动重新注入 verify。超时死锁保护作为兜底：Engine 等待超时（默认 5 分钟）后重新注入 verify（pending_verify 计数加 1），超过重试上限（默认 3 次）则标记为 blocked 并通知 owner。

### 阻塞处理

Blocking 类型步骤中 Engine 注入 goal 后将步骤状态标记为 blocked，等 owner 输入到达后评估并转为 verifying 进入正常流程。Agent 也可主动调用 workflow_blocked 请求阻塞，Engine 通知 owner 后等待回复，再评估并转为 verifying。超时死锁导致的 blocked 同样走 owner 决策路径。

## 模块关系

### 上游

- **Workflow Definition**：提供 Workflow 结构体，Engine 按 Step 定义驱动执行。
- **Session**：提供空闲判定，Engine 在 session idle 时触发 verify。
- **Gateway**：blocking 状态通知 owner 时通过 Gateway 发送消息。

### 下游

- **Session**：WorkflowRun 状态随 session checkpoint 持久化，由 Engine 写入。
- **Workflow Tools**：Engine 接收 workflow_verify/jump/blocked 工具调用并处理。

### 无关

- **LLM Provider**（无调用关系）：Engine 不直接调用 LLM，通过注入 workflow 消息间接驱动。
