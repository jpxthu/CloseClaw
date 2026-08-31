# Session 执行状态

## 概述

Session 执行状态跟踪 session 运行时的所有活跃操作：LLM 交互、前台工具执行、后台工具执行、子 session 执行。四个维度独立跟踪，组合判定 session 当前是否空闲可接收新输入。执行状态为纯内存数据，不进持久化——session resume 后执行状态初始为 Idle。

## 架构

### 四维执行状态

Session 的执行状态由四个独立维度组成。每个维度为布尔标志（true = 有活跃操作，false = 空闲），对外暴露供归档判定和消息分派消费。

```
ConversationSession
  ├── llm_active：LLM 是否正在推理或流式输出
  │     Idle ──→ LLM 请求发出 → Requesting
  │     Requesting ──→ 首 token 到达 → Receiving（流式）
  │     Requesting ──→ 完整响应返回 → Idle（非流式）
  │     Receiving ──→ 流结束 → Idle
  │     Requesting 或 Receiving 时 llm_active = true
  │
  ├── foreground_tool_active：是否有前台工具调用在运行（Agent 等待其返回结果以继续推理）
  │       每个前台工具调用独立跟踪
  │       Pending → 执行中 → 完成 | 失败 | 被终止 | 超时
  │     session 阻塞，不接受新的 LLM 请求直到完成
  │     终态后立即注销——只有 Pending 和执行中的前台工具参与状态判定
  │     stop/取消时直接终态，不经过执行阶段
  │
  ├── background_tool_active：是否有后台工具调用在运行（Agent 异步发出，不阻塞当前推理流程）
  │       每个后台工具调用独立跟踪
  │       Pending → 执行中 → 完成 | 失败 | 被终止 | 超时
  │     session 不阻塞，可继续对话，进程句柄保留
  │     终态后立即注销，结果注入消息队列
  │     stop/取消时直接终态，不经过执行阶段
  │
  └── child_active：Session 是否有未完成的子 session（已 spawn 但未完成或未终止）
         执行中 → 完成 | 被终止 | 出错
       子 session 由 spawn 创建，父 session 持有其引用
       子 session 完成时结果通过消息队列注入父 session
```

### 整体状态判定

Session 的整体状态由四维组合判定：

| llm_active | foreground_tool_active | background_tool_active | child_active | 整体判定 |
|------------|------------------------|------------------------|--------------|---------|
| false | false | false | false | **就绪（Idle）**：四维均空闲，可立即接收输入 |
| false | false | true | false | **就绪（Idle）**：后台工具不阻塞消息接收，可立即处理新输入 |
| false | false | * | true | **就绪（Idle）**：子 session 运行中，不阻塞消息接收（详见消息分派规则） |
| true | * | * | * | **Busy**：LLM 正在推理或流式输出 |
| * | true | * | * | **Busy**：前台工具执行中，session 阻塞等待结果 |

> 整体状态由四维派生为 `SessionExecStatus`（Idle / Waiting / Busy 三值枚举，见 [README.md](README.md) 四维执行状态节）。其中 Busy（LLM 推理或前台工具执行中）与 Idle/Waiting 是消息分派的关键区分——Busy 时消息排队，Idle/Waiting 时立即分发；**inactive（归档判据）** 是复合状态——四维均 false 且距上次用户活动超过 inactive 时长，才由归档判定触发（见下方「复合状态」），不在整体判定表内。

**复合状态**（由四维标志 + 时间条件组合判定）：

- **idle**：llm_active 和 foreground_tool_active 均为 false——session 可以立即接收新用户消息。background_tool_active 或 child_active 为 true 不影响 idle 判定。**Waiting（yield 后）是 `SessionExecStatus` 的第三态**：Waiting 下两维均为 false，`is_session_busy` 返回 false、消息立即分发不排队（见下方 Yield 机制节）——即分类上 Waiting 是与 Idle/Busy 平级的枚举值，分派行为上与 idle 一致
- **inactive**：四个活跃维度均为 false，且距上次用户活动超过配置的 inactive 时长——触发归档判定

> idle（消息就绪）与 Workflow 验收闸门是**两个不同概念**：idle 判定不把 child_active / background_tool_active 计入；而 Workflow 验收需要 agent 不再被任何活跃维度占用（含后台任务、子 Session），故验收闸门看四维全 false。二者目的不同，勿混同。

活跃维度由各消费方按需使用：
- 用户消息分派：idle 时直接分派；非 idle 时按排队规则处理
- 归档判定：inactive 时触发归档
- Workflow 验收：**四维任一活跃为 true 时不注入验收清单**（验收待 agent 无任何占用，详见 [workflow/README.md](../workflow/README.md)、[workflow/session-integration.md](../workflow/session-integration.md)）。session 是 Workflow 状态机的执行宿主——workflow 工具（workflow_start / workflow_verify / workflow_jump / workflow_blocked）经 ToolRegistry 与运行链路交给 Workflow Engine 推进，引擎按目标 / 验收 / 跳转控制 session 的 turn 结构，详见 [workflow/execution-engine.md](../workflow/execution-engine.md)

