# Session Integration

## 概述

Workflow Engine 深度集成在 session 生命周期中：WorkflowRun 状态随 session checkpoint 持久化，重启后可从断点恢复。进入 workflow 模式后，Engine 通过 system prompt 追加区注入 workflow context。

## 架构

### 持久化数据

WorkflowRun 作为 session 的附加状态随 session checkpoint 持久化：

- workflow_id：关联的 workflow 定义标识
- definition_version：定义版本号，用于检测定义变更
- current_step：当前步骤编号
- phase：executing / verifying / jumping / blocked / complete
- step_history：已完成步骤记录（步骤编号、进入时间、完成时间、状态）
- step_data：跨步骤共享的运行时数据（键值对）
- pending_verify：验证重试状态（注入次数、最后注入时间、最大重试次数）

持久化时机与 session checkpoint 一致——在对话轮次间写入。

### System Prompt 注入

进入 workflow 模式后，Engine 向 system prompt 追加区注入 workflow context。复用 /system add 的注入路径，内容包含 workflow 名称、描述和三阶段协议约束。注入与其他追加条目共存于追加区，以 --- WORKFLOW --- 和 --- WORKFLOW END --- 边界标记分隔。

注入时机：首次进入 workflow 模式（斜杠指令或工具调用）、Session 从归档恢复时（重新注入保证内容最新）、Compaction 完成后（system prompt 重建时重新注入）。

### 消息管理

Workflow 控制消息（role: workflow）独立于普通对话消息：goal 消息保留在 context 中不参与压缩；verify 和 jump 消息在 Engine 处理完成后从消息历史中删除（含对应的 tool_call 和 tool_result）。

## 数据流

### 进入 Workflow 模式

斜杠指令或 workflow_start 调用触发。Engine 加载定义并初始化 WorkflowRun（current_step 为 0，phase 为 executing，step_history 为空）。Engine 向 system prompt 追加区注入 workflow context，向消息历史注入 role 为 workflow 的 Step 0 goal 消息，Agent 开始执行。

### 轮次间持久化

每次 checkpoint 写入时 SessionCheckpoint 携带 WorkflowRun 的完整状态：workflow_id、definition_version、current_step、phase、step_history、step_data、pending_verify。

### 从归档恢复

Session 从归档恢复后 SessionManager 重建 ConversationSession，System Prompt Builder 重新构建 system prompt。Engine 检测到 WorkflowRun 存在且 phase 不是 complete 时：注入 recovered 消息（role: workflow，说明正在执行的 workflow 名称和当前步骤编号），注入当前步骤 goal 消息（role: workflow），向 system prompt 追加区重新注入 workflow context。Agent 从中断点继续。恢复后 verify 和 jump 的交互流程由 Engine 管理（详见 execution-engine.md），Engine 根据当前 phase 决定注入 verify 还是 jump。

### 退出 Workflow 模式

Workflow 正常结束（phase 为 complete）时 Engine 执行清理：从 system prompt 追加区移除 workflow context，清理消息历史中的 workflow goal 消息，清空 WorkflowRun 状态，触发一次 checkpoint 写入将空状态持久化。Session 恢复为普通 session。

### 定义版本变更

恢复时 Engine 检测到 definition_version 不匹配：当前步骤编号在新定义中仍存在则使用新定义继续；不存在则标记为 blocked 并通知 owner。

## 模块关系

### 上游

- **SessionManager**：session 创建和恢复时触发 Engine 初始化。checkpoint 持久化时 Engine 写入 WorkflowRun 状态。
- **System Prompt Builder**：提供追加区注入接口，Engine 通过此接口管理 workflow context。
- **Gateway**：session 恢复时如需注入 recovered 消息，通过 Gateway 路由。

### 下游

- **Execution Engine**：从 WorkflowRun 状态恢复后继续驱动步骤执行。verify 和 jump 的交互流程由执行引擎管理（详见 execution-engine.md）。

### 无关

- **Compaction**（无调用关系）：workflow 消息（除 goal）在完成后已删除，不参与压缩。Goal 消息为单条 workflow role 消息，压缩时保留。Compaction 完成后 system prompt 重建时 Engine 重新注入 workflow context。
- **Memory**（无调用关系）：workflow 不参与记忆挖掘或搜索注入。
