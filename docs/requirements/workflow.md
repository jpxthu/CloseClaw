# workflow 需求

## 概述

让 Agent 在复杂多步骤流程中受到结构化约束——步骤不跳不漏、每步有验收标准、完成情况可追踪、中断后可恢复。流程推进由系统强制控制，Agent 无法自行跳过或遗忘，解决 Agent 自行记忆和执行的不可靠问题。

## 功能需求

### F1. workflow 定义

User 可以定义 workflow，描述一个多步骤流程的结构化信息：包含哪些步骤、每步要完成什么、如何验收、完成后如何分支。

每个 workflow 是一个独立定义文件，定义文件分两部分：正文给 Agent 阅读（原则和注意事项），结构化定义给 Engine 读取（步骤、验收、跳转规则）。定义文件按优先级查找：Agent workspace 的 `workflows/` 目录 > 全局 `workflows/` 目录 > 系统内置。

User 通过 create-workflow skill 创建和修改 workflow 定义。定义产出时需通过内置校验——至少覆盖：步骤编号合法性、跳转规则合法性（无重复条件、有兜底分支、目标步骤存在）、验收清单完整性、枚举选项规范性。workflow 定义文件变更后，自下一次 workflow 启动时生效；执行中的 workflow 不受定义变更影响，中断续跑的恢复视为一次启动（见 F7）。

> **交叉引用**：配置重载机制详见 [config §F4](config.md)（配置重载）。

### F2. workflow 启动

workflow 有两种启动方式：

- **斜杠指令**：Owner 输入 `/workflow <名称>`，Engine 加载对应定义并启动

> **交叉引用**：该指令由 slash 模块拦截分发，指令注册详见 [slash §F15](slash.md)（workflow 指令）。

- **Agent 工具调用**：Agent 在对话中判断需要执行某个 workflow 时，调用 `workflow_start` 工具启动

启动后，当前 Session 进入 workflow 模式。Agent 收到注入的 workflow 上下文（位于 System Prompt 追加区，追加区机制详见 [system_prompt §F5](system_prompt.md)（追加指令管理）；注入时机见本节与 F7，移除时机见 F8），了解自己正在执行受控 workflow 以及需遵守的三阶段协议（收到步骤目标消息 → 执行；收到验收清单 → 验收；收到跳转问题 → 回答跳转）。

一个 Session 同一时刻只能执行一个 workflow。workflow 开始后不可回退为普通 Session——必须由 Engine 判定 jump 结果为 complete 后正常结束，或由 Owner 主动终止。

### F3. 步骤引导执行

workflow 启动后，Engine 按定义逐步骤推进。每个步骤 Agent 收到一条步骤目标消息（workflow 控制消息），描述当前步骤要完成的目标。Agent 基于步骤目标消息自主执行——可以调用任意工具、spawn 子 Session 等，Engine 不干预。

一个步骤 = Agent 一次连续执行，不受 Engine 中断。Agent 完成当前 turn 后，且 Session 满足以下条件时，Engine 进入验收阶段。验收判定条件 = LLM 推理=false AND 同步工具等待结果=false AND 后台任务=false AND 子 Session=false。四个活跃维度由 [session §F11](session.md)（Session 活跃维度）定义。任一维度为 true 时不进入验收，Engine 等待下次判定。

### F4. 步骤完成验收

Engine 在满足验收判定条件时注入验收清单（来自步骤定义中的 verify 字段）。验收判定条件见 F3。Agent 对照清单自查：

- **未完成**：继续执行步骤内容。Engine 不干预，等下次满足验收判定条件时重新注入验收清单
- **已完成**：Agent 调用 workflow_verify 声明步骤完成

验收重试有次数上限（可在 workflow 定义中配置，默认 3 次）。若 Agent 达到重试次数上限仍无法通过验收，Engine 将流程转为暂停并通知 Owner 介入。没有超时机制——Agent 执行步骤本身不受时间限制。

### F5. 流程分支控制

Agent 声明步骤完成后，Engine 注入跳转问题（来自步骤定义中的 jump 配置）。Agent 回答后，Engine 按预定义的跳转规则决定下一步：

- **前进**（goto）：进入指定步骤
- **重做**（reexecute）：重新执行指定步骤
- **结束**（complete）：workflow 执行完毕

跳转规则按顺序匹配，第一个条件满足的规则即生效。必须有兜底分支（default）确保总是有出路。条件评估方式可预测：给定 Agent 对跳转问题的回答，Engine 按预定义规则产出确定的结果，不依赖 LLM 做语义判断。

Agent 收到跳转问题后只需提供结构化答案，不需自己理解跳转规则。跳转问题以枚举选项形式呈现，不暴露内部值。

### F6. 流程暂停与恢复

workflow 执行过程中遇到需要 Owner 介入的情况时，流程暂停（blocked）：

**被动暂停**：验收重试次数耗尽后，Engine 自动暂停并通过 IM 消息通知 Owner
**主动暂停**：Agent 在验收阶段判断无法继续时，可主动请求暂停（若当前步骤在定义中标记为允许暂停）

暂停后，Engine 通过 IM 消息告知 Owner 暂停原因。Owner 回复后 Engine 解除暂停——重置验收重试次数，重新注入验收清单，Agent 从暂停前的阶段继续（步骤目标消息保留，Agent 仍知悉当前任务）。

Owner 也可以选择直接终止 workflow。

### F7. 中断续跑

workflow 状态随 Session 持久化保存，运行状态（运行中 / 暂停中）随之一并持久化。系统重启或 Session 归档恢复时，Engine 检测是否存在未完成的 workflow，并按持久化的运行状态分别处理：

- **运行中**：若当前步骤编号仍存在于最新定义 → 自动恢复，Engine 注入恢复提示和当前步骤目标消息，Agent 从中断点继续；若当前步骤编号在最新定义中已不存在 → 转为暂停，通过 IM 消息通知 Owner
- **暂停中**：保持暂停，不自动恢复。Engine 重新通过 IM 消息告知 Owner 暂停原因，等待 Owner 处理：其中恢复时检测到当前步骤在最新定义中已不存在的，仅能由 Owner 终止；其余暂停按 F6 解除暂停，Owner 亦可直接终止

恢复时 Engine 重新注入 workflow 上下文到 System Prompt 追加区（见 F2），保证 Agent 具备完整的执行上下文。

### F8. 流程生命周期

workflow 正常结束（jump 结果为 complete）或 Owner 终止后，Engine 执行退出清理：

- 从 System Prompt 追加区移除 workflow 上下文（见 F2）
- 清理对话历史中的 workflow 控制消息
- 清空 workflow 运行状态
- Session 恢复为普通 Session，Agent 不再受 workflow 约束

## 非功能需求

- **执行进度不丢失**：系统重启后，运行中的 workflow 必须能从断点恢复，不丢失执行进度（当前步骤在最新定义中已不存在的，按 F7 转为暂停）；暂停中的 workflow 恢复后保持暂停，不丢失暂停原因与等待状态
- **步骤不跳不漏**：Agent 无法自行跳过验收或绕过跳转——流程推进完全由 Engine 控制
- **通知及时**：当 workflow 转为暂停等待 Owner 介入时，IM 通知应即时推送，不让 Owner 在不知情中等候
- **控制消息精简**：验收清单与 Agent 确认记录（一问一答）在跳转决策完成后即从对话历史中删除——这些记录已无后续语义价值。步骤目标消息保留以确保 Agent 始终知道当前任务
