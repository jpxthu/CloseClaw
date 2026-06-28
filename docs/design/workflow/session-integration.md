# Session Integration

## 概述

Workflow Engine 深度集成在 session 生命周期中：WorkflowRun 状态随 session checkpoint 持久化，重启后可从断点恢复。进入 workflow 模式后，Engine 通过 system prompt 追加区注入 workflow context。

## 架构

### 持久化数据

WorkflowRun 作为 session 的附加状态随 session checkpoint 持久化：

- `workflow_id`：关联的 workflow 定义标识
- `definition_version`：定义版本号，用于检测定义变更
- `current_step`：当前步骤编号
- `phase`：executing / verifying / jumping / blocked / complete
- `step_history`：已完成步骤记录（步骤编号、进入时间、完成时间、状态）
- `step_data`：跨步骤共享的运行时数据（键值对）
- `pending_verify`：验证重试状态（注入次数、最后注入时间、最大重试次数）

持久化时机与 session checkpoint 一致——在对话轮次间写入。

### System Prompt 注入

进入 workflow 模式后，Engine 向 system prompt 追加区注入 workflow context。复用 `/system add` 的注入路径：

```
--- WORKFLOW ---
你正在执行受控工作流：{workflow_name}
描述：{description}
Engine 会通过 workflow 角色消息驱动步骤推进，必须遵守三阶段协议：
1. 收到 goal → 执行步骤
2. 收到 verify → 自查验收清单 → 完成则调 workflow_verify()，否则继续
3. 收到 jump → 回答问题 → 调 workflow_jump({answers})
不要自行跳步或跳过验证。
--- WORKFLOW END ---
```

注入与其他追加条目共存于追加区，以 `--- WORKFLOW ---` / `--- WORKFLOW END ---` 边界标记分隔。

注入时机：
- 首次进入 workflow 模式（斜杠指令或工具调用）
- Session 从 归档恢复时（重新注入，保证内容最新）
- Compaction 完成后（system prompt 重建时重新注入）

### 消息管理

Workflow 控制消息（role: workflow）独立于普通对话消息：

- **goal 消息**：保留在 context 中，不参与压缩（Agent 需要知道当前步骤目标）
- **verify/jump 消息**：Engine 处理完成后从消息历史中删除，不占用 context
- **workflow_verify/jump 工具调用**：对应的 tool_call 和 tool_result 在完成后删除

## 数据流

### 进入 Workflow 模式

```
/workflow <name> 或 workflow_start({name})
  ↓
Engine 加载 workflow 定义
  ↓
Engine 在 WorkflowRun 中初始化状态：
  current_step = 0
  phase = executing
  step_history = []
  ↓
Engine 向 system prompt 追加区注入 workflow context
  ↓
Engine 向消息历史注入 Step 0 goal 消息
  ↓
Agent 开始执行
```

### 轮次间持久化

```
每次 checkpoint 写入：
  SessionCheckpoint.workflow_state = WorkflowRun {
    workflow_id,
    definition_version,
    current_step,
    phase,
    step_history,
    step_data,
    pending_verify,
  }
```

### 从归档恢复

```
Session 从归档恢复
  ↓
SessionManager 重建 ConversationSession
  ↓
System Prompt Builder 重新构建 system prompt
  ↓
Engine 检测到 WorkflowRun 存在且 phase != complete
  ↓
Engine 注入 recovered 消息（role: workflow）：
  "正在执行 {workflow_name}，当前 Step {N}。"
  ↓
Engine 注入当前步骤 goal 消息（role: workflow）
  ↓
Engine 向 system prompt 追加区重新注入 workflow context
  ↓
Agent 从中断点继续
```

恢复后 verify 和 jump 的交互流程由 Engine 管理（详见 [execution-engine.md](execution-engine.md)），Engine 根据当前 phase 决定注入 verify 或 jump。

### 退出 Workflow 模式

```
Workflow 正常结束（phase = complete）
  ↓
Engine 清理：
  - 从 system prompt 追加区移除 workflow context
  - 清理消息历史中的 workflow goal 消息
  - 清空 WorkflowRun 状态
  - 触发一次 checkpoint 写入，将空状态持久化
  ↓
Session 恢复为普通 session
```

### 定义版本变更

```
恢复时 Engine 检测到 definition_version 不匹配：
  ├─ 当前步骤编号 在新定义中仍存在 → 使用新定义继续
  └─ 当前步骤编号 在新定义中不存在 → phase = blocked，通知 owner
```

## 模块关系

### 上游

- **SessionManager**：session 创建/恢复时触发 Engine 初始化。checkpoint 持久化时 Engine 写入 WorkflowRun 状态。
- **System Prompt Builder**：提供追加区注入接口，Engine 通过此接口管理 workflow context。
- **Gateway**：session 恢复时，如需注入 recovered 消息，通过 Gateway 路由。

### 下游

- **Execution Engine**：从 WorkflowRun 状态恢复后，继续驱动步骤执行。verify 和 jump 的交互流程由执行引擎管理（详见 [execution-engine.md](execution-engine.md)）。

### 无关

- **Compaction**（无调用关系）：workflow 消息（除 goal）在完成后已删除，不参与压缩。Goal 消息为单条 workflow role 消息，压缩时保留。Compaction 完成后 system prompt 重建时 Engine 重新注入 workflow context。
- **Memory**（无调用关系）：workflow 不参与记忆挖掘或搜索注入。