### 级联停止

停止一个 session 时，需清理该 session 拥有的所有活跃资源：

- **子 session**：递归调用每个 child session 的 stop 方法，形成自顶向下的级联停止
- **工具进程**：遍历所有活跃工具调用，对执行中的进程发送 kill 信号。前台和后台都停
- **LLM 请求**：若 LLM 状态为 Requesting 或 Receiving，通过取消机制终止进行中的请求

停止完成后，LLM 状态置 Idle，工具状态和子 Session 状态清空。

级联采用取消信号链：父 session 的取消信号触发时联动子 session，子 session 单独取消不影响父。

**级联超时保护**：Graceful 模式下，级联停止纳入整体超时范围——若子 session 在上级等待时限内未完成，上级不无限等待，而是向调用方报告该子 session 的标识和已执行时长。调用方自行决定继续等待或升级为 Forceful。Forceful 模式下无超时概念——直接 kill，不等待。

### 停止入口

停止操作按执行方式分两种模式，由触发入口携带：

- **Graceful**：等待 in-flight 操作完成后再停。等待中的工具调用允许自然完成，当前的 LLM turn 允许执行完毕。超时后不强制终止，而是向调用方报告进度和等待项。适用场景：Daemon 首次 SIGTERM
- **Forceful**：立即终止所有操作。工具进程直接 kill，LLM 请求直接 cancel。调用方接受数据不一致风险。适用场景：Daemon 重复 SIGTERM 或 SIGINT、用户 `/stop`

三种停止入口，均级联终止子 session：

- **斜杠指令**（`/stop`）：用户在 session 内输入，强制终止当前运行。/stop 为 Immediate 指令（指令分发属性，非停止模式）——绕过统一消息队列立即分发，LLM 运行中立即生效。无参数、无标记，固定 Forceful 语义：级联终止所有子 session、终止全部工具进程（前台+后台）、cancel 进行中的 LLM 请求、清空统一消息队列中的排队消息。停止后四维执行状态归零、session 转为 idle 待命——`/stop` 停止运行、不结束会话，对话历史完整保留，用户可继续对话（会话结束统一走归档，见 [session-lifecycle.md](session-lifecycle.md)）
- **父 session 停止**：父 session 被停时，对所有子 session 采用相同的模式和语义级联停止
- **系统关闭**：由 Daemon 触发，调用 SessionManager 统一关闭所有活跃 session。SessionManager 内部负责 session 树遍历和停止顺序，Daemon 只传模式参数和超时。所有 session 关闭完毕后，未在超时内完成的 session 留有未清除的 pending_operations——下次启动时由恢复扫描检测为 dirty 并注入恢复通知

### 统一消息队列

后台工具结果、子 session 完成通知等非用户消息与用户消息共用一条消息队列，per ConversationSession。

**优先级决定插入位置**（now > next > later），同一优先级内非用户消息排在用户消息前面：

```
统一消息队列（单队列）
  now 非用户 → now 用户 → next 非用户 → next 用户 → later 非用户 → later 用户
```

**分发规则**：
- llm_active 或 foreground_tool_active 为 true 时：消息保留在队列中，不解队
- llm_active 和 foreground_tool_active 均为 false 时：消息立即出队分发，注入对话流（无论 background_tool_active / child_active 状态）  ← 日志：消息注入事件（后台任务结果注入）

**优先级用途**：
- **now**：系统级紧急通知，排在队头
- **next**：子 session 完成、超时预警通知等需及时响应的内容
- **later**：普通后台工具完成通知

通知内容为结构化格式，包含任务标识、完成状态、结果或输出路径。**去重保护**：同一任务/同一子 Session 的结果只注入一次，防止重复消费。会话级去重由 `injected_task_ids` 集合实现，在结果注入（入队）时按任务标识去重——已在集合中则跳过；记忆摘要会话层去重见 [session-injection.md](session-injection.md) §消息级注入（active-searcher 事件级集合 + session 层去重共同构成）。

> 去重**只作用于完成任务的结果通知**（子 Session 完成、后台任务完成）。子 Session 超时预警的**循环注入不受去重影响**——同一未终止子 Session 会按间隔反复收到预警，那是不同的注入意图，不走 `injected_task_ids` 集合（见下节「子 Session 超时通知」）。

### 子 Session 超时通知

子 Session 的超时分为两阶段：**超时预警**（timeout_warning）和**硬超时**（timeout）。预警阶段通过循环通知提醒父 Agent；硬超时阶段系统自动终止子 Session。

