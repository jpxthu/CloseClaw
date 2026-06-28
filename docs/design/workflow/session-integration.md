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

进入 workflow 模式后，Engine 向 system prompt 追加区注入 workflow context，复用 /system add 的注入路径。

注入内容：

```
--- WORKFLOW ---
你正在执行受控工作流：{workflow_name}
描述：{description}
Engine 会通过 workflow 角色消息驱动步骤推进，必须遵守三阶段协议：
1. 收到 goal → 执行步骤
2. 收到 verify → 自查验收清单 → 完成则调用验证完成工具，否则继续
3. 收到 jump → 回答问题 → 调用跳转问答工具传递答案
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

1. 用户输入 /workflow <name> 或 Agent 调用 workflow_start({name})
2. Engine 加载定义，初始化 WorkflowRun：current_step 置 0，phase 置 executing
3. Engine 向 system prompt 追加区注入 workflow context
4. Engine 注入 role 为 workflow 的 Step 0 goal 消息（通过 Gateway 路由，与普通 workflow 消息路由路径一致）
5. Agent 开始执行

### 轮次间持久化

每次 checkpoint 写入时附带 WorkflowRun 的完整字段：

```
workflow_id
definition_version
current_step
phase
step_history[]
step_data
pending_verify
```

### 从归档恢复

1. Session 从归档恢复
2. SessionManager 重建 ConversationSession
3. System Prompt Builder 重新构建 system prompt
4. Engine 检测 WorkflowRun 存在且 phase ≠ complete
5. Engine 注入 recovered 消息（role: workflow）：
   - "正在执行 {workflow_name}，当前 Step {N}。"
6. Engine 注入当前步骤 goal 消息（role: workflow）
7. Engine 通过 System Prompt Builder 重新注入 workflow context
8. Agent 从中断点继续

恢复后 verify/jump 流程由 Engine 管理（详见 execution-engine.md），Engine 根据当前 phase 决定注入内容。

### 退出 Workflow 模式

1. Workflow 正常结束（phase = complete）
2. Engine 通过 System Prompt Builder 从追加区移除 workflow context
3. Engine 清理消息历史中的 workflow 控制消息（goal + recovered）
4. Engine 清空 WorkflowRun 状态
5. Engine 触发 checkpoint 写入，持久化空状态
6. Session 恢复为普通 session

### 定义版本变更

恢复时检测到 definition_version 不匹配：

- 当前步骤编号仍存在于新定义 → 使用新定义继续
- 当前步骤编号不存在于新定义 → phase = blocked，通知 owner

## 模块关系

### 上游

- **SessionManager**：session 创建/恢复时触发 Engine 初始化。checkpoint 持久化时 Engine 写入 WorkflowRun 状态。
- **System Prompt Builder**：提供追加区注入接口，Engine 通过此接口管理 workflow context。
- **Gateway**：恢复时注入 recovered 消息需通过 Gateway 路由。

### 下游

- **Execution Engine**：从 WorkflowRun 状态恢复后继续驱动步骤执行。

### 无关

- **Compaction**：workflow 消息（除 goal）在完成后已删除，不参与压缩。Goal 消息压缩时保留。Compaction 完成后 Engine 重新注入 workflow context。
- **Memory**：workflow 不参与记忆挖掘或搜索注入。
