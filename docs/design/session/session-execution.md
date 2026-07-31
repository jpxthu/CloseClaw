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

### Yield 机制

当 agent 通过 sessions_spawn 创建子 session 后，继续工作没有意义——它需要等待子 agent 的结果才能做下一步决策。Yield 机制让 agent 主动结束当前 turn，将执行权交还给系统，等待子 agent 完成通知。

#### sessions_yield 工具

sessions_yield 是 agent 明确表达「我 spawn 完了，等结果」的工具调用。调用后：

1. 当前 turn 立即结束，不再发起新的 LLM 请求
2. session 执行状态为 llm_active = false, child_active = true（有未完成的子 session）
3. 系统监控所有活跃子 session，全部完成后 child_active 变为 false

#### Waiting 状态行为

Waiting 有两种进入方式，行为不同。两者的共性是子 agent 完成自动触发通知；差异在于 agent turn 的结束方式。

- **被动 Waiting**：agent spawn 子 session 后未 yield，系统自动判定 child_active = true。agent 的当前 turn 自然结束后，session 回到 idle（child_active 不影响 idle 判定），后续用户消息和子 agent 完成通知均立即注入下一 turn
- **主动 Waiting（yield）**：agent 调用 sessions_yield 后当前 turn 主动结束。child_active = true。下一条消息（用户消息或子 agent 完成通知）到达时自动开启新 turn

**消息处理**：

yield 后 llm_active 和 foreground_tool_active 均为 false → session 为 idle → 用户消息和子 agent 完成通知均立即注入，不排队。child_active 不影响 idle 判定（idle 仅取决于 llm_active 和 foreground_tool_active）。

yield 不是硬阻塞——任何消息（用户消息、子 session 完成通知、超时预警通知）都解除等待、恢复父 session 的 turn。

**超时保护**：子 agent 超过 timeout_warning 时长未完成 → 向父 session 注入超时预警通知（next 优先级），子 agent 继续执行。子 agent 超过 timeout 时长未完成 → 终止该子 agent（级联终止其所有后代），注入超时通知。父 session 本身不因超时而强制恢复。

#### 禁止轮询

Yield 机制的配套约束：agent 在 spawn 子 session 后不应主动查询子 session 状态。子 session 的完成通知是 push-based——系统保证自动推送，agent 不需要也禁止调用 session 查询工具去轮询。这个约束在子 session 的系统提示词中明确注入。

#### Yield 循环

典型的 spawn→yield→resume 流程：

```
父 agent turn:
  → sessions_spawn(子A) + sessions_spawn(子B)
  → sessions_yield
  ↓
yield 后 llm_active = false, child_active = true
  ↓
子A 完成 → announce 立即注入父 session 消息队列
  → 开启新 turn，agent 看到子A 结果
子B 完成 → announce 立即注入父 session 消息队列
  → 开启新 turn，agent 看到子B 结果
```

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

- **持久化层**（无调用关系）：执行状态不进 CheckpointManager 持久化，resume 时重置。SessionStatus（Active / Archived）与执行状态独立——archived session 恢复后执行状态为 Idle
- **Permission 模块**（无调用关系）：停止操作不涉及权限检查