#### 超时预警（timeout_warning）

子 Session 运行时长达到 timeout_warning 秒后，系统不自动终止，而是通过循环通知机制提醒父 Agent。父 Agent 自行决定是否终止子 Session、继续等待、或向用户汇报。

**通知时机**：
- 子 Session 运行时长首次达到 timeout_warning 秒 → 向父 Session 消息队列注入超时预警通知（next 优先级）
- 若父 Agent 未终止该子 Session，且子 Session 仍在运行：等待 timeout_warning × intervalRatio 秒后再次注入通知
- 之后每次等待相同间隔，循环往复，直到父 Agent 主动终止子 Session 或子 Session 自然完成

**通知内容**：
- 设定的预期执行时长（timeout_warning）
- 实际运行时长（从子 Session 创建到通知生成的那一刻）
- 硬超时时间（timeout）及剩余时间
- context window 使用情况（已用 token / 总容量）
- 当前 token 用量（prompt tokens + completion tokens）

**间隔比例**：intervalRatio 默认为 0.5（即 50%），由父 Agent 配置中的 `subagents.timeoutNotifyIntervalRatio` 字段控制。

**超时来源**：timeout_warning 按以下优先级确定：sessions_spawn 显式参数 → 目标 Agent 配置（`subagents.timeout_warning`）→ 全局默认值。

```
子 Session 创建（timeout_warning=120s, intervalRatio=0.5）
  │
  ├─ t=120s ──→ 首次预警通知
  ├─ t=180s ──→ 第二次通知（120 + 60）
  ├─ t=240s ──→ 第三次通知（180 + 60）
  └─ ...     循环往复，直到父 Agent 终止子 Session 或子 Session 完成
```

#### 硬超时（timeout）

子 Session 运行时长达到 timeout 秒后，系统自动终止子 Session（级联终止其所有后代），并向父 Session 消息队列注入超时终止通知（now 优先级）。

- timeout 是系统级兜底保护，默认值 48 小时——一般情况下不会触发，仅防止子 Session 无限期运行
- 终止通知内容包含：设定硬超时时间、实际运行时长、终止原因
- timeout 按以下优先级确定：sessions_spawn 显式参数 → 目标 Agent 配置（`subagents.timeout`）→ 全局默认值（48 小时）

#### 禁止轮询

Agent 在 spawn 子 Session 后不应主动查询子 Session 状态。子 Session 的完成通知和超时通知是 push-based——系统保证自动推送，Agent 不需要也禁止调用 session 查询工具去轮询。此约束在子 Session 的 system prompt 中明确注入。

### Yield 机制

父 Session spawn 子 Session 后，可通过 `sessions_yield` 工具主动放弃当前 turn 并进入 **Waiting（等待）** 状态，等待子 Session 结果，而不是轮询或阻塞在当前 turn。这是 F4「子 Session 委托与协调」禁止轮询、push-based 交付的具体落地。

**进入 Waiting**：调用 `sessions_yield`（无参数，详见 [session-tools.md](session-tools.md) §sessions_yield）。调用后 session 进入 Waiting 状态并结束当前 turn。

**Waiting 期间的活跃判定**：Waiting 状态下，`llm_active` 与 `foreground_tool_active` 均为 false——因此 `is_session_busy` 返回 false，入站用户消息和子 Session 完成通知走正常路径**立即注入对话历史并分发 LLM，不排队**。Session 在任一消息到达时自动恢复（结束 Waiting），无需额外指定恢复时机。

**Yield 提醒（spawn_guard_reminder）**：若父 Session 当前有活跃子 Session 且未处于 Waiting 状态，系统会在 turn 边界注入一条提醒，建议调用 `sessions_yield` 等待子结果。处于 Waiting 状态时不再重复提醒。

**Yield 超时**：Waiting 不是无限期挂起。进入 Waiting 时启动 yield 超时（默认 600 秒，可按 sessions_spawn 的 timeout / timeout_warning 及通知间隔比例折算整体时限），满超时后 Session 自动恢复，避免因子 Session 长时间不产出而永久停滞。子 Session 自身的超时/僵死兜底见 [run-health.md](run-health.md) §Spawn 静默失败防护。

1. 父 Session 调用 `sessions_yield` → 进入 Waiting，结束当前 turn
2. Waiting 期间：`llm_active` / `foreground_tool_active` 均为 false → `is_session_busy` 返回 false
   - 子 Session 完成通知 / 用户消息 → 立即注入历史并分发，不排队；任一消息到达 → 自动恢复（结束 Waiting）
   - 无消息且 yield 超时（默认 600s）→ 自动恢复

> 全程注记：父 Session spawn 后未 yield 时，turn 边界注入 yield 提醒（spawn_guard_reminder）；进入 Waiting 后不再重复提醒。

## 数据流

