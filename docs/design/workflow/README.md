# Workflow

## 概述

- 关联需求文档：[requirements/workflow.md](../../requirements/workflow.md)
- Workflow 是 CloseClaw 的流程控制层，将复杂多步骤流程从纯 prompt 驱动转变为 Engine 驱动的状态机执行。Agent 负责执行步骤内容，Engine 负责追踪状态、注入步骤目标与验收清单、驱动验收和跳转；需要 Owner 介入时暂停流程，并通过 Gateway 系统通知接口即时通知 Owner。

## 架构

Workflow 由四个子功能组成：

**Workflow Definition**：定义 workflow 的 YAML frontmatter 结构（Step、Verify、Jump、Transition），以及 create-workflow skill 内置的校验规则。步骤编号从 0 开始。

**Execution Engine**：运行时状态机，管理 executing、verifying、jumping、blocked、complete 五个 phase。常规执行按三阶段协议（步骤目标 → 验收 → 跳转）循环，blocked 为等待 Owner 的暂停态，complete 为终态。跳转条件按 transitions 顺序做枚举值比对，不依赖 LLM。

**Session Integration**：WorkflowRun 状态随 session checkpoint 持久化。系统重启或 Session 归档恢复时，检测未完成的 workflow 并按持久化运行状态恢复。进入 workflow 模式后 Engine 通过 system prompt 追加区的系统注入通道（详见 [session-integration.md](session-integration.md)「System Prompt 注入」）注入 workflow context。

**Workflow Tools**：Agent 与 Engine 的通信接口——workflow_start 进入模式、workflow_verify 声明完成、workflow_jump 回答跳转问题、workflow_blocked 请求阻塞；同时承载 `/workflow` 斜杠入口的 WorkflowHandler。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [workflow-definition.md](workflow-definition.md) | Workflow 定义格式：YAML frontmatter 结构、Step/Verify/Jump/Transition 字段、校验规则 |
| [execution-engine.md](execution-engine.md) | 执行引擎：状态机生命周期、三阶段协议、跳转条件评估、阻塞处理 |
| [session-integration.md](session-integration.md) | Session 集成：持久化字段、重启恢复、system prompt 注入、消息管理 |
| [workflow-tools.md](workflow-tools.md) | Workflow 工具：workflow_start/verify/jump/blocked 参数与行为、斜杠指令、触发机制 |

## 数据流

### Workflow 创建与启动

用户输入 /workflow 指令或 Agent 调用 workflow_start 工具触发。Engine 确认当前 Session 无未完成 WorkflowRun（同一时刻只执行一个 workflow），加载、校验并解析定义、初始化 WorkflowRun（current_step=0，phase=executing），向追加区注入 workflow context，再经统一消息队列注入 Step 0 步骤目标消息。定义查找与校验见 workflow-definition.md，工具调用细节见 workflow-tools.md，入模与恢复见 session-integration.md。

### 步骤执行循环

每个步骤经过 executing → verifying → jumping 三个阶段：Engine 在每阶段注入对应 workflow 控制消息（步骤目标 / 验收清单 / 跳转问题），Agent 按三阶段协议响应，跳转决策完成后抹除本轮验收与跳转交互记录。协议细节、验收判定与消息抹除规则见 execution-engine.md。

### 暂停与恢复

Agent 未完成验收时可继续执行。Engine 待下次验收判定条件满足时先移除上一条验收清单再重新注入。每次注入 pending_verify 计数加一，达到上限（默认 3 次，可在 workflow 定义中配置）则 phase 转为 blocked 并通知 Owner。

当前步骤 allow_blocked 为 true 时，验收清单末尾附加 "如果确认任务无法继续，调用 workflow_blocked({reason: "原因"})" 提示。Agent 调用 workflow_blocked 后 phase 转为 blocked 并通知 Owner。如果 Agent 调用 workflow_blocked 时当前步骤不允许 blocked，Engine 返回错误，Agent 继续验收循环。

上述两种暂停（验收重试耗尽、Agent 主动阻塞）的解除动作一致——Owner 发送非斜杠消息后 Engine 解除阻塞（斜杠指令不解除阻塞；解除动作唯一定义见 execution-engine.md「阻塞处理」）。

Owner 输入 `/stop`（运行中或暂停中均可）时终止 workflow；非 Owner 的 `/stop` 与其他停止入口（父 Session 停止、系统关闭）只停止运行、不终止 workflow。终止语义与差异见 [execution-engine.md](execution-engine.md)「Owner 终止」。

### 恢复（系统重启与归档）

