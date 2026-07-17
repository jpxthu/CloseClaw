# Agent Spawn

## 概述

Spawn 是父 session 创建子 session 执行子任务的机制。一次 spawn = 创建一个 child session，子 session 使用目标 agent 的配置档案运行。sessions_spawn 由 Session 模块提供工具定义，tools 模块在启动编排时注册到 ToolRegistry 并暴露给 LLM 调用。LLM 调用该工具时，SkillTool 触发 SpawnValidator（trait，定义在 config crate）执行前置检查（depth、并发、白名单等），通过后由 SpawnController（session crate）创建和管理子 session。

## 架构

### 入口：sessions_spawn 工具

`sessions_spawn` 由 Session 模块注册到 ToolRegistry（分组 `sessions`），参数定义详见 [session-tools.md](../session/session-tools.md)。以下展开 spawn 特有的行为描述。

### Spawn 控制流程

```
LLM 调用 sessions_spawn(agentId, task, ...)
  ↓
SkillTool 触发 SpawnValidator 执行前置检查（注意：下文「父 agent」指配置来源，「父 session」指发出 spawn 调用的运行时会话）：
  ① depth 检查：父 agent 有效预算 ≤ 0 → 拒绝（预算见 Depth 追踪节）
  ② 并发检查：活跃子 session 数 >= 父 agent.subagents.maxChildren → 拒绝
  ③ requireAgentId 检查：spawn 未传 agentId 且 requireAgentId=true → 拒绝
  ④ agentId 解析：spawn 未传 agentId 时默认使用当前 Agent 的 ID（spawn 自身分身）
  ⑤ 白名单检查：agentId 不在父 agent.subagents.allowAgents 中 → 拒绝
  ↓
SpawnValidator 前置检查通过 → sessions_spawn 经 tools 模块触发 PermissionEngine.evaluate() 执行权限检查：
  ⑥ 权限检查：子 agent 经权限继承计算后无任何执行权限 → 拒绝（见 agent-permissions.md）
  ↓
全部检查通过 → SpawnController 创建 child session：
  - 加载目标 agent 的配置档案（config.json + permissions.json）
  - workspace：spawn 参数指定 → 目标 agent.workspace → 父 agent workspace 下的子目录（{agent_id}/{user_id}）
  - bootstrap 模式：lightContext=true → minimal；否则 → 目标 agent.bootstrapMode
  - 注入 task 作为首条用户消息
  - tools：`allowedTools` 参数提供时完全替换子 agent 的 config.tools，否则使用 agent 配置的工具白名单；有效预算 ≤ 0 时从白名单中移除 sessions_spawn
  - skills：按 agent 配置的 skills 白名单过滤
  - 注入 spawn 角色标记（parent_session_id, depth, spawn_mode, fork）
  - timeout：spawn 参数指定 → 目标 agent 配置 → 全局默认，控制子 session 的最大执行时长
  ↓
子 session 注册到父 session 的子 session 跟踪表
  ↓
通信配置生成（详见下方「通信配置」小节）
  ↓
子 session 启动执行
  ↓
mode="run"：子 session 完成后触发 announce
mode="session"：子 session 保持存活，等待父 agent 后续 steer
```

> 模型覆盖（按优先级链选择子 agent 模型，详见 agent-config.md「模型解析优先级」）不在上述编号步骤内执行，由 sessions_spawn 工具在子 session 创建阶段单独完成，无匹配时回退系统默认，不拒绝 spawn。

### 通信配置（CommunicationConfig）

Spawn 子 agent 时生成通信路由表，定义子 agent 的消息可达范围——可以向哪些 agent（以 agent ID 标识）发送消息、接收哪些 agent 的消息。

> **与权限的关系**：跨 agent 通信的操作权限（能否通信）由 Permission Engine 的"跨 Agent 通信"维度评估。CommunicationConfig 只负责消息路由——决定消息能否送达——不参与权限判定。当权限放行但路由未配置时，消息不可达；当路由允许但权限拒绝时，消息不发送。

通信配置包含两个方向的白名单：

- **outbound**：允许发送消息的目标 agent ID 列表。`"*"` 表示不限制
- **inbound**：允许接收消息的来源 agent ID 列表。`"*"` 表示不限制

生成规则：每次 spawn 时默认 outbound 和 inbound 均设置为仅含父 agent ID。

路由检查为双向匹配：

