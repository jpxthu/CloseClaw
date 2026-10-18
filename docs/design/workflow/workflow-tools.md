# Workflow Tools

## 概述

Workflow Tools 是 Agent 与 Engine 之间的结构化通信接口，也是 `/workflow` 斜杠入口的承载方。Agent 通过标准工具调用向 Engine 报告步骤状态，Engine 根据工具调用结果驱动状态转换；Owner 通过 `/workflow` 经 SlashDispatcher 与 WorkflowHandler 启动流程。

## 架构

### 工具清单

| 工具 | 发起方 | 用途 | 参数 |
|------|--------|------|------|
| workflow_start | Agent | 进入 workflow 模式（已有未完成 WorkflowRun 时返回错误） | name：workflow 名称 |
| workflow_verify | Agent | 声明步骤完成 | 无 |
| workflow_jump | Agent | 回答跳转问题 | 按定义的结构化答案 |
| workflow_blocked | Agent | 主动请求阻塞 | reason：阻塞原因 |

所有工具向 ToolRegistry 注册，Agent 通过标准工具调用路径与 Engine 通信。

### 进入模式触发

斜杠指令：Owner 输入 /workflow <name>，Gateway 识别 `/` 前缀并按发送者角色校验仅 Owner 可用（非 Owner 直接回复权限不足，不进 SlashDispatcher）；SlashDispatcher 解析后分派给 workflow 模块提供的 WorkflowHandler（SlashHandler 实现，非 Immediate）。WorkflowHandler 仅解析 name 并返回 `SlashResult::WorkflowStart`，由 Gateway 经 [WorkflowControl](../common/core-traits.md#workflowcontrol) 调用 Engine 启动流程，完成后回复确认消息。非 Immediate 语义为满足排队条件时在当前 LLM 调用结束后执行：Session 正忙时入队并回复等待提示，空闲后分派。

工具调用：Agent 在对话中途调用 workflow_start({name})，当前 session 转入 workflow 模式。适用场景：用户要求执行特定 workflow，Agent 自主判断并调用工具。

### 工具注册

Workflow 工具在 ToolRegistry 初始化时注册，属于系统级工具——不受 Agent 权限配置限制；加载策略为延迟加载（一级索引仅展示名称与危险度标记，详见 [tools README](../tools/README.md)）。WorkflowHandler 在 HandlerRegistry 初始化时注册 /workflow 路由。

## 数据流

### workflow_start

1. Agent 调用 workflow_start({name})；Engine 先确认当前 Session 无未完成 WorkflowRun，已有则返回错误
2. Engine 取当前 Session 绑定的 agent_id，按优先级查找定义文件（Agent 专属目录下的 `workflows/<name>/SKILL.md` → 全局 `workflows/<name>/SKILL.md` → 系统内置），三级均未命中则返回错误
3. Engine 加载并校验定义（失败则返回错误，不创建 WorkflowRun）
4. Engine 初始化 WorkflowRun：current_step 置 0，phase 置 executing
5. Engine 通过 system prompt 追加区的系统注入通道注入 workflow context（含定义文件路径），待注入完毕后经统一消息队列注入 Step 0 步骤目标消息（role: workflow）
6. 返回 tool result 确认；步骤目标消息在 Agent 当前 turn 结束后经队列送达

### workflow_verify

1. Agent 收到验收清单并自查，完成则调用 workflow_verify()（无参数）
2. Engine 检查当前 phase：不是 verifying 则返回错误；是 verifying 则 phase 转为 jumping，注入跳转问题
3. 返回 tool result（不在 verify 时立即抹除，待跳转决策完成后与跳转交互记录一并抹除）

验收清单的注入时机：Agent 当前 turn 结束且四维活跃维度均否时，Engine 进入 verifying 并注入验收清单（Owner `/stop` 造成的四维归零不触发验收）。workflow_verify 只是"我做完了"的信号；验收清单来源于 Step 定义中的 verify 字段，Engine 不校验条目真伪。

### workflow_jump

1. Agent 调用 workflow_jump({answers})
2. Engine 检查当前 phase：不是 jumping 则返回错误
3. Engine 取当前步骤定义的 transitions，按顺序匹配条件。全部不匹配则执行 default
4. Engine 执行匹配到的 action（goto/reexecute/complete）
5. Engine 抹除本轮验收清单与跳转问题的交互记录（两者的注入消息 + tool_call + tool_result）
6. Engine 更新 WorkflowRun 状态
7. Engine 注入下一步的步骤目标消息或结束
8. 返回 tool result（已随交互记录抹除）

`answers` 为以 JumpQuestion.id 为键、选项字母为值的对象（如 `{q1: "A", q2: "B"}`）；选项字母（A/B/C/D...）非选项内部值。

跳转问题来自当前步骤定义中的 jump 字段，option_labels 用于将选项内部值渲染为 ABCD 标签。Engine 收到字母答案后按 options 顺序映射回内部值，再与 transitions 的 expected_value 比对。

### workflow_blocked

1. Agent 调用 workflow_blocked({reason})
2. Engine 检查当前 phase：不是 verifying 则返回错误；再检查当前 step 的 allow_blocked：为 false 则返回错误，Agent 继续验收循环
3. Engine 将 phase 设为 blocked，通过 Gateway 系统通知接口即时向 Owner 推送 IM 通知（含 reason）
4. Engine 抹除本轮验收清单交互记录（注入消息 + tool_call + tool_result），返回 tool result

**后续：Owner 介入（异步）**

- Owner 非斜杠消息：Gateway 在消息路由到 Session 前经 WorkflowControl 通知 Engine → 解除阻塞（解除动作见 execution-engine.md「阻塞处理」），Engine 注入验收清单 → Agent 自查验收，通过后进入跳转流程
- Owner 输入 `/stop`：由 Gateway 终止 workflow（见 execution-engine.md「Owner 终止」）

### WorkflowControl 信号

除四个工具外，Engine 还接收 Gateway 经 [WorkflowControl](../common/core-traits.md#workflowcontrol) 的三类信号：`start`（执行 `SlashResult::WorkflowStart` 启动流程）、`on_owner_message`（blocked 时解除可解除暂停）、`terminate`（Owner `/stop` 终止）。信号语义见 execution-engine.md「阻塞处理」「Owner 终止」。

### 斜杠指令

1. Owner 输入 /workflow <name>
2. Gateway 识别 `/` 前缀并校验发送者为 Owner（非 Owner 直接回复权限不足）；非 Immediate 指令在 Session 正忙时先入队，空闲后分派
3. SlashDispatcher 匹配 /workflow 路由，提取 name 参数，分派给 WorkflowHandler
4. WorkflowHandler 返回 `SlashResult::WorkflowStart`（载荷 name），不直接产生副作用
5. Gateway 经 WorkflowControl 调用 Engine：确认无活跃 WorkflowRun、加载并校验定义、初始化 WorkflowRun、注入 workflow context 与 Step 0 步骤目标消息
6. 启动成功后回复确认消息；启动失败回复错误

## 模块关系

### 上游

- **Gateway / SlashDispatcher**：Gateway 识别 `/` 前缀并校验 `/workflow` 仅 Owner 可用；SlashDispatcher 解析指令并分派给 WorkflowHandler。
- **Agent**：通过标准工具调用路径调用 workflow 工具。
- **Session**：承载工具调用与 workflow 控制消息，是 Agent 与 Engine 通信的会话宿主。

### 下游

- **ToolRegistry**：workflow 工具注册到 ToolRegistry 供 Agent 发现和调用。
- **Engine**（同模块子功能）：接收工具调用结果，驱动状态机转换。
- **Session**：承载 workflow 控制消息的统一消息队列（role: workflow），供 LLM 消费。
- **Gateway**：执行 `SlashResult::WorkflowStart` 时经 [WorkflowControl](../common/core-traits.md#workflowcontrol) 调用 Engine 启动流程；blocked 状态通知 Owner 时经 [Gateway 系统通知接口](../gateway/outbound-flow.md#系统通知接口)推送 IM 通知；workflow 控制消息（role: workflow）经 Gateway 实现的 SessionLookup 进入 session 统一消息队列；Gateway 在 Owner 非斜杠消息与 Owner `/stop` 时经 WorkflowControl 通知 Engine（解除阻塞 / 终止）。

### 共享类型 / 核心 trait

- 实现 [WorkflowControl](../common/core-traits.md#workflowcontrol)：供 Gateway 在 WorkflowStart、Owner 非斜杠消息与 Owner `/stop` 入口调用。
- 实现 [SlashHandler](../common/core-traits.md#slashhandler)：WorkflowHandler 处理 `/workflow` 指令。
- 消费 [SlashSessionQuery](../common/core-traits.md#slashsessionquery)：查询/持久化 WorkflowRun 与当前活跃 workflow phase。
- 消费 [SessionLookup](../common/core-traits.md#sessionlookup)：向统一消息队列注入 workflow 控制消息（role: workflow）。

### 无关

- **Permission**：workflow 工具为系统级工具，不经过 Agent 权限检查。
- **IM Adapter**：工具调用和返回不经过消息渲染链路。
