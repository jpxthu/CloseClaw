# Agent Spawn

## 概述

Spawn 是父 session 创建子 session 执行子任务的机制。一次 spawn = 创建一个 child session，子 session 使用目标 agent 的配置档案运行。sessions_spawn 由 Session 模块注册到 ToolRegistry 并暴露给 LLM 调用，由 Session 模块负责协调控制——读取父 agent 配置中的 subagents 参数执行前置检查，并创建和管理子 session。

## 架构

### 入口：sessions_spawn 工具

`sessions_spawn` 由 Session 模块注册到 ToolRegistry（分组 `sessions`），参数定义详见 [session-tools.md](../session/session-tools.md)。以下展开 spawn 特有的行为描述。

### Spawn 控制流程

```
LLM 调用 sessions_spawn(agentId, task, ...)
  ↓
Session 模块读取父 agent 配置中的 subagents 参数：
  ① depth 检查：父 agent.maxSpawnDepth = 0 → 拒绝（子 agent 实际能力见 Depth 追踪）
  ② 并发检查：活跃子 session 数 >= 父 agent.subagents.maxChildren → 拒绝
  ③ 白名单检查：agentId 不在父 agent.subagents.allowAgents 中 → 拒绝
  ③a agentId 回退：spawn 未传 agentId 时使用父 agent 配置的 default_child_agent。回退值也为空且 requireAgentId 为 true → 拒绝
  ④ requireAgentId 检查：配置要求显式指定但未提供（无参数且无 default_child_agent）→ 拒绝
  ④a 模型覆盖：父 agent 可通过 model 参数为子 agent 指定不同模型，不传则使用子 agent 自身配置的默认模型
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
通信配置生成：子 agent 默认仅允许与父 agent 通信（outbound 允许父 + inbound 接受父）。双向白名单机制确保子 agent 不会与无关 agent 交互
  ↓
子 session 启动执行
  ↓
mode="run"：子 session 完成后触发 announce
mode="session"：子 session 保持存活，等待父 agent 后续 steer
```

### Depth 追踪

每个 agent 的 `maxSpawnDepth` 控制其自身子孙树的最大层级数（详见 agent-config.md）。spawn 时，子 agent 的实际 spawn 能力取其自身配置与父 agent 剩余能力的较小值：子 agent 实际能力 = min(子 agent.maxSpawnDepth, 父 agent.maxSpawnDepth - 父 agent 当前 depth - 1)。能力 ≤ 0 时禁止该 agent 继续 spawn。

简化：当父 agent depth 为 0 时，公式简化为 min(子 agent.maxSpawnDepth, 父 agent.maxSpawnDepth - 1)。

```
root (depth=0, maxSpawnDepth=1)
  └── spawn(child, child.maxSpawnDepth=2)
        └── child 实际能力 = min(2, 1-0-1) = 0，child 不可再 spawn

mid (depth=1, maxSpawnDepth=3)
  └── spawn(child, child.maxSpawnDepth=2)
        └── child 实际能力 = min(2, 3-1-1) = 1，child 还可再 spawn 一层
```

配置 `maxSpawnDepth = 0` 的 agent 完全禁止 spawn 任何子 agent。

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

子 session 完成后，结果通过 announce 作为消息注入父 session 对话流（push-based，父 session 不 poll）：

- 子 session 的最后一条 assistant 消息作为 announce 内容
- announce 通过消息队列（FIFO）注入父 session 对话流
- 父 session 在下一轮 turn 开始时消费该消息

### 子 Agent 提示词工程

子 agent 创建后，系统向其注入两层行为引导，确保子 agent 正确理解自身角色和通信方式。

#### 系统提示词规则

子 agent 的 system prompt 中注入以下行为约束：

