# Workflow Tools

## 概述

Workflow Tools 是 Agent 与 Engine 之间的结构化通信接口。Agent 通过标准工具调用向 Engine 报告步骤状态，Engine 根据工具调用结果驱动状态转换。

## 架构

### 工具清单

| 工具 | 发起方 | 用途 | 参数 |
|------|--------|------|------|
| workflow_start | Agent | 进入 workflow 模式 | name：workflow 名称 |
| workflow_verify | Agent | 声明步骤完成 | 无 |
| workflow_jump | Agent | 回答跳转问题 | 按定义的结构化答案 |
| workflow_blocked | Agent | 主动请求阻塞 | reason：阻塞原因 |

所有工具向 ToolRegistry 注册，Agent 通过标准工具调用路径与 Engine 通信。

### 触发机制

斜杠指令：用户输入 /workflow <name>，由 SlashDispatcher 在 Gateway 层拦截，转发给 Engine 加载定义并初始化 WorkflowRun。

工具调用：Agent 在对话中途调用 workflow_start，当前 session 转入 workflow 模式。适用场景：用户要求执行特定 workflow 时 Agent 自主判断并调用工具。

### 工具注册

Workflow 工具在 ToolRegistry 初始化时注册，属于系统级工具——不受 Agent permission 配置限制。斜杠指令在 SlashDispatcher 初始化时注册 /workflow 路由。

## 数据流

### workflow_start

Agent 调用 workflow_start 传入 workflow 名称。Engine 按优先级查找定义文件（命中即用，不再继续）：首先查 agent workspace 下的 workflows 目录，其次查 .closeclaw 目录下的 workflows 目录，最后查内置。Engine 解析 YAML frontmatter 得到 Workflow 结构体，初始化 WorkflowRun（current_step 为 0，phase 为 executing），向 system prompt 追加区注入 workflow context，向消息历史注入 Step 0 的 goal 消息。返回 tool result 确认 workflow 已启动。

### workflow_verify

Agent 调用 workflow_verify 不带参数。Engine 检查当前 phase 是否为 verifying——不是则返回错误"当前不在验证阶段"，是则将 phase 转为 jumping 并注入跳转问题。

verify 只是 Agent 声明"我完成了"的信号。验收清单来源于 Step 定义中的 verify 字段（纯文本条目列表），Engine 不校验清单条目的真伪。

Engine 执行 phase 转换：workflow_start 初始化时 phase 为 executing，Agent turn 结束 session idle 后 Engine 将 phase 转为 verifying 并注入验收清单——此转换机制由 Engine 自动管理（详见 execution-engine.md）。

### workflow_jump

Agent 调用 workflow_jump 传入结构化答案。答案格式由 jump 问题定义中的 type 决定：boolean 类型传 true 或 false，enum 类型传 "A"/"B"/"C"/"D"（对应选项序号，非 option 内部值），string 类型传自由文本。

Engine 检查当前 phase 是否为 jumping，否则返回错误。相位正确则取当前步骤定义的 transitions，按顺序匹配条件，执行匹配到的 action（goto/reexecute/complete）。Engine 更新 WorkflowRun 状态并注入下一步 goal 或结束。

### workflow_blocked

Agent 调用 workflow_blocked 传入阻塞原因。Engine 将 phase 转为 blocked，通过 Gateway 向 owner 发送通知（含原因），返回 tool result 确认已阻塞。

阻塞解除由 owner 通过对话消息输入后，Engine 评估回复，转为 verifying 并注入 verify 消息继续流程。

### 斜杠指令

用户输入 /workflow <name>。SlashDispatcher 匹配 /workflow 路由并提取 name 参数，转发给 Engine。Engine 执行与 workflow_start 相同的流程，向用户返回确认消息。

## 模块关系

### 上游

- **SlashDispatcher**：注册 /workflow 路由，拦截斜杠指令后转发给 Engine。
- **Agent**：通过标准工具调用路径调用 workflow 工具。

### 下游

- **ToolRegistry**：workflow 工具注册到注册中心供 Agent 发现和调用。
- **Engine**：接收工具调用结果，驱动状态机转换。
- **Gateway**：blocked 状态通知 owner 时通过 Gateway 出站。

### 无关

- **Permission**（无调用关系）：workflow 工具为系统级工具，不经过 Agent 权限检查。
- **IM Adapter**（无调用关系）：工具调用和返回不经过消息渲染链路。