```
Agent A 向 Agent B 发送消息
  ↓
权限检查：PermissionEngine 评估"跨 Agent 通信"维度
  ↓ （通过）
路由检查：A 的 outbound 是否包含 B（或 "*"）
  ↓
路由检查：B 的 inbound 是否包含 A（或 "*"）
  ↓
任意一方不匹配 → 消息不可达（非权限拒绝）
两者都匹配 → 消息送达
```

此配置注入子 session 的 LLM 会话上下文，在消息路由时由 Session 模块使用。默认仅父 agent 可达，如需扩展（如兄弟 agent 之间），需显式配置额外 agent ID。

### Depth 追踪

每个 agent 的 `maxSpawnDepth` 控制其自身子孙树的最大层级数（详见 agent-config.md）。

spawn 时，有效预算在 spawn 链上逐层递传。根 agent 的初始预算取其 `maxSpawnDepth`；每 spawn 一层，预算减 1：

```
根.有效预算 = 根.maxSpawnDepth
子.有效预算 = min(子.maxSpawnDepth, 父.有效预算 - 1)
```

子 agent 的 `maxSpawnDepth` 仅用于额外加严约束——即使父 agent 允许更多层级，子 agent 可收窄自己的子树范围，但不能放大。最终取 min 值。

有效预算 ≤ 0 时禁止该 agent 继续 spawn：session 创建时不注入 sessions_spawn 工具（工具层面拦截），同时步骤 ① 的 depth 检查在调用入口做校验——即使工具被注入，spawn 请求也会在入口被拒。两层防护互不冲突。

多层示例：

```
root (maxSpawnDepth=3)
  └── child1 (maxSpawnDepth=5)
        └── 有效预算 = min(5, 3-1) = 2
        └── child2 (maxSpawnDepth=5)
              └── 有效预算 = min(5, 2-1) = 1
              └── child3 (maxSpawnDepth=1)
                    └── 有效预算 = min(1, 1-1) = 0，不可再 spawn
```

root 设定 maxSpawnDepth=3，全树最多 3 层子孙。child1 配置 5 但被 root 预算压制为 2；child3 配置 1 主动收窄到 0。

配置 `maxSpawnDepth = 0` 的 agent 完全禁止 spawn 任何子 agent。

### Fork 模式

Fork 是 spawn 的变体：在子 session 的 system prompt 和 task prompt 之间插入父 session 的对话历史，使子 agent 继承父 session 的上下文认知。

```
Spawn:   [system prompt] [task prompt]
Fork:    [system prompt] [父 session messages] [task prompt]
```

Fork 与 Spawn 共用同一 session 创建流程，区别仅在 session 组装阶段：fork 模式在注入 task 之前先注入父 agent 的 transcript messages。

| | Spawn | Fork |
|---|-------|------|
| 子 agent 上下文 | 仅 task 描述 | 父 session 完整对话历史 + task 描述 |
| 适用场景 | 独立子任务，需完整 briefing | 父 agent 需要子 agent 理解已发生的事 |
| 父 session 的说明成本 | 子 agent 不知前情，需解释背景 | 子 agent 已知前情，只需写指令 |

### Announce 机制

子 session 完成后，结果通过 announce 作为消息注入父 session 对话流（push-based，父 session 不 poll）：

- 子 session 的最后一条 assistant 消息作为 announce 内容
- announce 通过消息队列注入父 session 对话流
- 父 session 在下一轮 turn 开始时消费该消息

### 子 Agent 提示词工程

子 agent 创建后，系统向其注入两层行为引导，确保子 agent 正确理解自身角色和通信方式。

#### 系统提示词规则

子 agent 的 system prompt 中注入以下行为约束：

- **信任推送式完成通知**：spawn 的子 agent 完成后结果自动推送回父 agent，子 agent 无需主动查询
- **禁止轮询**：子 agent 不应调用 session 查询工具去检查自己 spawn 的子 agent 是否完成。如需等待子 agent 结果，使用 yield 机制结束当前 turn
- **直接执行**：子 agent 收到 task 后直接执行，不需要反问、确认或建议下一步——这些由父 agent 负责
- **不嵌套 spawn**：子 agent 默认不允许继续 spawn 子 agent（由 depth 追踪控制）。如果子 agent 的 depth 允许，系统提示词中会包含 spawn 指引

#### System Prompt 注入

子 agent 的 system prompt 末尾注入以下 spawn 上下文：
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

