# Workflow 需求

## 概述

让 agent 在复杂多步骤流程中受到结构化约束——步骤不跳不漏、每步有验收标准、完成情况可追踪、中断后可恢复。用 Engine 驱动的状态机替代纯 prompt 描述，解决 agent 自行记忆和执行的不可靠问题。

## 功能需求

### F1. Workflow 定义

Owner 可以定义工作流，描述一个多步骤流程的结构化信息：包含哪些步骤、每步要完成什么、如何验收、完成后如何分支。

定义放在 agent workspace 的 `workflows/` 目录下，与 skill 共用目录结构但独立管理。每个 workflow 是一个 SKILL.md 文件——正文部分给 agent 阅读（原则和注意事项），YAML frontmatter 给 Engine 读取（步骤、验收、跳转规则）。

Owner 通过 create-workflow skill 创建和修改 workflow 定义。定义产出时需通过内置校验——覆盖步骤编号合法性、跳转规则合法性（无重复条件、有兜底分支、目标步骤存在）、验收清单完整性、枚举选项规范性等。

### F2. Workflow 启动

Workflow 有两种启动方式：

- **斜杠指令**：Owner 输入 `/workflow <名称>`，Engine 加载对应定义并启动
- **Agent 工具调用**：Agent 在对话中判断需要执行某个 workflow 时，调用 `workflow_start` 工具启动

启动后，当前 session 进入 workflow 模式。Agent 收到 Engine 注入的 workflow 上下文说明（在 system prompt 中），了解自己正在执行受控工作流以及需遵守的三阶段协议（收到 goal → 执行；收到 verify → 验收；收到 jump → 回答跳转）。

一个 session 同一时刻只能执行一个 workflow。Workflow 开始后不可降级为普通 session，必须走完或由 owner 终止。

### F3. 步骤引导执行

Workflow 启动后，Engine 按定义逐步骤推进。每个步骤 agent 收到一条 goal 消息（role: workflow），描述当前步骤要完成的目标。Agent 基于 goal 自主执行——可以调用任意工具、spawn 子 session 等，Engine 不干预。

一个步骤 = agent 一次连续执行，不受 Engine 中断。Agent 完成当前 turn、session 空闲后，Engine 进入验收阶段。

### F4. 步骤完成验证

Agent 完成步骤执行后，Engine 在 session 空闲时注入验收清单（来自步骤定义中的 verify 字段）。Agent 对照清单自查：

- **未完成**：继续执行步骤内容。Engine 不干预，等下次 session 空闲时重新注入验收清单
- **已完成**：Agent 调用 workflow_verify 声明步骤完成

验证重试有次数上限（可在 workflow 定义中配置，默认 3 次）。若 agent 连续 N 次收到验收清单仍无法通过验证，Engine 将流程转为阻塞状态并通知 owner 介入。没有超时机制——agent 执行步骤本身不受时间限制。

### F5. 流程分支控制

Agent 声明步骤完成后，Engine 注入跳转问题（来自步骤定义中的 jump 配置）。Agent 回答后，Engine 按预定义的跳转规则决定下一步：

- **前进**（goto）：进入指定步骤，清空步骤间的共享数据
- **重做**（reexecute）：重新执行指定步骤，保留共享数据
- **结束**（complete）：Workflow 执行完毕

跳转规则按顺序匹配，第一个条件满足的生效。必须有兜底分支（default）确保总是有出路。所有条件评估硬编码——布尔比对、枚举匹配、字符串比对——不依赖 LLM 做语义判断。

Agent 收到跳转问题后只需回答结构化答案，不需自己理解跳转规则。跳转问题以 ABCD 选项形式呈现，不暴露内部枚举值。

### F6. 流程暂停与恢复

Workflow 执行过程中遇到需要 owner 介入的情况时，流程暂停（blocked）：

**被动暂停**：验证重试次数耗尽后，Engine 自动暂停并通知 owner
**主动暂停**：Agent 在验收阶段判断无法继续时，可主动请求暂停（若当前步骤允许）

暂停后，Engine 通过通知告知 owner 暂停原因。Owner 回复后 Engine 解除暂停——清除旧的步骤目标，重置验证计数，立即重新注入验收清单，agent 按正常验收→跳转流程继续。

Owner 也可以选择直接终止 workflow。

### F7. 中断续跑

Workflow 状态随 session 持久化保存。系统重启或 session 归档恢复时，Engine 检测是否存在未完成的 workflow：

- 若存在且当前步骤编号仍在新定义中 → 自动恢复，Engine 注入恢复提示和当前步骤 goal，agent 从中断点继续
- 若当前步骤编号已不存在于新定义 → 转为阻塞状态，通知 owner

恢复时 Engine 重新注入 workflow 上下文到 system prompt，保证 agent 具备完整的执行上下文。

### F8. 流程生命周期

Workflow 正常结束（jump 结果为 complete）或 owner 终止后，Engine 执行退出清理：

- 从 system prompt 中移除 workflow 上下文
- 清理对话历史中的 workflow 控制消息
- 清空 Workflow 运行状态
- Session 恢复为普通 session

退出后的 session 不再受 workflow 约束，agent 恢复为自由对话模式。

## 关联设计文档

- [✓] README.md
- [✓] workflow-definition.md
- [✓] execution-engine.md
- [✓] session-integration.md
- [✓] workflow-tools.md

## 非功能需求

- **流程不丢失**：系统重启后，未完成的 workflow 必须能从断点恢复，不丢失执行进度
- **步骤不跳过**：Agent 无法自行跳过验收或绕过跳转——流程推进完全由 Engine 控制
- **通知及时**：当 workflow 转为阻塞状态等待 owner 介入时，通知必须即时送达
- **控制消息不干扰**：workflow 控制消息（验收清单、跳转问题）在完成后从对话历史中清除，不污染 agent 上下文
