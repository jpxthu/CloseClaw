# Workflow

## 概述

Workflow Engine 是 CloseClaw 的流程控制层，将复杂多步骤流程从纯 prompt 驱动转变为 Engine 驱动的状态机执行。Agent 负责执行步骤内容，Engine 负责追踪状态、注入步骤目标、验收完成、执行跳转。

## 架构

Workflow Engine 由四个子功能组成：

**Workflow Definition**：定义 workflow 的 yaml frontmatter 结构（Step、Verify、Jump、Transition），以及 create-workflow skill 内置的校验规则。Step 的 id 为从 0 开始的整数。

**Execution Engine**：运行时状态机，管理 executing → verifying → jumping → blocked → complete 五个 phase。Engine 按 goal→verify→jump 三阶段协议驱动步骤推进。Jump Engine 硬编码匹配 transitions 条件，执行 goto、reexecute 或 complete。

**Session Integration**：WorkflowRun 状态随 session checkpoint 持久化。重启后扫描活跃 session 检测未完成的 workflow 并注入恢复消息。进入 workflow 模式后 Engine 通过 system prompt 追加区注入 workflow context。

**Workflow Tools**：Agent 与 Engine 的通信接口——workflow_start 进入模式、workflow_verify 声明完成、workflow_jump 回答跳转问题、workflow_blocked 请求阻塞。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [workflow-definition.md](workflow-definition.md) | Workflow 定义格式：yaml frontmatter 结构、Step/Verify/Jump/Transition 字段、校验规则 |
| [execution-engine.md](execution-engine.md) | 执行引擎：状态机生命周期、goal→verify→jump 三阶段协议、跳转条件评估 |
| [session-integration.md](session-integration.md) | Session 集成：持久化字段、重启恢复、system prompt 注入、追加区格式 |
| [workflow-tools.md](workflow-tools.md) | Workflow 工具：workflow_start/verify/jump/blocked 参数与行为、斜杠指令、触发机制 |

## 数据流

### Workflow 创建与启动

Agent 调用 workflow_start 或用户输入 /workflow 指令触发。Engine 按优先级查找定义文件（agent workspace/workflows/ → .closeclaw/workflows/ → 内置），解析 yaml frontmatter 得到 Workflow 结构体。Engine 初始化 WorkflowRun（current_step=0，phase=executing），向 system prompt 追加区注入 workflow context，注入 Step 0 的 goal 消息（role: workflow），Agent 开始执行。

### 步骤执行循环

每个步骤经过三个阶段（详见 execution-engine.md）：

1. **Goal**：Engine 注入步骤目标描述（role: workflow），Agent 连续工具调用完成步骤
2. **Verify**：session idle 时 Engine 注入验收清单。Agent 自查——未完成则继续执行（等下次 idle 重新注入），完成则调 workflow_verify()，Engine 抹除本轮 verify 交互记录
3. **Jump**：Engine 注入跳转问题，Agent 回答后 Engine 抹除本轮 jump 交互记录（含注入消息），匹配 transitions 决定 goto/reexecute/complete

### Workflow 运行状态

WorkflowRun 跟踪运行时状态：workflow_id 和 definition_version 关联定义，current_step 记录当前步骤，phase 为 executing/verifying/jumping/blocked/complete，step_history 记录已完成步骤，step_data 存储跨步骤共享数据，pending_verify 记录验证重试状态。状态随 session checkpoint 持久化，重启后可恢复。

### 暂停与恢复

Agent 未完成验证时可继续执行，Engine 等下次 session idle 自动重新注入 verify。每次注入 pending_verify 计数 +1，超过上限（可在 workflow 定义中配置，默认 3 次）→ blocked 并通知 owner。

当前步骤 allow_blocked 为 true 时，Agent 可在 verify 阶段调用 workflow_blocked 主动请求阻塞，等待 owner 回复后恢复。

工作流正常结束后 Engine 清理：移除追加区中的 workflow context、清理 goal 消息、清空 WorkflowRun 状态并触发 checkpoint 持久化。

## 模块关系

### 上游

- **Session**：workflow 运行在 session 内。Engine 依赖 session 的空闲判定决定何时注入 verify；WorkflowRun 状态随 session checkpoint 持久化；session 恢复时 Engine 检测未完成 workflow 并注入恢复消息。
- **System Prompt**：进入 workflow 模式后，Engine 通过追加区注入 workflow context。恢复时重新注入保证内容最新。
- **Slash**：/workflow 斜杠指令触发 workflow 启动，由 SlashDispatcher 拦截后转发给 Engine。
- **Gateway**：Engine 通过 Gateway 将 workflow role 消息注入 session（不经入站 Processor Chain），blocked 通知 owner 时也通过 Gateway 出站。

### 下游

- **Tools**：workflow 工具注册到 ToolRegistry，Agent 通过标准工具调用与 Engine 通信。
- **Skills**：workflow 定义文件复用 skill 的目录结构和优先级查找机制，但不走 system prompt skill listing。

### 无关

- **LLM Provider**（无调用关系）：Engine 不直接调用 LLM，通过注入 workflow role 消息驱动 Agent。
- **IM Adapter**（无调用关系）：workflow 控制消息不经过出站渲染链路。
- **Memory**（无调用关系）：workflow 不参与记忆挖掘或搜索注入。
