# Workflow

## 概述

Workflow Engine 是 CloseClaw 的流程控制层，将复杂多步骤流程从纯 prompt 驱动转变为 Engine 驱动的状态机执行。Agent 负责执行步骤内容，Engine 负责追踪状态、注入步骤目标、验收完成、执行跳转。

## 架构

Workflow Engine 由四个子功能组成：

```
Workflow Engine
  │
  ├── Workflow Definition  ← workflow 定义格式（yaml frontmatter）
  │     ├── Step：id（从 0 开始的整数）、type、goal、verify、jump、transitions
  │     └── Validation：校验规则（create-workflow skill 内置）
  │
  ├── Execution Engine     ← 运行时状态机
  │     ├── Phase：executing → verifying → jumping → blocked → complete
  │     ├── Protocol：goal 注入 → verify 自查 → jump 问答
  │     └── Jump Engine：硬编码匹配 transitions 条件，执行 goto/reexecute/complete
  │
  ├── Session Integration  ← session 生命周期集成
  │     ├── Persistence：WorkflowRun 随 session checkpoint 持久化
  │     ├── Recovery：重启后扫描活跃 session，注入恢复消息
  │     └── System Prompt：workflow context 注入追加区
  │
  └── Workflow Tools       ← Agent 与 Engine 的通信接口
        ├── workflow_start：进入 workflow 模式
        ├── workflow_verify：声明步骤完成
        ├── workflow_jump：回答跳转问题
        └── workflow_blocked：主动请求阻塞
```

### 子功能索引

| 文档 | 内容 |
|------|------|
| [workflow-definition.md](workflow-definition.md) | Workflow 定义格式：yaml frontmatter 结构、Step/Verify/Jump/Transition 字段、校验规则 |
| [execution-engine.md](execution-engine.md) | 执行引擎：状态机生命周期、goal→verify→jump 三阶段协议、跳转条件评估 |
| [session-integration.md](session-integration.md) | Session 集成：持久化字段、重启恢复、system prompt 注入、追加区格式 |
| [workflow-tools.md](workflow-tools.md) | Workflow 工具：workflow_start/verify/jump/blocked 参数与行为、斜杠指令、触发机制 |

## 数据流

### Workflow 创建与启动

```
Agent 调 workflow_start({name}) 或用户输入 /workflow <name>
  ↓
Engine 加载 workflow 定义（workspace/workflows/ 或 .closeclaw/workflows/）
  ↓
Engine 向 system prompt 追加区注入 workflow context（复用 /system add 路径）
  ↓
Engine 注入 Step 0 goal 消息（role: workflow）
  ↓
Agent 开始执行步骤
```

### 步骤执行循环

```
[Engine] 注入 goal（role: workflow, type: goal）
  ↓
[Agent] 执行步骤（连续工具调用，Engine 不干预）
  ↓ Agent turn 结束，session idle
[Engine] 注入 verify（验收清单）
  ↓
[Agent] 自查
  ├─ 未完成 → 继续执行 → 等下次 idle → Engine 再注入 verify
  └─ 完成 → workflow_verify()
      ↓
[Engine] 抹除 verify 交互记录
  ↓
[Engine] 注入 jump（跳转问题）
  ↓
[Agent] workflow_jump({answers})
  ↓
[Engine] 评估 transitions → goto / reexecute / complete
  ↓
[Engine] 抹除 jump 交互记录
  ↓
[Engine] 更新 WorkflowRun 状态
  ↓
[Engine] 注入下一步 goal
  ↓
循环...
```

### Workflow 运行状态

WorkflowRun 跟踪 workflow 的运行时状态：

- `workflow_id` + `definition_version`：关联定义
- `current_step`：当前步骤编号
- `phase`：executing / verifying / jumping / blocked / complete
- `step_history`：已完成步骤记录（含时间戳）
- `step_data`：跨步骤共享的运行时数据
- `pending_verify`：验证重试计数和最后注入时间

状态随 session checkpoint 持久化，重启后可恢复。

### 暂停与恢复

Agent 未完成验证时可继续执行——Engine 等下次 session idle 自动重新注入 verify。

超时死锁保护作为兜底：

```
Engine 连续多次注入 verify 后 Agent 仍未响应
  → Engine 等待超时（可配置，默认 5 分钟）
  → 重新注入 verify（pending_verify 计数 +1）
  → 超过重试上限（默认 3 次）→ phase = blocked → 通知 owner
```

Agent 也可主动请求阻塞：

```
Agent 主动阻塞：
  Agent → workflow_blocked({reason})
  Engine → phase = blocked → 通知 owner

死锁保护：
  Engine 注入 verify → 等待超时 → 重新注入（最多 N 次）
  超过重试上限 → phase = blocked → 通知 owner

重启恢复：
  Engine 扫描活跃 session → 发现未完成 workflow
  → 重建 WorkflowRun → 注入 recovered 消息 → Agent 继续
```

## 模块关系

### 上游

- **Session**：workflow 运行在 session 内。Engine 依赖 session 的空闲判定（三维执行状态）决定何时注入 verify；WorkflowRun 状态随 session checkpoint 持久化；session 恢复时 Engine 检测未完成 workflow 并注入恢复消息。
- **System Prompt**：进入 workflow 模式后，Engine 通过追加区注入 workflow context（workflow 名称、阶段协议约束）。恢复时重新注入保证内容最新。
- **Slash**：`/workflow <name>` 斜杠指令触发 workflow 启动，由 SlashDispatcher 在 Gateway 层拦截后转发给 Engine。
- **Gateway**：workflow 模式下的消息路由——workflow role 消息由 Engine 直接注入，不经过入站 Processor Chain。

### 下游

- **Tools**：workflow 工具（workflow_start/verify/jump/blocked）注册到 ToolRegistry，Agent 通过标准工具调用与 Engine 通信。
- **Skills**：workflow 定义文件复用 skill 的目录结构和优先级查找机制，但不走 system prompt skill listing（workflow 有独立注入路径）。

### 无关

- **LLM Provider**（无调用关系）：Engine 不直接调用 LLM，通过注入 workflow role 消息驱动 Agent。
- **IM Adapter**（无调用关系）：workflow 控制消息不经过出站渲染链路。
- **Memory**（无调用关系）：workflow 不参与记忆挖掘或搜索注入。
