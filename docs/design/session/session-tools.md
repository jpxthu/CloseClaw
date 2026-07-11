# Session 对外工具

## 概述

Session 模块向 ToolRegistry 注册三个工具，供 agent 在其生命周期内管理子 session。这三个工具统一归类到 `sessions` 分组。Session 模块暴露 `register_tools(registry)` 方法，由 tools 模块在启动编排时调用

## 架构

### 工具清单

| 工具 | 说明 | 加载策略 |
|------|------|---------|
| sessions_spawn | 创建子 session 执行子任务 | 始终加载 |
| sessions_steer | 向存活中的子 session 发送新任务 | 始终加载 |
| sessions_kill | 终止子 session | 始终加载 |

三个工具注册到 ToolRegistry 的分组 `sessions` 下，注册时机为 SessionManager 初始化阶段。

### sessions_spawn

创建子 session 执行子任务。一次 spawn = 创建 child session，子 session 使用目标 agent 的配置档案运行。spawn 由 Session 模块协调控制——读取父 agent 配置中的 subagents 参数执行前置检查，并创建和管理子 session。完整的 spawn 控制流程和策略（depth 追踪、Fork 模式、Announce 回传）见 Agent 模块的 [agent-spawn.md](../agent/agent-spawn.md)。

参数：

| 参数 | 含义 | 必填 | 默认值 |
|------|------|------|--------|
| `agentId` | 目标 agent 的 ID | 否 | 当前 Agent 的 ID（spawn 自身分身） |
| `task` | 任务描述，注入子 session 首条消息 | 是 | — |
| `mode` | `"run"`（一次性）/ `"session"`（持久线程） | 否 | `"run"` |

> `mode` 描述子 session 的持久化策略，与 SessionCheckpoint 中的 `mode` 字段（对话模式：direct/plan/stream）含义不同——二者作用于不同数据结构。
| `fork` | 是否 fork 父 agent 上下文 | 否 | `false` |
| `model` | 覆盖目标 agent 的默认模型（解析优先级见下方） | 否 | 按优先级链自动解析 |
| `timeout` | 子 agent 最大执行时长（秒），覆盖目标 agent 配置和全局默认值 | 否 | 目标 agent.subagents.timeout → 全局配置 |
| `workspace` | 独立工作目录 | 否 | spawn 参数指定 → 目标 agent.workspace → 父 Agent 工作目录 |
| `label` | 子 session 简短标签 | 否 | 自动生成 |
| `lightContext` | 是否使用 minimal bootstrap | 否 | `false` |
| `promptTemplate` | 注入的 prompt 模板（`explore` / `validation`） | 否 | 无 |
| `allowedTools` | 限制子 session 可用的工具白名单 | 否 | 目标 agent 配置中的工具集 |

`lightContext` 复用 session 模块已有的 `bootstrapMode: "minimal"` 机制。spawn 时指定 `lightContext: true`，子 session 以 minimal bootstrap 启动。

`promptTemplate` 为框架提供嵌入式 prompt 模板：
- `explore`：注入"只做研究不修改文件"的行为约束
- `validation`：注入"逐条校验并报告差异"的结构化输出要求

模板不影响 agent 配置，仅在 spawn 调用时作为 prompt 前缀注入。

`model` 参数解析优先级（未指定时按以下顺序回退，直到找到非空值）：

1. spawn 调用中显式传入的 `model` 参数
2. 父 agent 配置中 `subagents.model` 字段
3. 目标 agent 配置中 `model` 字段
4. 系统默认模型

详见 [agent-config.md](../agent/agent-config.md)「模型解析优先级」。

### sessions_steer

向存活中的 `mode="session"` 子 session 发送新任务，子 session 重新执行。系统在执行前通过 Permission 引擎的「跨 Agent 通信」维度校验发起方 agent 是否有权 steer 目标 agent。

参数：

| 参数 | 含义 | 必填 |
|------|------|------|
| `sessionId` | 目标子 session 的 ID | 是 |
| `task` | 新任务描述 | 是 |

### sessions_kill

终止存活中的 `mode="session"` 子 session，释放资源，session 对话历史保留。系统在执行前通过 Permission 引擎的「跨 Agent 通信」维度校验发起方 agent 是否有权 kill 目标 agent。

参数：

| 参数 | 含义 | 必填 |
|------|------|------|
| `sessionId` | 目标子 session 的 ID | 是 |

## 数据流

### 工具调用流程

```
LLM 调用 sessions_spawn / sessions_steer / sessions_kill
  ↓
Session 模块接收调用
  ↓
sessions_spawn：
  → 读取父 agent.subagents 配置 → 前置检查（depth/并发/白名单/requireAgentId）
  → 权限检查（spawn 链路权限继承，见 ../agent/agent-permissions.md）
  → 全部通过 → 创建 child session → 注册到父 session 子 session 跟踪表
  → 子 session 启动执行 → 完成时 announce 注入父 session 消息队列

sessions_steer / sessions_kill：
  → 校验子 session 归属（parent session 一致）
  → Permission 引擎 evaluate（跨 Agent 通信维度）→ 权限检查
  → steer → 注入新 task 到子 session 对话流
  → kill → 级联停止子 session 的执行状态
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| ToolRegistry | Session 模块在初始化时调用 `register()` 注册三个工具 |
| Agent 模块 | spawn 时读取父 agent 的 subagents 配置 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| System Prompt 构建器 | 通过 ToolRegistry 获取 sessions 分组的工具索引 |
| Permission 模块 | spawn/steer/kill 时进行权限校验 |

### 无关

| 模块 | 说明 |
|------|------|
| IM Adapter | session 工具不涉及外部消息路由 |
| Processor Chain | 工具执行不参与消息处理链 |
