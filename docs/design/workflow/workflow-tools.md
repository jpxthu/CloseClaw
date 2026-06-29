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

斜杠指令：用户输入 /workflow <name>，由 SlashDispatcher 在 Gateway 层拦截，转发给 Engine 加载定义并初始化 WorkflowRun。执行完毕后通过 Gateway 向用户返回确认消息。

工具调用：Agent 在对话中途调用 workflow_start({name})，当前 session 转入 workflow 模式。适用场景：用户要求执行特定 workflow，Agent 自主判断并调用工具。

### 工具注册

Workflow 工具在 ToolRegistry 初始化时注册，属于系统级工具——不受 Agent permission 配置限制。斜杠指令在 SlashDispatcher 初始化时注册 /workflow 路由。

## 数据流

### workflow_start

1. Agent 调用 workflow_start({name})
2. Engine 按优先级查找定义文件（agent workspace/workflows/ → .closeclaw/workflows/ → 内置），三级均未命中则返回错误
3. Engine 解析 YAML frontmatter
4. Engine 初始化 WorkflowRun：current_step 置 0，phase 置 executing
5. Engine 向 system prompt 注入 workflow context，待注入完毕后注入 Step 0 goal 消息（role: workflow）
6. 返回 tool result 确认

### workflow_verify

1. phase 从 executing 到 verifying 的转换由 Engine 自动管理：session idle 时 Engine 注入验收清单
2. Agent 收到验收清单后自查，完成则调用 workflow_verify()（无参数）
3. Engine 检查当前 phase：不是 verifying 则返回错误；是 verifying 则 phase 转为 jumping，注入 jump 消息
4. 返回 tool result（被抹除）

verify 只是"我做完了"的信号。验收清单来源于 Step 定义中的 verify 字段，Engine 不校验条目真伪。

### workflow_jump

1. Agent 调用 workflow_jump({answers})
2. Engine 检查当前 phase：不是 jumping 则返回错误
3. Engine 取当前步骤定义的 transitions，按顺序匹配条件。全部不匹配则执行 default
4. Engine 执行匹配到的 action（goto/reexecute/complete）
5. Engine 更新 WorkflowRun 状态
6. Engine 注入下一步 goal 或结束
7. 返回 tool result（被抹除）

答案格式由 jump 问题的 type 决定：

- boolean：YAML 原生布尔值 true / false
- enum：对应的选项字母（A/B/C/D...），非选项内部值
- string：自由文本

jump 问题来自当前步骤定义中的 jump 字段，option_labels 用于将选项内部值渲染为 ABCD 标签。

### workflow_blocked

1. Agent 调用 workflow_blocked({reason})
2. Engine 检查当前 step 的 allow_blocked：为 false 则返回错误，Agent 继续 verify 循环
3. Engine 将 phase 设为 blocked，通过 Gateway 向 owner 发送通知（含 reason）
4. 返回 tool result（被抹除）
5. Owner 回复：Engine 通过 Gateway 感知 owner 消息 → 解除阻塞 → 移除旧 goal → pending_verify 归零 → 立即注入 verify 消息
6. Agent 按正常 verify → jump 流程继续

### 斜杠指令

1. 用户输入 /workflow <name>
2. SlashDispatcher 匹配 /workflow 路由，提取 name 参数
3. 转发给 Engine
4. Engine 执行与 workflow_start 相同的初始化流程
5. 通过 Gateway 向用户返回确认消息

## 模块关系

### 上游

- **SlashDispatcher**：注册 /workflow 路由，拦截斜杠指令后转发给 Engine。
- **Agent**：通过标准工具调用路径调用 workflow 工具。

### 下游

- **ToolRegistry**：workflow 工具注册到 ToolRegistry 供 Agent 发现和调用。
- **Engine**（同模块）：接收工具调用结果，驱动状态机转换。
- **Gateway**：blocked 状态通知 owner 时通过 Gateway 出站。斜杠指令确认消息和 owner 回复感知均通过 Gateway。

### 无关

- **Permission**：workflow 工具为系统级工具，不经过 Agent 权限检查。
- **IM Adapter**：工具调用和返回不经过消息渲染链路。
