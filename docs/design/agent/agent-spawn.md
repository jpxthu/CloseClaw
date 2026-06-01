# Agent Spawn

## 概述

Spawn 是父 agent 创建子 agent 执行子任务的机制。一次 spawn = 创建一个 child session，子 session 使用目标 agent 的配置档案运行。Spawn 由 tools 模块暴露给 LLM 调用，由 agent 模块负责协调控制。

## 架构

### 入口：sessions_spawn 工具

`sessions_spawn` 注册在 tools 模块中，LLM 调用时携带以下参数：

| 参数 | 含义 | 默认 |
|------|------|------|
| `agentId` | 目标 agent 的 ID | 父 agent 配置中的 `defaultChildAgent` |
| `task` | 任务描述，注入子 session 首条消息 | 必填 |
| `mode` | `"run"`（一次性）/ `"session"`（持久线程） | `"run"` |
| `fork` | 是否 fork 父 agent 上下文 | `false` |
| `model` | 覆盖目标 agent 的默认模型 | 目标 agent 配置 |
| `workspace` | 独立工作目录 | 目标 agent 配置 |
| `label` | 子 session 简短标签 | 自动生成 |
| `lightContext` | 是否使用 minimal bootstrap | `false` |
| `promptTemplate` | 注入的 prompt 模板（`explore` / `validation`） | 无 |

`lightContext` 复用 session 模块已有的 `bootstrapMode: "minimal"` 机制。spawn 时指定 `lightContext: true`，子 session 以 minimal bootstrap 启动。

### Spawn 控制流程

```
LLM 调用 sessions_spawn(agentId, task, ...)
  ↓
Agent 协调层前置检查：
  ① depth 检查：当前深度 >= 父 agent.subagents.maxSpawnDepth → 拒绝
  ② 并发检查：活跃子 session 数 >= 父 agent.subagents.maxChildren → 拒绝
  ③ 白名单检查：agentId 不在父 agent.subagents.allowAgents 中 → 拒绝
  ④ requireAgentId 检查：配置要求显式指定但未提供 → 拒绝
  ⑤ 权限检查：spawn 链路权限继承计算 → 子 agent 无执行权限则拒绝（见 agent-permissions.md）
  ↓
全部检查通过 → 创建 child session：
  - 加载目标 agent 的配置档案（config.json + permissions.json）
  - workspace：spawn 参数指定 → 目标 agent.workspace → 父 workspace 子目录
  - bootstrap 模式：lightContext=true → minimal；否则 → 目标 agent.bootstrapMode
  - 注入 task 作为首条用户消息
  - 按 agent 配置过滤 tools/skills
  - 注入 spawn 角色标记（parent_session_id, depth, spawn_mode, fork）
  ↓
子 session 注册到父 session 的子 session 跟踪表
  ↓
子 session 启动执行
  ↓
mode="run"：子 session 完成后触发 announce
mode="session"：子 session 保持存活，等待父 agent 后续 steer
```

### Depth 追踪

Spawn 深度沿父链递归计算。根 session（用户直接对话）depth = 0。默认 `maxSpawnDepth = 1`，即 root session 可 spawn 出 depth=1 的 child，depth=1 的 child 不可再 spawn。

```
用户会话（depth=0）
  └── spawn(code-reviewer, depth=1)
        └── 拒绝 spawn 任何子 agent（maxSpawnDepth=1）
```

### Fork 模式

Fork 是 spawn 的变体：在子 session 的 system prompt 和 task prompt 之间插入父 agent 的对话历史，使子 agent 继承父 agent 的上下文认知。

```
Spawn:   [system prompt] [task prompt]
Fork:    [system prompt] [父 agent messages] [task prompt]
```

Fork 与 Spawn 共用同一 session 创建流程，区别仅在 session 组装阶段：fork 模式在注入 task 之前先注入父 agent 的 transcript messages。

| | Spawn | Fork |
|---|-------|------|
| 子 agent 上下文 | 仅 task 描述 | 父 agent 完整对话历史 + task 描述 |
| 适用场景 | 独立子任务，需完整 briefing | 父 agent 需要子 agent 理解已发生的事 |
| 父 agent 的说明成本 | 子 agent 不知前情，需解释背景 | 子 agent 已知前情，只需写指令 |

### Announce 机制

子 session 完成后，结果通过 announce 注入父 session（push-based，父 agent 不 poll）：

- 子 session 的最后一条 assistant 消息作为 announce 内容
- announce 作为内部事件推送到父 session 的消息队列
- 父 agent 在下一轮 turn 开始时处理该事件
- announce 不混入用户消息流，是一条独立的内部事件记录

### Steer 和 Kill

父 agent 对存活中的子 session（mode="session"）有两种控制操作，通过 `sessions_steer` / `sessions_kill` 工具暴露给 LLM：

- **Steer**：向子 session 发送新 task，子 session 重新执行
- **Kill**：终止子 session，释放资源，session 对话历史保留

两个操作通过子 session 跟踪表完成。

## 数据流

### Spawn Run 模式完整流程

```
父 session 调用 sessions_spawn(mode="run", agentId, task, ...)
  ↓
前置检查：depth / 并发 / 白名单 / requireAgentId / 权限
  ↓ （全部通过）
创建 child session：
  agent_id = 目标 agent
  parent_session_id = 父 session
  depth = 父 depth + 1
  bootstrap = 按 lightContext 决定
  tools = 目标 agent 配置白名单
  permissions = 继承计算结果（见 agent-permissions.md）
  first_message = task 内容
  ↓
子 session 注册到父 session 的子 session 跟踪表
  ↓
子 agent 执行 task（可能多轮 turn）
  ↓
子 session 完成：
  - 最后一条 assistant 消息提取为 announce 内容
  - announce 入队到父 session 的消息队列
  - 跟踪表中标记完成
  ↓
父 agent 下一轮 turn 开始时处理 announce
```

### Fork 模式流程

```
父 session 调用 sessions_spawn(fork=true, task, ...)
  ↓
与 Spawn 流程完全相同，唯一区别在 session 组装时：
  首条注入的不是 task
  → 先注入父 session 的 transcript messages
  → 再注入 task 消息
  ↓
子 agent 看到完整上下文 + 新任务
```

### Session 模式流程

```
父 session 调用 sessions_spawn(mode="session", agentId, task, ...)
  ↓
创建 child session（与 run 模式一致）
  ↓
子 session 启动执行 → 完成后保持存活
  ↓
父 agent 可通过 sessions_steer 发送新 task
父 agent 可通过 sessions_kill 终止子 session
  ↓
子 session 最终被 kill 或超时自动清理
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Tools | 注册 sessions_spawn / sessions_steer / sessions_kill 工具，LLM 调用后委托 agent 模块执行 |
| Agent Config | 读取父 agent 的 subagents 配置（allowAgents、maxSpawnDepth 等），读取目标 agent 的完整配置 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Session | 创建 child session、注入 task 消息、管理子 session 跟踪表和 announce 队列 |
| System Prompt | 按 lightContext/agent.bootstrapMode 决定子 session 的 bootstrap 文件集 |

### 无关

| 模块 | 说明 |
|------|------|
| Permission | spawn 链路的权限继承计算由 Permission 模块独立完成，Agent 模块不调用 Permission |
| LLM Provider | spawn 不直接调用 LLM，子 session 的 LLM 调用由 session 模块管理 |
| Processor Chain / Renderer | announce 内容的渲染由 session 的消息渲染管线完成 |
| IM Adapter | spawn 不涉及外部消息路由 |
