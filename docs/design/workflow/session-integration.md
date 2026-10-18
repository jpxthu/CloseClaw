# Session Integration

## 概述

Workflow Engine 深度集成在 session 生命周期中：WorkflowRun 状态随 session checkpoint 持久化，系统重启或 Session 归档恢复后可从断点恢复。进入 workflow 模式后，Engine 通过 system prompt 追加区注入 workflow context。

## 架构

### 持久化数据

WorkflowRun 作为 session 的附加状态随 session checkpoint 持久化（SessionCheckpoint 数据模型见 [session-lifecycle.md](../session/session-lifecycle.md)）：

- workflow_id：关联的 workflow 定义标识
- definition_version：定义版本指纹——Engine 加载时对结构化定义计算的内容指纹（覆盖 Workflow 的 id/name/description/allow_blocked/verify_retry_limit，以及全部 Step 的 id/name/allow_blocked/goal/verify、jump（含 id/prompt/options/option_labels）、transitions（含 when/action/target_step）），用于检测定义变更；不写入定义文件
- current_step：当前步骤编号
- phase：executing / verifying / jumping / blocked / complete
- step_history：已完成步骤记录（步骤编号、进入时间、完成时间）；进入时间取首次进入该步骤（goto）的时间，完成时间取首次完成（workflow_verify 通过并完成跳转决策）的时间；reexecute 不追加记录、不修改已记录时间
- pending_verify：验收重试状态（当前注入次数、最大重试次数）；最大重试次数为加载时的快照，供恢复时使用，避免依赖重新读取定义
- paused_reason：暂停原因（仅在 phase 为 blocked 时写入，取值因触发来源而异：主动阻塞为 Agent 提供的 reason；被动暂停（验收重试耗尽）为固定文本「验收重试次数耗尽」；恢复时检测到当前步骤在最新定义中已不存在时为「当前步骤在最新定义中已不存在」。非 blocked 阶段为空。）

正常持久化时机随 session checkpoint，在对话轮次间写入。workflow 退出时 Engine 主动触发一次额外 checkpoint 写入空状态。

### System Prompt 注入

进入 workflow 模式后，Engine 通过 system prompt 追加区的**系统注入通道**注入 workflow context——这是追加区承载的系统注入内容，与 Owner 动态指令（`/system add`）相互独立：写入/移除不触发 System Prompt 重新组装，也不清除 Owner 动态指令（详见 [system_prompt/appends.md](../system_prompt/appends.md) §两类内容与独立管理路径）。

注入内容：