- **信任推送式完成通知**：spawn 的子 agent 完成后结果自动推送回父 agent，子 agent 无需主动查询
- **禁止轮询**：子 agent 不应调用 session 查询工具去检查自己 spawn 的子 agent 是否完成。如需等待子 agent 结果，使用 yield 机制结束当前 turn
- **直接执行**：子 agent 收到 task 后直接执行，不需要反问、确认或建议下一步——这些由父 agent 负责
- **不嵌套 spawn**：子 agent 默认不允许继续 spawn 子 agent（由 depth 追踪控制）。如果子 agent 的 depth 允许，系统提示词中会包含 spawn 指引

#### 首次消息结构

子 agent 收到的第一条消息包含上下文告知和任务内容两部分：

上下文告知（系统生成）：
- 角色声明：「你是作为子 agent 运行的」
- 层级信息：当前 depth / 最大 depth
- 通信方式：「结果自动推送回父 agent，不要主动轮询状态」

任务内容（父 agent 传入的 task 参数）：
- 直接注入为子 agent 的首条用户消息
- fork 模式下先注入父 agent 对话历史，再注入 task

#### 结构化输出

子 agent 完成后，系统引导其输出结构化摘要，便于父 agent 解析和决策：

- **任务范围**：用一句话确认自己理解的任务范围
- **执行结果**：关键发现或答案
- **涉及文件**：相关文件路径列表
- **文件变更**：修改过的文件及变更说明
- **发现的问题**：执行过程中遇到的问题或潜在风险

结构化输出是可选的引导——子 agent 仍可以自由文本回复，但结构化格式让父 agent 的处理更可靠。

#### Prompt 模板

sessions_spawn 支持通过 `promptTemplate` 参数选择预定义的行为约束模板，在子 agent 的 task 前注入对应的行为指引：

- **Explore 模式**：注入只读约束——「仅做调研和分析，不修改任何文件」。适用于代码审查、架构分析、信息收集等纯读取场景
- **Validation 模式**：注入审计约束——「逐条校验并报告差异，输出结构化 PASS/FAIL 清单」。适用于设计文档验证、代码对照等审计场景

模板不影响 agent 配置，仅在 spawn 时作为 prompt 前缀注入。未指定 `promptTemplate` 时子 agent 无额外行为约束。

#### 父 Agent 的 Task 编写指引

系统提示词中向父 agent 提供 task 编写的策略建议：

- 像给刚走进房间的聪明同事交代任务一样——说明你要做什么、为什么
- 不要把综合判断推给子 agent——父 agent 应完成理解和决策，子 agent 负责执行
- 需要子 agent 理解完整上下文时使用 fork 模式，独立子任务使用普通 spawn

### Steer 和 Kill

父 agent 对存活中的子 session（mode="session"）有两种控制操作，通过 `sessions_steer` / `sessions_kill` 工具暴露给 LLM。参数定义与权限校验逻辑详见 [session-tools.md](../session/session-tools.md)。

- **Steer**：将新 task 入队到子 session 的 pending 消息队列（FIFO），在子 session 当前 turn 完成后消费，不中断当前执行
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
  - announce 入队到父 session 的消息队列（FIFO，作为消息注入对话流）
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
| Session | 注册 sessions_spawn / sessions_steer / sessions_kill 工具到 ToolRegistry，LLM 调用后由 Session 模块执行 |
| Agent Config | 读取父 agent 的 subagents 配置（allowAgents、maxSpawnDepth 等），读取目标 agent 的完整配置 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Session | 创建 child session、注入 task 消息、管理子 session 跟踪表和 announce 队列 |
| System Prompt | 按 lightContext/agent.bootstrapMode 决定子 session 的 bootstrap 文件集 |

### 无关

| 模块 | 说明 |
|------|------|
| Permission | Agent 模块（纯配置层）不直接调用 Permission；spawn 流程中的权限检查由 Session 模块协调，通过 Permission 模块完成继承计算 |
| LLM Provider | spawn 不直接调用 LLM，子 session 的 LLM 调用由 session 模块管理 |
| Processor Chain / Renderer | announce 内容的渲染由 session 的消息渲染管线完成 |
| IM Adapter | spawn 不涉及外部消息路由 |
