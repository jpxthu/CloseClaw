# Session Integration

## 概述

Workflow Engine 深度集成在 session 生命周期中：WorkflowRun 状态随 session checkpoint 持久化，重启后可从断点恢复。进入 workflow 模式后，Engine 通过 system prompt 追加区注入 workflow context。

## 架构

### 持久化数据

WorkflowRun 作为 session 的附加状态随 session checkpoint 持久化（SessionCheckpoint 数据模型见 [session-lifecycle.md](../session/session-lifecycle.md)）：

- workflow_id：关联的 workflow 定义标识
- definition_version：定义版本号，用于检测定义变更
- current_step：当前步骤编号
- phase：executing / verifying / jumping / blocked / complete
- step_history：已完成步骤记录（步骤编号、进入时间、完成时间、状态）
- pending_verify：验证重试状态（注入次数、最后注入时间、最大重试次数）
- paused_reason：暂停原因（仅在 phase 为 blocked 时写入，取值因触发来源而异：主动阻塞为 Agent 提供的 reason；被动暂停（验收重试耗尽）为固定文本「验收重试次数耗尽」；恢复时检测到当前步骤在最新定义中已不存在时为「当前步骤在最新定义中已不存在」。非 blocked 阶段为空。）

正常持久化时机随 session checkpoint，在对话轮次间写入。workflow 退出时 Engine 主动触发一次额外 checkpoint 写入空状态。

### System Prompt 注入

进入 workflow 模式后，Engine 通过 system prompt 追加区的**系统注入通道**注入 workflow context——这是追加区承载的系统注入内容，与 Owner 动态指令（`/system add`）相互独立：写入/移除不触发 System Prompt 重新组装，也不清除 Owner 动态指令（详见 [system_prompt/appends.md](../system_prompt/appends.md) §两类内容与独立管理路径）。

注入内容：

```
--- WORKFLOW ---
你正在执行受控工作流：{workflow_name}
描述：{description}
Engine 会通过 workflow 角色消息驱动步骤推进，必须遵守三阶段协议：
1. 收到 goal → 执行步骤
2. 收到 verify → 自查验收清单 → 完成则调用 workflow_verify，否则继续
3. 收到 jump → 回答问题 → 调用 workflow_jump 传递答案
不要自行跳步或跳过验证。
--- WORKFLOW END ---
```

注入时机：

1. 首次进入 workflow 模式（斜杠指令或工具调用）
2. Session 从归档恢复时（重新注入，保证内容最新）
3. Compaction 完成后（system prompt 重建时重新注入）

### 消息管理

Workflow 控制消息（role: workflow）与普通对话消息独立管理：

- goal 消息：保留，不参与压缩
- recovered 消息：保留，不参与压缩，退出时随 goal 消息一并清理
- verify 消息：处理后删除（含对应的 tool_call 和 tool_result）
- jump 消息：处理后删除（含对应的 tool_call 和 tool_result）

## 数据流

### 进入 Workflow 模式

一个 Session 同一时刻只能执行一个 workflow——若当前 Session 已处于 workflow 模式（存在未完成的 WorkflowRun），Engine 拒绝新的入模请求（斜杠指令或 workflow_start 均返回错误）。

1. 用户输入 /workflow <name> 或 Agent 调用 workflow_start({name})
2. Engine 确认当前 Session 无未完成 WorkflowRun（见上），加载定义，初始化 WorkflowRun：current_step 置 0，phase 置 executing
3. Engine 向 system prompt 追加区注入 workflow context
4. 待追加区写入完毕后，Engine 注入 role 为 workflow 的 Step 0 goal 消息（与其他 workflow 角色消息复用同一路由路径）
5. Agent 开始执行

workflow 一旦开始即不可回退为普通 Session——只能由 Engine 判定 jump 结果为 complete 后正常结束，或由 Owner 主动终止。

### 轮次间持久化

每次 checkpoint 写入时附带 WorkflowRun 的完整字段：
workflow_id、definition_version、current_step、phase、step_history、pending_verify、paused_reason。