Session 重启或归档恢复时，Engine 检测未完成 workflow：运行中的 workflow 在当前步骤仍存在时注入 recovered 恢复提示、重新注入 workflow context 与步骤目标消息，从中断点继续；当前步骤已不存在时转为 blocked，仅能由 Owner 终止。暂停中的 workflow 保持 blocked，重新告知 Owner 暂停原因；当前步骤已不存在时同样仅能由 Owner 终止。完整判定见 [session-integration.md](session-integration.md)「系统重启与归档恢复」「定义版本变更」。

### 退出 Workflow 模式

正常结束（跳转结果为 complete）或 Owner 通过 `/stop` 终止后，Engine 执行退出清理（移除 workflow context、清理控制消息、清空 WorkflowRun 并持久化），Session 恢复为普通 session；完整步骤见 [session-integration.md](session-integration.md)「退出 Workflow 模式」。

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

### 上游

- **Session**：workflow 运行在 session 内。Engine 依据 Session 的四维活跃维度判断验收时机并决定何时注入验收清单；WorkflowRun 状态随 session checkpoint 持久化；系统重启或 Session 归档恢复时，Engine 检测未完成 workflow 并注入恢复消息。
- **System Prompt**：进入 workflow 模式后，Engine 通过追加区注入 workflow context。恢复和 compaction 后重新注入保证内容最新。
- **Slash**：`/workflow` 斜杠指令（仅 Owner 可用、非 Immediate）由 workflow 模块提供的 WorkflowHandler（SlashHandler 实现）处理；WorkflowHandler 仅解析 name 并返回 `SlashResult::WorkflowStart`，由 Gateway 经 WorkflowControl 调用 Engine 启动流程。
- **Gateway**：Engine 经 [SessionLookup](../common/core-traits.md#sessionlookup) 的统一消息队列注入 workflow 控制消息（role: workflow）（Gateway 的 SessionManager 实现该接口，消息不经入站 Processor Chain）；blocked 通知经 [Gateway 系统通知接口](../gateway/outbound-flow.md#系统通知接口)向 Owner 推送 IM 通知。Gateway 在 Owner 非斜杠消息路由前与 Owner `/stop` 停止完成后经 [WorkflowControl](../common/core-traits.md#workflowcontrol) 通知 Engine（解除阻塞 / 终止）。此为数据流关系，Engine 不直接依赖 gateway crate。
- **Skills**：Skills 模块提供的 create-workflow skill 产出和修改 workflow 定义（需求 F1）。workflow 定义按 Agent 隔离与技能同维度，但不参与 skill 注册与 system prompt skill listing。
- **Compaction**：压缩完成后触发 System Prompt 重建，Engine 重新注入 workflow context，并按当前 phase 重新注入必要控制消息（步骤目标、验收清单或跳转问题）；workflow 控制消息不参与压缩。

### 下游

- **Tools**：workflow 工具（系统级工具，不受 Agent 权限配置限制）注册到 ToolRegistry，Agent 通过标准工具调用与 Engine 通信。

### 无关

- **LLM Provider**：Engine 不直接调用 LLM，通过注入 workflow 控制消息（role: workflow）驱动 Agent。
- **IM Adapter**：workflow 控制消息不经过出站渲染链路。
- **Memory**：workflow 不参与记忆挖掘或搜索注入。

### 共享类型 / 核心 trait

- 实现 [WorkflowControl](../common/core-traits.md#workflowcontrol)：Gateway 在 WorkflowStart、Owner 非斜杠消息与 Owner `/stop` 终止入口调用。
- 实现 [SlashHandler](../common/core-traits.md#slashhandler)：WorkflowHandler 处理 `/workflow` 指令。
- 消费 [SessionLookup](../common/core-traits.md#sessionlookup)：向统一消息队列注入 workflow 控制消息（role: workflow）。
- 消费 [SlashSessionQuery](../common/core-traits.md#slashsessionquery)：查询/持久化 WorkflowRun 与当前活跃 workflow phase。

无共享类型——`WorkflowRun` 为 workflow 模块的领域内部状态，不作为 common 共享类型定义。其跨模块可见性经类型擦除机制承载：

- **跨模块流转**：`WorkflowRun` 以类型擦除形态经 [common 的 SlashSessionQuery](../common/core-traits.md#slashsessionquery) 跨模块流转（该接口由 gateway 提供实现、slash 与 workflow 消费）——common 不依赖 workflow 领域类型。类型擦除使 workflow 领域状态无需作为 common 共享类型定义即可供消费方访问。
- **持久化**：`WorkflowRun` 随 Session checkpoint 持久化（恢复流程详见 [session-integration.md](session-integration.md)）。