sessions_spawn 支持通过 `promptTemplate` 参数选择预定义的行为约束模板，在子 agent 的 task 前注入对应的行为指引。可用模板：`explore` / `plan` / `executor` / `validation`。各模板的完整 prompt 内容定义见 [mode/references/prompts.md](../mode/references/prompts.md) 第 7 节，模板索引见 agent-config.md「Prompt 模板」。模板不影响 agent 配置，仅在 spawn 时作为 prompt 前缀注入。未指定 `promptTemplate` 时子 agent 无额外行为约束。

#### 父 Agent 的 Task 编写指引

系统提示词中向父 agent 提供 task 编写的策略建议：

- 像给刚走进房间的聪明同事交代任务一样——说明你要做什么、为什么
- 不要把综合判断推给子 agent——父 agent 应完成理解和决策，子 agent 负责执行
- 需要子 agent 理解完整上下文时使用 fork 模式，独立子任务使用普通 spawn

### Steer 和 Kill

父 agent 对存活中的子 session（mode="session"）有两种控制操作，通过 `sessions_steer` / `sessions_kill` 工具暴露给 LLM。参数定义与权限校验逻辑详见 [session-tools.md](../session/session-tools.md)。

- **Steer**：向子 session 发送新 task，子 session 重新执行
- **Kill**：终止子 session 及其所有后代（级联），释放资源，session 对话历史保留。对所有 mode（run / session）有效

两个操作通过子 session 跟踪表完成。

### Spawn 树形拓扑

Spawn 树由 Session 模块内部的 spawn_tree 子组件维护，记录父子 session 的运行时关系。每棵 spawn 树的根节点是顶层 session（直接由用户或外部事件创建，非 spawn 产生），子节点由 sessions_spawn 创建时注册。

#### 存储结构

spawn_tree 维护一张内存查找表，以父 session ID 为键，子 session 列表为值。每个节点记录以下信息：

| 字段 | 含义 |
|------|------|
| session_id | 子 session 唯一标识 |
| parent_session_id | 父 session 标识（顶层 session 为空） |
| agent_id | 目标 agent ID |
| depth | 当前层级（根节点为 0） |
| mode | spawn 模式（run / session） |

spawn 成功时注册新节点，子 session 完成时标记状态，kill 时移除节点。

#### 查询接口

spawn_tree 提供三类只读查询，供 Session 模块内部使用：

- **list_children**：查询某 session 的所有**直接子节点**。Session 模块的 depth 检查、并发检查、steer/kill 操作依赖此查询
- **list_descendants**：递归查询某 session 的**所有后代节点**（子树遍历）。级联 kill 和父 session 结束时自动清理依赖此查询
- **get_parent**：查询某 session 的**父节点**。用于层级完整性校验

#### 级联 Kill

sessions_kill 终止指定 session 及其所有后代（子树）。kill 操作始终级联——不存在仅杀单个 session 而不杀其子孙的模式。

级联 kill 的执行顺序：

```
kill session A
  ↓
递归遍历 A 的子树，找出所有后代节点
  ↓
从最深层向浅层逐级终止（先杀叶子，再杀父）：
  每步执行：取消子 session → 清理子 session 数据 → 从 spawn_tree 移除节点
  ↓
最后终止 A 自身
```

从深层向浅层逐级执行，确保每层数据清理完毕后再向上走，不留下孤儿节点。已完成或已终止的 session 跳过终止步骤，但仍从 spawn_tree 移除。

#### 生命周期联动

除显式 kill 外，以下场景自动触发级联清理（子 session 对话历史均保留供查阅）：

- **父 session 正常结束**：父 session 完成时，所有仍活跃的子 session 被自动级联终止。否则子 session 失去父节点后无法被 steer
- **父 session 超时清理**：父 session 被 sweeper 超时清理时，同上级联终止所有子 session

子 session 的结束不影响父 session 或其他兄弟节点。

#### 重启恢复

spawn_tree 的运行时数据（内存查找表）随网关重启丢失。恢复依赖 session checkpoint 持久化：

**Checkpoint 字段**：`SessionCheckpoint` 包含以下字段用于记录 spawn 关系：

| 字段 | 含义 |
|------|------|
| parent_session_id | 谁 spawn 了我（顶层 session 为空） |
| depth | 当前层级（根节点为 0） |

spawn 子 session 时写入这两个字段。根 session（非 spawn 创建）没有 parent_session_id，depth 为 0。

**恢复流程**：