### 从归档恢复

Session 归档恢复时，Engine 重建会话后检测未完成 workflow，并按持久化的运行状态分派。运行状态 = 运行中（phase ∈ executing/verifying/jumping）或暂停中（phase = blocked）。

**运行中恢复**

1. Session 从归档恢复，SessionManager 重建 ConversationSession
2. 检测 WorkflowRun 存在且 phase ≠ complete
3. 若当前步骤在最新定义中仍存在 → 自动恢复：
   - Engine 注入 recovered 消息（role: workflow）："正在执行 {workflow_name}，当前 Step {N}"
   - Engine 注入当前步骤 goal 消息（role: workflow）
   - Engine 通过 System Prompt 重新注入 workflow context
   - Agent 从中断点继续
4. 若当前步骤在最新定义中已不存在（定义已变更）→ 按「定义版本变更」转为暂停：phase 置 blocked、填入暂停原因、通知 Owner，不自动恢复

**暂停中恢复（phase = blocked）**

1. Session 从归档恢复，SessionManager 重建 ConversationSession
2. Engine 检测 WorkflowRun phase = blocked
3. Engine 判断当前步骤在最新定义中是否仍存在：
   - 已不存在（定义已变更）→ 按「定义版本变更」处置：将 paused_reason 置为固定文本「当前步骤在最新定义中已不存在」，通知 Owner，该暂停仅能由 Owner 终止
   - 仍存在或未检出定义变更 → 保持暂停，不自动恢复。Engine 通过 System Prompt 重新注入 workflow context，并经 Gateway 重新告知 Owner 持久化的暂停原因（paused_reason），等待 Owner 处理
4. 对该仍存在的阻塞性暂停（来源为验收重试耗尽或 Agent 主动阻塞），Owner 回复后 Engine 按 F6（见 execution-engine.md 阻塞处理）解除：保留当前步骤目标消息、pending_verify 归零、清理残留 verify 消息、注入 verify，Agent 从暂停前阶段继续；Owner 亦可直接终止 workflow

后续注入：运行中恢复且当前 phase 为 verifying，Engine 待验收判定条件满足后重新注入 verify；若为 jumping，重新注入 jump 问题。

### 退出 Workflow 模式

1. Workflow 正常结束（phase = complete）或 Owner 终止
2. Engine 从追加区移除 workflow context
3. Engine 清理消息历史中的 workflow 控制消息（goal + recovered）
4. Engine 清空 WorkflowRun 状态
5. Engine 主动触发 checkpoint 写入，持久化空状态
6. Session 恢复为普通 session

### 定义版本变更

判定依据：恢复时对比持久化的定义版本与最新定义，若当前步骤已在最新定义中删除，则无法续跑/无法按原解除路径继续。处置按暂停与否分：

- **运行中**：phase 转为 blocked，填入暂停原因「当前步骤在最新定义中已不存在」，通知 Owner。该暂停仅能由 Owner 终止，不能解除续跑（无法对已不存在的步骤复写验收流程）
- **暂停中（phase = blocked）**：保持暂停（按暂停中恢复 step3 处置），Engine 告知 Owner「当前步骤在最新定义中已不存在」，该暂停仅能由 Owner 终止，不能解除。

## 模块关系

### 上游

- **SessionManager**：session 创建/恢复时触发 Engine 初始化。checkpoint 持久化时 Engine 写入 WorkflowRun 状态。
- **System Prompt**：提供追加区注入接口，Engine 通过此接口管理 workflow context。
- **Gateway**：恢复时注入 recovered 消息需通过 Gateway 路由。

### 下游

- **Execution Engine**（同模块）：从 WorkflowRun 状态恢复后继续驱动步骤执行。

### 无关

- **Compaction**：workflow 消息（除 goal）在完成后已删除，不参与压缩。Goal 消息压缩时保留。Compaction 完成后 Engine 重新注入 workflow context。
- **Memory**：workflow 不参与记忆挖掘或搜索注入。
