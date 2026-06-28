# Workflow Tools

## 概述

Workflow Tools 是 Agent 与 Engine 之间的结构化通信接口。Agent 通过标准工具调用向 Engine 报告步骤状态，Engine 根据工具调用结果驱动状态转换。

## 架构

### 工具清单

| 工具 | 发起方 | 用途 | 参数 |
|------|--------|------|------|
| `workflow_start` | Agent | 进入 workflow 模式 | `name`：workflow 名称 |
| `workflow_verify` | Agent | 声明步骤完成 | 无 |
| `workflow_jump` | Agent | 回答跳转问题 | 按定义的结构化答案 |
| `workflow_blocked` | Agent | 主动请求阻塞 | `reason`：阻塞原因 |

所有工具向 ToolRegistry 注册，Agent 通过标准工具调用路径与 Engine 通信。

### 触发机制

Workflow 模式通过两种方式触发：

**斜杠指令**：用户在 session 中输入 `/workflow <name>`，由 SlashDispatcher 在 Gateway 层拦截，转发给 Engine 加载定义并初始化 WorkflowRun。

**工具调用**：Agent 在对话中途调用 `workflow_start({name})`，当前 session 转入 workflow 模式。适用场景：用户说"严格执行你 workspace 里的 workflow xxx"，Agent 自主判断并调用工具。

### 工具注册

Workflow 工具在 ToolRegistry 初始化时注册，属于系统级工具——不受 Agent permission 配置限制，所有 Agent 均可用。

斜杠指令在 SlashDispatcher 初始化时注册 `/workflow` 路由，与其他斜杠指令同等待遇。

## 数据流

### workflow_start

```
Agent: workflow_start({ name: "design-doc-modify" })
  ↓
Engine: 按优先级查找定义文件（命中即用，不继续）：
    1. workspace/workflows/design-doc-modify/SKILL.md
    2. .closeclaw/workflows/design-doc-modify/SKILL.md
    3. 内置
  ↓
Engine: 解析 YAML frontmatter
Engine: 初始化 WorkflowRun（current_step=0, phase=executing）
Engine: 向 system prompt 注入 workflow context
Engine: 注入 Step 0 goal 消息
  ↓
返回 tool result："Workflow 'design-doc-modify' 已启动，当前 Step 0"
```

### workflow_verify

```
Agent: workflow_verify()
  ↓
Engine: 检查当前 phase 是否为 verifying
  ├─ 否 → 返回错误："当前不在验证阶段"
  └─ 是 → phase = jumping
     Engine 注入 jump 消息
  ↓
返回 tool result（被抹除，Agent 在 jump 消息中看到下一步）
```

无参数——只是一个"我做完了"的信号。验收清单项目的真实性 Engine 不验证，由 Agent 诚实自查。

### workflow_jump

```
Agent: workflow_jump({ change_type: "B" })
  ↓
Engine: 检查当前 phase 是否为 jumping
  ├─ 否 → 返回错误
  └─ 是 → 取当前步骤定义
     Engine 按 transitions 顺序匹配
     Engine 执行匹配到的 action
     Engine 更新 WorkflowRun state
     Engine 注入下一步 goal 或完成
  ↓
返回 tool result（被抹除）
```

答案格式由 jump 问题定义决定：
- boolean 类型：`true` / `false`
- enum 类型：`"A"` / `"B"` / `"C"` / `"D"`（对应选项序号，非 option 内部值）
- string 类型：自由文本

### workflow_blocked

```
Agent: workflow_blocked({ reason: "设计文档与代码存在冲突，需 owner 裁决" })
  ↓
Engine: phase = blocked
Engine: 通过 Gateway 向 owner 发送通知（含 reason）
  ↓
返回 tool result："Workflow 已阻塞，等待 owner 决策"
```

阻塞解除由 owner 通过对话消息输入后，Engine 评估回复 → 标记为 verifying → 注入 verify 消息继续流程。

### 斜杠指令

```
用户: /workflow design-doc-modify
  ↓
SlashDispatcher: 匹配 /workflow 路由 → 提取 name 参数
  ↓
Engine: 同 workflow_start 流程
  ↓
向用户返回："Workflow 'design-doc-modify' 已启动"
```

## 模块关系

### 上游

- **SlashDispatcher**：注册 `/workflow` 路由，拦截斜杠指令后转发给 Engine。
- **Agent**：通过标准工具调用路径调用 workflow 工具。

### 下游

- **ToolRegistry**：workflow 工具注册到此中心，供 Agent 发现和调用。
- **Engine**：接收工具调用结果，驱动状态机转换。
- **Gateway**：blocked 状态通知 owner 时，通过 Gateway 出站。

### 无关

- **Permission**（无调用关系）：workflow 工具为系统级工具，不经过 Agent 权限检查。
- **IM Adapter**（无调用关系）：工具调用和返回不经过消息渲染链路。