```
网关启动
  ↓
Session 模块逐个恢复活跃 session（现有恢复流程，从 checkpoint 重建 session）
  ↓
spawn_tree 重建：
  遍历所有已恢复的 session 的 checkpoint
  → 有 parent_session_id 且父 session 也已恢复 → 在 spawn_tree 中注册父子关系
  → 有 parent_session_id 但父 session 未恢复（已被 sweep）→ 子 session 降级为根节点，depth 重置为 0
  → 无 parent_session_id → 确认为根节点
```

降级策略：父 session 已不存在时，子 session 降级为独立根节点而非级联清理。恢复是被动行为——重启不应主动删除已持久化的 session 数据。降级后的 session 仍可正常服务用户请求。Announce 队列不持久化——若子 session 恰好在重启前完成但父 session 还来不及消费 announce，该 announce 丢失。

## 数据流

### Spawn Run 模式完整流程

```
父 session 调用 sessions_spawn(mode="run", agentId, task, ...)
  ↓
前置检查：depth / 并发 / requireAgentId / agentId 解析 / 白名单 / 权限
  ↓ （全部通过）
创建 child session：
  agent_id = 目标 agent
  parent_session_id = 父 session
  depth = 父 depth + 1
  bootstrap = 按 lightContext 决定
  tools = allowedTools 参数提供时完全替换，否则使用目标 agent 配置白名单
  permissions = 继承计算结果（见 agent-permissions.md）
  model = 按优先级链解析（显式 model > 父.subagents.model > 目标.model > 系统默认），不拒绝 spawn
  promptTemplate = 按 spawn 参数注入行为约束模板（若无则不注入）
  first_message = task 内容
  ↓
子 session 注册到父 session 的子 session 跟踪表
  ↓
子 agent 执行 task（可能多轮 turn）
  ↓
子 session 完成：
  - 最后一条 assistant 消息提取为 announce 内容
  - announce 入队到父 session 的消息队列（作为消息注入对话流）
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

### 级联 Kill 流程

```
sessions_kill(session_id)
  ↓
spawn_tree 递归遍历 session_id 的子树，收集所有后代节点
  ↓
从最深层向浅层逐级处理每个后代：
  - 取消子 session（终止 LLM 调用和工具执行）
  - 清理子 session 数据（从 conversation_sessions 和 sessions 表移除）
  - 从 spawn_tree 移除节点
  - 已完成/已终止的 session 跳过取消步骤，仅移除节点
  ↓
最后终止目标 session 自身
```

### 父 Session 结束时的级联清理

```
父 session 完成或超时
  ↓
spawn_tree 查询父 session 的所有后代
  ↓
从深层向浅层级联终止所有仍活跃的子 session
  ↓
清理子 session 数据
```

### 重启恢复流程

```
网关启动
  ↓
Session 模块逐个恢复活跃 session（从 checkpoint 重建）
  ↓
spawn_tree 重建：遍历所有已恢复 session 的 checkpoint
  → 有 parent_session_id 且父 session 也已恢复 → 注册为父子关系
  → 有 parent_session_id 但父 session 未恢复（已被 sweep）→ 子 session 降级为根节点，depth 重置为 0
  → 无 parent_session_id → 确认为根节点
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Session | 注册 sessions_spawn / sessions_steer / sessions_kill 工具到 ToolRegistry。LLM 调用时 SkillTool 触发 SpawnValidator 执行前置检查，通过后由 SpawnController 创建子 session |
| Agent Config | 读取父 agent 的 subagents 配置（allowAgents、maxSpawnDepth 等），读取目标 agent 的完整配置 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Session | 创建 child session、注入 task 消息、管理子 session 跟踪表和 announce 队列。spawn_tree 作为 Session 模块的内部子组件，维护 spawn 树的父子关系、提供树形查询、执行级联清理和重启恢复 |
| System Prompt | 按 lightContext/agent.bootstrapMode 决定子 session 的 bootstrap 文件集 |

### 无关

| 模块 | 说明 |
|------|------|
| Permission | Agent 模块（纯配置层）不直接调用 Permission；spawn 流程中的权限检查：SpawnValidator 执行前置检查后，sessions_spawn 工具经 tools 模块触发 PermissionEngine.evaluate()，由 Permission 模块完成继承计算 |
| LLM Provider | spawn 不直接调用 LLM，子 session 的 LLM 调用由 session 模块管理 |
| Processor Chain / Renderer | announce 内容的渲染由 session 的消息渲染管线完成 |
| IM Adapter | spawn 不涉及外部消息路由 |
