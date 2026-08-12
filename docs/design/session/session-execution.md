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
| false | false | false | false | **Idle**：完全空闲，等待输入 |
| false | false | true | false | **Idle**：后台工具不阻塞消息接收，session 可立即处理新输入 |
| false | false | * | true | **Idle**：子 session 运行中，不阻塞消息接收（详见消息分派规则） |
| true | * | * | * | **Busy**：LLM 正在推理或流式输出 |
| * | true | * | * | **Busy**：前台工具执行中，session 阻塞等待结果 |

**复合状态**（由四维标志 + 时间条件组合判定）：

- **idle**：llm_active 和 foreground_tool_active 均为 false——session 可以立即接收新用户消息。background_tool_active 或 child_active 为 true 不影响 idle 判定
- **inactive**：四个活跃维度均为 false，且距上次用户活动超过配置的 inactive 时长——触发归档判定

活跃维度由各消费方按需使用：
- 用户消息分派：idle 时直接分派；非 idle 时按排队规则处理
- 归档判定：inactive 时触发归档
- Workflow 验收：任一活跃维度为 true 时不注入验收清单（详见 [workflow/README.md](../workflow/README.md)）

### 级联停止

停止一个 session 时，需清理该 session 拥有的所有活跃资源：

- **子 session**：递归调用每个 child session 的 stop 方法，形成自顶向下的级联停止
- **工具进程**：遍历所有活跃工具调用，对执行中的进程发送 kill 信号。前台和后台都停
- **LLM 请求**：若 LLM 状态为 Requesting 或 Receiving，通过取消机制终止进行中的请求

停止完成后，LLM 状态置 Idle，工具状态和子 Session 状态清空。

级联采用取消信号链：父 session 的取消信号触发时联动子 session，子 session 单独取消不影响父。

**级联超时保护**：Graceful 模式下，级联停止纳入整体超时范围——若子 session 在上级等待时限内未完成，上级不无限等待，而是向调用方报告该子 session 的标识和已执行时长。调用方自行决定继续等待或升级为 forceful。forceful 模式下无超时概念——直接 kill，不等待。

### 停止入口

停止操作统一支持两种模式：

- **Graceful（默认）**：等待 in-flight 操作完成后再停。等待中的工具调用允许自然完成，当前的 LLM turn 允许执行完毕。超时后不强制终止，而是向调用方报告进度和等待项。适用场景：Daemon 首次 SIGTERM、用户 `/stop`
- **Forceful**：立即终止所有操作。工具进程直接 kill，LLM 请求直接 cancel。调用方接受数据不一致风险。适用场景：Daemon 重复 SIGTERM 或 SIGINT、用户 `/stop --force`

三种停止入口：

- **斜杠指令**（`/stop`）：用户在 session 内输入，停当前 session。支持 `--cascade`（级联子 session）和 `--force`（强制终止）标记，可组合使用
- **父 session 停止**：父 session 被停时，对子 session 采用相同的停止模式（graceful 或 forceful）
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
- llm_active 和 foreground_tool_active 均为 false 时：消息立即出队分发（无论 background_tool_active / child_active 状态）

**优先级用途**：
- **now**：系统级紧急通知，排在队头
- **next**：子 session 完成、超时预警通知等需及时响应的内容
- **later**：普通后台工具完成通知

通知内容为结构化格式，包含任务标识、完成状态、结果或输出路径。带去重保护——同一任务只注入一次。

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
  ↓
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
触发停止（/stop 或级联或系统关闭）
  ↓
  确定模式

Graceful 模式：
  →
  1. 暂停外部输入：停止接受新消息，暂停触发新自主 turn
  ↓
  2. 若 cascade：遍历子 session，对每个递归 graceful 停止
  ↓
  3. 等待 in-flight 操作完成：
      ├─ 当前 LLM stream → 收完
      └─ 当前工具调用 → 等完成
      工具结果写入对话记录后持久化，不触发新 LLM turn——下次用户消息自然衔接
  ↓
  4. 超时处理：
      ├─ 超时前全部完成 → 正常结束
      └─ 超时 → 不杀进程，向调用方报告进度
          报告：等待项名称 + 已执行时长
          调用方决定：继续等 / 升级为 force / 放弃
  ↓
  5. 清理：清空工具状态、清空子 session 状态、LLM 状态置 Idle
     SessionManager 移除运行时引用 → 持久化对话记录和元数据

Forceful 模式：
  →
  1. 若 cascade：遍历子 session，对每个递归 force 停止
  ↓
  2. 杀工具进程：遍历所有活跃工具调用 → 全部 kill
  ↓
  3. cancel LLM 请求
  ↓
  4. 清理运行时状态：清空内存中的工具状态和子 session 状态（进程句柄、运行时跟踪），不涉及 checkpoint 持久化字段——pending_operations 保持原样，下次启动由恢复扫描处理。LLM 状态置 Idle
  ↓
  5. 持久化对话记录和元数据
```

系统关闭时，SessionManager 遍历所有活跃 session 构建父子树，叶子→根顺序、同级 session 并行停止，级联机制同步处理子 session。

### 子 session 完成注入

```
子 session 执行完成
  → 提取子 session 的最后一条 assistant 消息作为结果
  → 父 session 中子 session 状态标记为完成
  → 生成结构化通知消息
  → 入队父 session 消息队列（优先级 next）
  → 带去重保护
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

- **LLM Client**：停止时通过取消机制终止进行中的请求
- **工具进程管理**（Session 内部）：停止时遍历并终止当前 session 持有的所有工具进程（前台+后台），进程句柄由 Session 自行管理
- **Spawn 协调**：子 session 完成时通过消息队列注入结果

### 无关

- **持久化层**（无调用关系）：执行状态不进 CheckpointManager 持久化，resume 时重置。SessionStatus（Active / Migrating / Archived）与执行状态独立——archived 或 migrating session 恢复后执行状态为 Idle
- **Permission 模块**（无调用关系）：停止操作不涉及权限检查
