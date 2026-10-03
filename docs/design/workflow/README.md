# Workflow

## 概述

- 关联需求文档：[requirements/workflow.md](../../requirements/workflow.md)
- Workflow Engine 是 CloseClaw 的流程控制层，将复杂多步骤流程从纯 prompt 驱动转变为 Engine 驱动的状态机执行。Agent 负责执行步骤内容，Engine 负责追踪状态、注入步骤目标、验收完成、执行跳转，并在需要 Owner 介入时暂停流程并主动通知。

## 架构

Workflow Engine 由四个子功能组成：

**Workflow Definition**：定义 workflow 的 YAML frontmatter 结构（Step、Verify、Jump、Transition），以及 create-workflow skill 内置的校验规则。步骤编号从 0 开始。

**Execution Engine**：运行时状态机，管理 executing、verifying、jumping、blocked、complete 五个 phase。Engine 按三阶段协议（步骤目标 → 验收 → 跳转）驱动步骤推进。跳转条件评估全硬编码——按 transitions 顺序做布尔/枚举比对，不依赖 LLM。

**Session Integration**：WorkflowRun 状态随 session checkpoint 持久化。系统重启或 Session 归档恢复时，检测未完成的 workflow 并按持久化运行状态恢复。进入 workflow 模式后 Engine 通过 system prompt 追加区注入 workflow context。

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

用户输入 /workflow 指令或 Agent 调用 workflow_start 工具触发。Engine 先确认当前 Session 无未完成 WorkflowRun（一个 Session 同一时刻只执行一个 workflow，已入模则拒绝新的入模请求）；随后按优先级查找定义文件（Agent 专属目录下的 workflows/ 目录 → 全局 workflows/ 目录 → 系统内置），解析 YAML frontmatter 得到 Workflow 结构体。Engine 初始化 WorkflowRun（当前步骤为 0，phase 为 executing），向 system prompt 追加区注入 workflow context，待追加区写入完毕后注入 Step 0 的步骤目标消息（role: workflow）——保证 Agent 收到首条 workflow 消息时已具备 workflow context。定义文件三级均未命中则返回错误。

### 步骤执行循环

每个步骤经过三个阶段（详见 execution-engine.md）：

1. **executing 阶段**：Engine 注入步骤目标消息（role: workflow），Agent 连续工具调用完成步骤内容。Engine 在 Agent 执行期间不干预。
2. **verifying 阶段**：满足验收判定条件时（四维活跃维度均否，判断见 execution-engine.md「验收时机」），Engine 注入验收清单。Agent 自查——未完成则继续执行，Engine 等下次验收判定条件满足时先移除上一条验收清单再重新注入；完成则调用 workflow_verify，进入 jumping（本轮验收交互记录暂留，待跳转决策完成后统一抹除）。
3. **jumping 阶段**：Engine 注入跳转问题，Agent 回答后 Engine 匹配 transitions 决定下一步（goto/reexecute/complete），随即抹除本轮验收清单与跳转问题的交互记录（各自的注入消息 + tool_call + tool_result），更新状态，注入新步骤的步骤目标消息或结束。

### 暂停与恢复

Agent 未完成验收时可继续执行。Engine 待下次验收判定条件满足时先移除上一条验收清单再重新注入。每次注入 pending_verify 计数加一，达到上限（默认 3 次，可在 workflow 定义中配置）则 phase 转为 blocked 并通知 Owner。

当前步骤 allow_blocked 为 true 时，验收清单末尾附加 "如果确认任务无法继续，调用 workflow_blocked({reason: "原因"})" 提示。Agent 调用 workflow_blocked 后 phase 转为 blocked 并通知 Owner。如果 Agent 调用 workflow_blocked 时当前步骤不允许 blocked，Engine 返回错误，Agent 继续验收循环。

上述两种暂停（验收重试耗尽、Agent 主动阻塞）的解除动作一致——Owner 回复后 Engine 解除阻塞（解除动作唯一定义见 execution-engine.md「阻塞处理」）。

恢复时若检测到当前步骤在最新定义中已不存在（定义版本变更），phase 转为 blocked，且该暂停仅能由 Owner 终止、不可解除续跑（判定与处置见 session-integration.md「定义版本变更」）。

若 Owner 选择终止 workflow，phase 转为 complete，Engine 执行退出清理。

### Workflow 结束

正常结束（跳转结果为 complete）或 Owner 终止后，Engine 从追加区移除 workflow context，清理消息历史中的 workflow 控制消息（步骤目标消息、恢复提示，以及终止时仍在飞的验收清单、跳转问题交互记录），清空会话中的 workflow 运行状态（WorkflowRun），并主动触发一次 checkpoint 写入以持久化该清空后的状态，session 恢复为普通 session。

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

### 上游

- **Session**：workflow 运行在 session 内。Engine 依据 Session 的四维活跃维度判断验收时机并决定何时注入验收清单；WorkflowRun 状态随 session checkpoint 持久化；系统重启或 Session 归档恢复时，Engine 检测未完成 workflow 并注入恢复消息。
- **System Prompt**：进入 workflow 模式后，Engine 通过追加区注入 workflow context。恢复和 compaction 后重新注入保证内容最新。
- **Slash**：/workflow 斜杠指令（仅 Owner 可用）触发 workflow 启动，由 SlashDispatcher 拦截后转发给 Engine。
- **Gateway**：Engine 通过 Gateway 将 workflow role 消息注入 session（不经入站 Processor Chain），blocked 通知 Owner 时也通过 Gateway 出站；此为数据流关系，Engine 不直接依赖 gateway crate。

### 下游

- **Tools**：workflow 工具（系统级工具，不受 Agent 权限配置限制）注册到 ToolRegistry，Agent 通过标准工具调用与 Engine 通信。
- **Skills**：workflow 定义与技能隔离同维度——按 Agent 隔离（Agent 专属目录对该 Agent 全部 User 共享），但不参与 skill 注册与 system prompt skill listing。

### 无关

- **LLM Provider**：Engine 不直接调用 LLM，通过注入 workflow role 消息驱动 Agent。
- **IM Adapter**：workflow 控制消息不经过出站渲染链路。
- **Memory**：workflow 不参与记忆挖掘或搜索注入。
- **Compaction**：workflow 控制消息（步骤目标消息除外）在完成后已删除，不参与压缩；步骤目标消息压缩时保留。Compaction 完成后 Engine 重新注入 workflow context。

### 共享类型

无——`WorkflowRun` 为 workflow 模块的领域内部状态，不作为 common 共享类型定义。其跨模块可见性经类型擦除机制承载：

- **跨模块流转**：`WorkflowRun` 以类型擦除形态经 [common 的 SlashSessionQuery](../common/core-traits.md#slashsessionquery) 跨模块流转（该接口由 gateway 提供实现、slash 消费）——common 不依赖 workflow 领域类型。类型擦除使 workflow 领域状态无需作为 common 共享类型定义即可供消费方访问。
- **持久化**：`WorkflowRun` 随 Session checkpoint 持久化（恢复流程详见 [session-integration.md](session-integration.md)）。