### 执行状态转换

```
新 session 创建
  → 所有执行状态初始为空闲
  ↓
收到用户消息
  → 组装 LLM 请求 → LLM 状态变为 Requesting
    → 流式：首 token 到 → Receiving → 流结束 → Idle
    → 非流式：完整响应后 → Idle
  ↓  ← 日志：活跃维度变化（任一维度开启或关闭）
LLM 返回 tool call
  → 创建工具调用 → 状态为 Pending
    → 前台执行 → 阻塞等待完成 → 完成后注销
    → 后台执行 → 不阻塞 → 进程退出 → 注入结果到消息队列
  ↓
LLM 返回 spawn 请求
  → 创建子 session → 状态为执行中
    → 子 session 执行中，父 session 不阻塞（等待通知）
    → 子 session 完成 → 状态改为完成 → 结果注入父 session 消息队列
  ↓
Session resume（从 archived 恢复）
  → 所有执行状态重置为空闲
  → 对话历史从 transcript 重建
```

### 停止流程

```
触发停止（/stop、级联停止或系统关闭）
  ↓
  确定模式：/stop 固定 Forceful；系统关闭由 Daemon 按信号判定；
  级联停止继承父 session 的模式

Forceful 模式（/stop、Daemon 重复信号）：
  →
  1. 遍历子 session，对每个递归 Forceful 停止
  ↓
  2. 杀工具进程：遍历所有活跃工具调用 → 全部 kill
  ↓
  3. cancel LLM 请求
  ↓
  4. 清理运行时状态：清空内存中的工具状态和子 session 状态（进程句柄、运行时跟踪），不涉及 checkpoint 持久化字段——pending_operations 保持原样，下次启动由恢复扫描处理。LLM 状态置 Idle
  ↓
  5. 清空统一消息队列（仅 /stop 入口）：排队消息全部丢弃
  ↓
  6. 持久化对话记录和元数据
     /stop 入口：session 保留，转为 idle 待命，SessionManager 不移除引用
     系统关闭入口：SessionManager 移除运行时引用

Graceful 模式（Daemon 首次 SIGTERM）：
  →
  1. 暂停外部输入：停止接受新消息，暂停触发新自主 turn
  ↓
  2. 遍历子 session，对每个递归 graceful 停止
  ↓
  3. 等待 in-flight 操作完成：
      - 当前 LLM stream → 收完
      - 当前工具调用 → 等完成
      两类 in-flight 操作全部完成后，工具结果写入对话记录并持久化，不触发新 LLM turn——下次用户消息自然衔接
  ↓
  4. 超时处理：
      ├─ 超时前全部完成 → 正常结束
      └─ 超时 → 不杀进程，向调用方报告进度
          报告：等待项名称 + 已执行时长
          调用方决定：继续等 / 升级为 Forceful / 放弃
  ↓
  5. 清理：清空工具状态、清空子 session 状态、LLM 状态置 Idle
     SessionManager 移除运行时引用 → 持久化对话记录和元数据
```

系统关闭时，SessionManager 遍历所有活跃 session 构建父子树，叶子→根顺序、同级 session 并行停止，级联机制同步处理子 session。

### 子 session 完成注入

```
子 session 执行完成  ← 日志：子 Session 创建与完成
  → 提取子 session 的最后一条 assistant 消息作为结果
  → 生成结构化通知消息
  → 入队父 session 消息队列（优先级 next）
  → 带去重保护
  → spawn_tree 回收该子节点（见 spawn-tree.md 节点回收）
  ↓
父 session 下一轮 turn
  → 消息队列出队 → 子 session 完成通知作为消息注入对话流
  → agent 看到通知内容，可据此继续决策
```

## 模块关系

### 上游

- **Gateway**：用户 `/stop` 指令触发 session 停止
- **Daemon**：系统关闭时委托 SessionManager 统一关闭所有活跃 session（详见 daemon/README 关闭路径），Daemon 不直接操作单个 session
- **父 session**：父 session 停止时级联触发子 session 停止
- **Mode 模块**：Mode 通过 session 存储模式标记，切换模式时控制 session 的工具可用性、权限边界和 system prompt 注入

### 下游

- **LLM 调用器（LlmCaller）**：停止时通过取消机制终止进行中的请求
- **工具进程管理**（Session 内部）：停止时遍历并终止当前 session 持有的所有工具进程（前台+后台），进程句柄由 Session 自行管理
- **Spawn 协调**：子 session 完成时通过消息队列注入结果

### 无关

- **持久化层**（无调用关系）：执行状态不进 CheckpointManager 持久化，resume 时重置。SessionStatus（Active / Migrating / Archived）与执行状态独立——archived 或 migrating session 恢复后执行状态为 Idle
- **Permission 模块**（无调用关系）：停止操作不涉及权限检查