```
--- WORKFLOW ---
你正在执行受控工作流：{workflow_name}
描述：{description}
定义文件：{path}
Engine 会通过 workflow 控制消息（role: workflow）驱动步骤推进，必须遵守三阶段协议：
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

workflow 控制消息（role: workflow）经 session 统一消息队列注入（now 优先级），不经入站 Processor Chain、不出站渲染，与普通对话消息独立管理：

- 步骤目标消息：保留，不参与压缩
- recovered 恢复提示：保留，不参与压缩，退出时随步骤目标消息一并清理
- 验收清单：等待 Agent 响应期间不参与压缩；跳转决策完成后删除（含对应的 tool_call 和 tool_result）；Agent 主动阻塞时随阻塞立即删除
- 跳转问题：等待 Agent 响应期间不参与压缩；跳转决策完成后删除（含对应的 tool_call 和 tool_result）

恢复时先清理 transcript 中残留的 workflow 控制消息（步骤目标、recovered、未完成的验收/跳转交互），再按当前 phase 重新注入，保证每个控制消息只有一份。Compaction 完成后按当前 phase 重新注入必要控制消息（步骤目标、验收清单或跳转问题），不依赖压缩前消息是否保留。

## 数据流

### 进入 Workflow 模式

一个 Session 同一时刻只能执行一个 workflow——若当前 Session 已处于 workflow 模式（存在未完成的 WorkflowRun），Engine 拒绝新的入模请求（斜杠指令或 workflow_start 均返回错误）。

1. 用户输入 /workflow <name> 或 Agent 调用 workflow_start({name})
2. Engine 确认当前 Session 无未完成 WorkflowRun（见上），加载定义，初始化 WorkflowRun：current_step 置 0，phase 置 executing
3. Engine 向 system prompt 追加区注入 workflow context
4. 待追加区写入完毕后，Engine 经统一消息队列注入 role 为 workflow 的 Step 0 步骤目标消息（与其他 workflow 控制消息复用同一路由路径）
5. Agent 开始执行

workflow 一旦开始即不可回退为普通 Session——只能由 Engine 判定跳转结果为 complete 后正常结束，或由 Owner 主动终止。

### 轮次间持久化

每次 checkpoint 写入时附带 WorkflowRun 的完整字段：
workflow_id、definition_version、current_step、phase、step_history、pending_verify、paused_reason。

### 系统重启与归档恢复

系统重启或 Session 归档恢复时，Engine 重建会话后检测未完成 workflow，并按持久化的运行状态分派。运行状态 = 运行中（phase ∈ executing/verifying/jumping）或暂停中（phase = blocked）。

**运行中恢复**

1. Session 从归档恢复，SessionManager 重建 ConversationSession
2. 检测 WorkflowRun 存在且 phase ≠ complete
3. 若当前步骤在最新定义中仍存在 → 自动恢复：
   - Engine 清理 transcript 中残留的 workflow 控制消息（步骤目标、recovered、未完成的验收/跳转交互）
   - Engine 通过 System Prompt 重新注入 workflow context
   - Engine 注入 recovered 恢复提示（role: workflow）："workflow 已恢复：{workflow_name}，当前 Step {N}（phase: {phase}）"
   - Engine 注入当前步骤的步骤目标消息（role: workflow）
   - 步骤目标消息经统一消息队列出队后触发 Agent turn（session 恢复后四维为 idle），Agent 从中断点继续
4. 若当前步骤在最新定义中已不存在（定义已变更）→ 按「定义版本变更」转为暂停：phase 置 blocked、填入暂停原因、通知 Owner，不自动恢复

**暂停中恢复（phase = blocked）**

1. Session 从归档恢复，SessionManager 重建 ConversationSession
2. Engine 检测 WorkflowRun phase = blocked
3. Engine 判断当前步骤在最新定义中是否仍存在：
   - 已不存在（定义已变更）→ 按「定义版本变更」处置：将 paused_reason 置为固定文本「当前步骤在最新定义中已不存在」，通知 Owner，该暂停仅能由 Owner 终止
   - 当前步骤仍存在（定义未变更，或变更后该步骤仍在）→ 保持暂停，不自动恢复。Engine 通过 System Prompt 重新注入 workflow context，并经 Gateway 重新告知 Owner 持久化的暂停原因（paused_reason），等待 Owner 处理
4. 对该仍存在的阻塞性暂停（来源为验收重试耗尽或 Agent 主动阻塞），Owner 发送非斜杠消息后 Engine 解除阻塞（解除动作见 execution-engine.md「阻塞处理」），Agent 从暂停前阶段继续；Owner 输入 `/stop` 则终止 workflow（退出清理见「退出 Workflow 模式」）

后续注入：运行中恢复且当前 phase 为 verifying，Engine 待验收判定条件满足后重新注入验收清单，该次注入不增加 pending_verify 计数（状态恢复，不是重试）；若为 jumping，重新注入跳转问题。

### 退出 Workflow 模式

1. Workflow 正常结束（phase = complete）或 Owner 通过 `/stop` 终止；非 Owner 的 `/stop`、父 Session 停止、系统关闭不触发退出清理，WorkflowRun 保留供恢复
2. Engine 从追加区移除 workflow context
3. Engine 清理消息历史中的 workflow 控制消息（步骤目标消息、recovered 恢复提示，以及终止时仍在飞的验收清单、跳转问题交互记录）
4. Engine 清空会话中的 workflow 运行状态（WorkflowRun）
5. Engine 主动触发 checkpoint 写入，持久化该清空后的状态
6. Session 恢复为普通 session

### 定义版本变更

判定分两级：

1. 持久化的 definition_version 与最新定义指纹相同 → 定义未变更，按持久化状态恢复。
2. 指纹不同 → 以最新定义为准（需求 F1：中断续跑的恢复视为一次启动）：当前步骤仍存在 → 继续恢复，后续按最新定义的 goal / verify / jump 执行，并将 definition_version 更新为最新指纹。当前步骤已不存在 → 进入终止性暂停。step_history 保留原记录，不按新定义重映射。

终止性暂停按暂停与否处置：

- **运行中**：phase 转为 blocked，填入暂停原因「当前步骤在最新定义中已不存在」，通知 Owner。该暂停仅能由 Owner 终止，不能解除续跑（无法对已不存在的步骤复写验收流程）
- **暂停中（phase = blocked）**：保持暂停，将暂停原因更新为「当前步骤在最新定义中已不存在」，通知 Owner；该暂停仅能由 Owner 终止，不能解除。

## 模块关系

> 完整模块关系（含 Skills、Tools 等）见 [README.md](README.md#模块关系)。

### 上游

- **SessionManager**：session 创建/恢复时触发 Engine 初始化。checkpoint 持久化时 Engine 写入 WorkflowRun 状态。
- **System Prompt**：提供追加区注入接口，Engine 通过此接口管理 workflow context。
- **Slash**：`/workflow` 斜杠指令触发 workflow 启动（仅 Owner 可用、非 Immediate），由 workflow 模块提供的 WorkflowHandler 处理。
- **Gateway**：Engine 经 [SessionLookup](../common/core-traits.md#sessionlookup) 的统一消息队列注入控制消息与 recovered 恢复提示；blocked 通知经 [Gateway 系统通知接口](../gateway/outbound-flow.md#系统通知接口)向 Owner 推送 IM 通知；Gateway 在 Owner 非斜杠消息路由前与 Owner `/stop` 停止完成后经 [WorkflowControl](../common/core-traits.md#workflowcontrol) 通知 Engine（解除阻塞 / 终止）。
- **Compaction**：压缩完成后触发 System Prompt 重建，Engine 重新注入 workflow context，并按当前 phase 重新注入必要控制消息（步骤目标、验收清单或跳转问题）；workflow 控制消息不参与压缩。

### 下游

- **Execution Engine**（同模块子功能）：从 WorkflowRun 状态恢复后继续驱动步骤执行。

### 无关

- **Memory**：workflow 不参与记忆挖掘或搜索注入。

### 共享类型 / 核心 trait

- 实现 [WorkflowControl](../common/core-traits.md#workflowcontrol)：供 Gateway 在 WorkflowStart、Owner 非斜杠消息与 Owner `/stop` 入口调用。
- 消费 [SessionLookup](../common/core-traits.md#sessionlookup)：向统一消息队列注入 workflow 控制消息（role: workflow）。
- 消费 [SlashSessionQuery](../common/core-traits.md#slashsessionquery)：查询/持久化 WorkflowRun 与当前活跃 workflow phase。

