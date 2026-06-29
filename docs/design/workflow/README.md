# Workflow

## 概述

Workflow Engine 是 CloseClaw 的流程控制层，将复杂多步骤流程从纯 prompt 驱动转变为 Engine 驱动的状态机执行。Agent 负责执行步骤内容，Engine 负责追踪状态、注入步骤目标、验收完成、执行跳转。

## 架构

Workflow Engine 由四个子功能组成：

**Workflow Definition**：定义 workflow 的 YAML frontmatter 结构（Step、Verify、Jump、Transition），以及 create-workflow skill 内置的校验规则。步骤编号从 0 开始。

**Execution Engine**：运行时状态机，管理 executing、verifying、jumping、blocked、complete 五个 phase。Engine 按三阶段协议（goal → verify → jump）驱动步骤推进。跳转条件评估全硬编码——按 transitions 顺序做布尔/枚举/字符串比对，不依赖 LLM。

**Session Integration**：WorkflowRun 状态随 session checkpoint 持久化。重启后扫描活跃 session 检测未完成的 workflow 并注入恢复消息。进入 workflow 模式后 Engine 通过 system prompt 追加区注入 workflow context。

**Workflow Tools**：Agent 与 Engine 的通信接口——workflow_start 进入模式、workflow_verify 声明完成、workflow_jump 回答跳转问题、workflow_blocked 请求阻塞。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [workflow-definition.md](workflow-definition.md) | Workflow 定义格式：YAML frontmatter 结构、Step/Verify/Jump/Transition 字段、校验规则 |
| [execution-engine.md](execution-engine.md) | 执行引擎：状态机生命周期、三阶段协议、跳转条件评估、阻塞处理 |
| [session-integration.md](session-integration.md) | Session 集成：持久化字段、重启恢复、system prompt 注入、消息管理 |
| [workflow-tools.md](workflow-tools.md) | Workflow 工具：workflow_start/verify/jump/blocked 参数与行为、斜杠指令、触发机制 |

## 数据流

### Workflow 创建与启动

用户输入 /workflow 指令或 Agent 调用 workflow_start 工具触发。Engine 按优先级查找定义文件（agent workspace 下的 workflows/ 目录 → .closeclaw/workflows/ → 内置），解析 YAML frontmatter 得到 Workflow 结构体。Engine 初始化 WorkflowRun（当前步骤为 0，phase 为 executing），向 system prompt 追加区注入 workflow context，随后注入 Step 0 的 goal 消息（role: workflow）。Step 0 的 goal 注入需确保 system prompt 已更新完毕，Agent 收到首条 workflow 消息时已具备 workflow context。定义文件三级均未命中则返回错误。

### 步骤执行循环

每个步骤经过三个阶段（详见 execution-engine.md）：

1. **executing 阶段**：Engine 注入步骤目标描述（role: workflow），Agent 连续工具调用完成步骤内容。Engine 在 Agent 执行期间不干预。
2. **verifying 阶段**：session idle 时（判定由 Session 模块统一管理），Engine 注入验收清单。Agent 自查——未完成则继续执行，等下次 idle 重新注入验证清单；完成则调用 workflow_verify，Engine 抹除本轮 verify 交互记录（注入消息 + tool_call + tool_result），进入 jumping。
3. **jumping 阶段**：Engine 注入跳转问题，Agent 回答后 Engine 匹配 transitions 决定下一步（goto/reexecute/complete），抹除本轮 jump 交互记录（含注入消息），更新状态，注入新步骤 goal 或结束。

### 暂停与恢复

Agent 未完成验证时可继续执行。Engine 等待下次 session idle 自动重新注入 verify 消息。每次注入 pending_verify 计数加一，超过上限（默认 3 次，可在 workflow 定义中配置）则 phase 转为 blocked 并通知 owner。Owner 回复后 Engine 解除阻塞，pending_verify 归零，移除旧 goal，立即注入 verify 消息。

当前步骤 allow_blocked 为 true 时，verify 消息末尾附加 "如果确认任务无法继续，调用 workflow_blocked" 提示。Agent 调用 workflow_blocked 后 phase 转为 blocked 并通知 owner。Owner 回复后 Engine 解除阻塞，pending_verify 归零，移除旧 goal，立即注入 verify 消息。如果 Agent 调用 workflow_blocked 时当前步骤不允许 blocked，Engine 返回错误，Agent 继续 verify 循环。

若 owner 选择终止 workflow，phase 转为 complete，Engine 执行退出清理。

### Workflow 结束

正常结束（jump 结果为 complete）或 owner 终止后，Engine 从追加区移除 workflow context，清理消息历史中的 workflow 控制消息（goal + recovered），清空 WorkflowRun 状态，主动触发一次 checkpoint 持久化空状态，session 恢复为普通 session。

## 模块关系

### 上游

- **Session**：workflow 运行在 session 内。Engine 依赖 session 的空闲判定决定何时注入 verify；WorkflowRun 状态随 session checkpoint 持久化；session 恢复时 Engine 检测未完成 workflow 并注入恢复消息。
- **System Prompt**：进入 workflow 模式后，Engine 通过追加区注入 workflow context。恢复和 compaction 后重新注入保证内容最新。
- **Slash**：/workflow 斜杠指令触发 workflow 启动，由 SlashDispatcher 拦截后转发给 Engine。
- **Gateway**：Engine 通过 Gateway 将 workflow role 消息注入 session（不经入站 Processor Chain），blocked 通知 owner 时也通过 Gateway 出站。

### 下游

- **Tools**：workflow 工具注册到 ToolRegistry，Agent 通过标准工具调用与 Engine 通信。
- **Skills**：workflow 定义文件复用 skill 的目录结构和优先级查找机制，但不走 system prompt skill listing。

### 无关

- **LLM Provider**：Engine 不直接调用 LLM，通过注入 workflow role 消息驱动 Agent。
- **IM Adapter**：workflow 控制消息不经过出站渲染链路。
- **Memory**：workflow 不参与记忆挖掘或搜索注入。
