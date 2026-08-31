# Session 模块

## 概述

关联需求文档：[requirements/session.md](../../requirements/session.md)

Session 模块是 CloseClaw 的运行时载体，管理 session 的全生命周期。一个 session 代表一次独立的 agent 对话实例，其职责分层如下：

- **生命周期协调**：SessionManager 维护会话路由键映射并协调创建/查找/恢复，ArchiveSweeper 后台定时归档 idle 会话与清理过期数据——二者位于持久化层与执行层之上，非持久化职责。
- **持久化层**：对话上下文的创建与持久化（checkpoint + transcript）。Session 持有 system prompt 和对话历史，是 agent 与 LLM 交互的载体。
- **执行层**：运行时执行状态跟踪（LLM 交互、工具进程、子 Session）、级联停止协调、后台任务结果注入、对话压缩。一个 agent 可以有多个 session 同时运行，每个 session 独立管理自己的执行状态。

## 架构

### 子功能索引

| 文档 | 内容 |
|------|------|
| [session-lifecycle.md](session-lifecycle.md) | 持久化模型：SessionCheckpoint 数据模型（含 system prompt 追加区）、SQLite + JSONL 存储、Sweeper 自动归档与恢复 |
| [session-execution.md](session-execution.md) | 执行状态：四维状态模型（llm_active / foreground_tool_active / background_tool_active / child_active）、级联停止、统一消息队列 |
| [session-injection.md](session-injection.md) | System Prompt 注入链路（session 创建/恢复/compaction 时触发）和 memory_injection 槽位（消息级记忆摘要注入） |
| [working-directory.md](working-directory.md) | 工作目录的定义：字段、默认值、`/cd` 变更、`/pwd` 读取、system prompt 注入 |
| [compact-process.md](compact-process.md) | 会话上下文压缩：触发机制、LLM summarization、system prompt 隔离保护 |
| [llm-session-enhancements.md](llm-session-enhancements.md) | LLM 交互增强：流式输出、Reasoning Level 控制、用量统计、Thinking 内容管理 |
| [session-tools.md](session-tools.md) | 对外工具：sessions_spawn / sessions_steer / sessions_kill 的参数、行为、向 ToolRegistry 注册 |
| [spawn-tree.md](spawn-tree.md) | 父子 session 运行时关系（spawn_tree）：存储结构、查询接口、节点回收与 GC 兜底、级联 Kill、生命周期联动、重启恢复 |
| [run-health.md](run-health.md) | 运行时安全网：turn 边界健康检测（硬规则 + Hook 审查）、运行快照创建与回滚 |
| [session-recovery.md](session-recovery.md) | 重启恢复：dirty 检测、恢复通知注入、工具调用失败模拟、出站消息补投、树状恢复策略 |

Session 模块由生命周期协调、持久化层、执行层三部分组成：

```
Gateway / SessionManager  -- 生命周期协调者
  <- 日志：会话创建/查找/归档恢复

  ArchiveSweeper  -- 后台定时任务：idle 归档 + 过期清理（daemon 启动时 spawn，生命周期协调而非持久化）

  持久化层
    CheckpointManager  -- 协调 checkpoint 读写缓存 + 持久化
      -> PersistenceService -> SqliteStorage  -- SQLite 元数据 + JSONL transcript

  执行层
    ConversationSession  -- 运行时对话状态（system_prompt + messages）
      <- 日志：对话轮次追加/活跃维度变化
      <- 日志：健康检查与异常检测结果
      llm_state  -- Idle / Requesting / Receiving
      tool_handles  -- 前台 + 后台工具进程句柄
      child_handles  -- 子 Session 句柄（spawn 时注册）
      <- 日志：子 Session 创建/完成
    Message Queue  -- 优先级 now > next > later（后台结果注入，与 ConversationSession 并列）
      <- 日志：消息注入事件（后台任务结果 + 记忆注入）
```

SessionManager 维护会话路由键 -> session_id 映射表，路由到最近活跃 session。

级联停止由 ConversationSession 触发：递归停止所有子 Session -> 杀死工具进程 -> cancel LLM 请求。

- **SessionManager**：session 的生命周期协调者，位于持久化层和执行层之上。维护会话路由键 → session_id 映射表，协调各组件的 session 创建、查找、恢复。session_id 格式为 `{agent_id}_{timestamp}_{random_suffix}`，其中 timestamp 精确到秒（`YYYYMMDDhhmmss`），random_suffix 为 8 位小写 hex 随机字符串。

  **session_key 与会话路由键**：
  - session_key = {timestamp_ms}-{hash}，算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session_key-算法)
  - session_key 是消息级标识，用于日志追踪。SessionManager 内部从消息路由字段中提取稳定的**会话路由键**（platform + sender_id + peer_id + account_id）用于 registry 查找——session_key 本身不直接参与路由。peer_id 为会话上下文锚点，由插件按平台语义构造（如私聊话题粒度的「用户 + 话题」组合）
  - 会话路由键是稳定的 lookup 键。同一会话路由键下可以有多个 session（`/new` 指令创建新 session 后覆盖映射——仅映射表指针更新，旧 session 不删除，仍可通过 SQLite 查询到归档历史）。
  - session_key 用于日志追踪，会话路由键用于 registry 查找，两者是不同概念、不同用途的键。出站定向引用 reply_ref 同样随路由字段入 checkpoint 存储（见下方数据模型），仅用于出站定向投递，不参与路由

  **key registry 生命周期**：
  - 启动时：SessionManager 扫描所有 status=active 的 session，按会话路由键（platform + sender_id + peer_id + account_id）分组，取各会话路由键下 last_message_at 最大的 session_id 写入映射表。archived 和 migrating session 不加载。同时执行数据一致性校验（详见 [session-lifecycle.md](session-lifecycle.md) 数据一致性校验节）
  - 运行时：SessionManager 收到会话解析请求，从消息路由字段中提取会话路由键，查映射表获取已有 session
    - 命中 → 校验 session status：
      - active → 返回已有 session
      - migrating → 等待 Sweeper 归档完成后 status 变为 archived → 从映射表移除该条目 → 查询 SQLite，取 last_message_at 最新的 archived session 恢复并注册。等待时通知用户「会话归档中，稍后恢复…」等待方式：Sweeper 归档不主动通知，SessionManager 以短间隔轮询 status（归档仅文件移动 + 状态更新，秒级完成，轮询即 lookup status 校验的重复执行）
      - archived → 从映射表移除该条目 → 查询 SQLite，取 last_message_at 最新的 archived session 恢复并注册
    - 未命中 → 通过会话路由键查询 SQLite
      - 查到 active → 取 last_message_at 最大的直接注册到映射表（自愈：映射表因重启丢失但 SQLite 中保有 active 记录）
      - 查到 migrating → 等待归档完成后 status 变为 archived → 按 archived 路径恢复（等待方式同上：短间隔轮询 status）。等待时通知用户「会话归档中，稍后恢复…」
      - 查到 archived → 取 last_message_at 最新的一条恢复并注册
      - 查不到 → 双重确认该会话路由键下无 active session（防御性检查）后，创建新 session 并注册。若双重确认发现 active → 注册已有 session（自愈）
  - 创建新 session 后覆盖映射。`/new` 指令同理
  - 映射表为纯内存数据结构，不单独持久化——重建依赖 SessionCheckpoint 中的会话路由键字段
  - SessionManager 对每个 agent_id 串行处理请求，确保同一会话路由键的 lookup、恢复、创建操作不会并发竞态

- **生命周期协调**：
  - **ArchiveSweeper**：daemon 启动时 spawn 的后台定时任务，扫描 idle session 并归档、扫描过期 archive 并清理，属于生命周期协调职责而非持久化层。归档判定通过 SessionManager 暴露的四维活跃维度只读查询（`activity_dimensions()`）读取（任一为 true 即跳过）；默认 idle 30 分钟触发归档、归档数据不立即删除（由独立清理阈值触发删除），各配置项独立回退到系统默认值。完整机制见 [session-lifecycle.md](session-lifecycle.md) §Sweeper 机制。
- **持久化层组件**：
  - **CheckpointManager**：协调 SessionCheckpoint 的读写缓存和持久化。需要持久化时调用 PersistenceService。
  - **SqliteStorage**：生产级持久化后端。SQLite 存元数据，JSONL 文件存 transcript。

- **执行层组件**：
  - **ConversationSession**：运行时对象，持有 system prompt、消息历史、追加区内容（system prompt 第三分区 AppendSection，持久化在 checkpoint 的 system_appends 字段中）、RunningStats（token/cache 统计）、Verbosity 等级（控制出站信息块过滤，详见 [slash 模块 verbose 指令](../slash/verbose.md)）。同时持有执行状态句柄（LLM 状态、工具进程、子 Session 引用）。对话模式（normal/plan/auto）标记持久化在 SessionCheckpoint 的 `session_mode` 字段中，/mode 切换时新模式先写入内存待应用值，下一条用户消息前惰性应用并回写 checkpoint（详见 [session-lifecycle.md](session-lifecycle.md) 模式切换节；压缩保护见 [compact-process.md](compact-process.md)，模式的行为约束详见 [mode/README.md](../mode/README.md)）。三个模式概念正交、独立存储：`session_mode`（对话模式，SessionMode：normal/plan/auto，行为约束）、`reasoning_mode`（推理呈现模式，ReasoningMode：Direct/Plan/Stream/Hidden）、`mode_state`（推理步骤状态，ReasoningModeState）——`reasoning_mode` 字段带 `#[serde(alias = "mode")]` 兼容旧 checkpoint，设计文档所述的「对话模式」一律指 `session_mode`，与推理相关字段不混用。
  - **四维执行状态**：llm_active、foreground_tool_active、background_tool_active、child_active 四维独立跟踪。执行状态为纯内存数据，不进持久化——resume 后 session 回到 Idle；若崩溃前存在未完成操作，恢复扫描会注入恢复通知（详见 [session-recovery.md](session-recovery.md)），未完成操作在后续 turn 中处理。由四维派生整体执行状态 `SessionExecStatus`（Idle / Waiting / Busy）：Idle 对应可接收输入（四维中后台工具与子 Session 不计入），Busy 对应 LLM 推理或前台工具执行中，Waiting 仅在通过 `sessions_yield` 主动让出 turn 时出现。完整状态表见 [session-execution.md](session-execution.md) 四维执行状态节。

    **idle（输入就绪）**：llm_active 和 foreground_tool_active 均为 false——session 可以立即接收新输入。background_tool_active 和 child_active 不影响 idle 判定。

    **inactive（归档判定）**：四维均为 false 且距上次用户活动超过配置的 inactive 时长——触发归档。与 idle 判定的区别：background_tool_active、child_active 不影响 idle（session 可继续接收输入），但四维任一为 true 时 session 不被判定为 inactive——后台工具和子 Session 稍后还会注入消息，不能归档。llm_active 是 llm_state 的布尔投影：llm_state 在 Requesting 或 Receiving 时 llm_active 为真，Idle 时为假。llm_state 自身有三态内部状态机（Idle / Requesting / Receiving），详见 [session-execution.md](session-execution.md) 四维执行状态节。
  - **级联停止**：级联停止是通用机制——当触发级联停止时，递归停止其所有子 Session，杀死该 session 的所有工具进程，取消该 session 正在进行的 LLM 请求。具体行为受停止模式影响：Graceful 模式等待 in-flight 操作完成后停（级联子 Session 纳入超时保护），Forceful 模式立即终止。所有停止入口（/stop、父 session 停止、系统关闭）均级联终止子 Session，无「仅停单个 session」的模式（见 [session-execution.md](session-execution.md) 停止入口节）。
  - **优雅关闭的会话级接线（shutdown_handle）**：每个 ConversationSession 持有一份 `shutdown_handle`（会话与 Daemon 关闭协调器之间的运行时连接，创建/恢复时由 SessionManager 接线）。系统关闭时 Daemon 委托 SessionManager 统一关停各 session，不直接操作单个 session——在此期间 session 通过 shutdown_handle 登记/释放活跃操作计数（消息处理、后台工具执行、子 Session 协调），供关闭器判断「等待当前工作完成」还是超时升级为 forceful；同步工具执行不存入活跃计数，由关闭流程按执行状态维度等待兜底。完整协调语义见 [daemon/shutdown.md](../daemon/shutdown.md)（shutdown_handle 权威定义）、需求见 [daemon F2](../../requirements/daemon.md)。
  - **后台结果注入**：后台工具完成或子 Session 完成时，结果通过优先级消息队列（now > next > later）作为消息注入对话流，agent 在下一轮 turn 中消费。
  - **消息队列**：统一消息队列管理用户消息和非用户消息（子 Session 完成通知、后台工具结果）。优先级决定插入位置，同一优先级内非用户消息排在用户消息前面。llm_active 或 foreground_tool_active 为 true 时消息排队不解队；两者均为 false 时消息立即出队分发（无论 background_tool_active / child_active 状态）。入队时 Session 生成"⏳ 正在排队..."提示语，经 Gateway 系统通知接口发送。Immediate 斜杠指令由 Gateway 直接执行，不进入此队列；非 Immediate 斜杠指令在 Session 正忙时与其他消息同样入队排队，Session 空闲后由 Gateway 按原路由分派给 SlashDispatcher（详见 [Gateway 路由决策](../gateway/README.md)）。记忆注入走独立槽位机制（详见 [session-injection.md](session-injection.md) 消息级注入），与通用后台消息队列独立运作，两者可共存于同一批次消息中。

各子功能的关系：
- **生命周期**是持久化骨架：SessionCheckpoint 数据模型和 SqliteStorage 是其他持久化功能的底层依赖。SessionStatus（Active / Migrating / Archived）描述持久化状态，与执行状态无关。
- **执行状态**是运行时骨架：四维状态跟踪（llm_active / foreground_tool_active / background_tool_active / child_active）贯穿每次会话交互，级联停止依赖执行状态做决策，后台结果注入依赖统一消息队列调度。
- **注入**是 session 生命周期事件——决定何时构建 system prompt。触发时机（详见 session-injection.md）包括：session 创建、归档恢复、compaction 完成。注入链路不关心 system prompt 的 Section 组装细节，只负责在正确时机调用 builder 并存储结果。
- **压缩**在 session 运行时发生：对过长的对话历史做 summarization。支持手动触发（`/compact`）和自动触发（token 用量阈值），内含熔断保护和分级告警。system prompt 独立于对话消息流，不参与压缩（详见 [compact-process.md](compact-process.md#概述)），确保角色定义在任意次压缩后完整无损。
- **LLM 增强**贯穿每次 API 调用：流式推送、reasoning level 控制、cache hit 统计在每次会话交互中生效。
- **健康检测**在每个 turn 边界触发：硬规则检查（响应超时、空响应）+ 可选的 Hook 质量门禁（plan-check / loop-check / progress-check）。详见 [run-health.md](run-health.md)。

## 数据流

### Session 创建与查找

1. 用户消息到达 Gateway
2. Gateway 提取 metadata 中的会话路由字段（platform / sender_id / peer_id / account_id）传递给 SessionManager，由 SessionManager 内部提取稳定的会话路由键
3. SessionManager 查找或创建 session（per agent_id 串行）← 日志：会话创建/查找/归档恢复
   - **查映射表**
   - **命中**：校验 session status：
     - active → 返回已有 session
     - migrating → 等待 Sweeper 归档完成后 status 变为 archived → 从映射表移除该条目 → 查询 SQLite 取 last_message_at 最新的 archived session，按下方 recovery 步骤 1-7 恢复。等待时通知用户「会话归档中，稍后恢复…」（等待方式：短间隔轮询 status，见架构节 key registry）
     - archived → 从映射表移除该条目 → 查询 SQLite 取 last_message_at 最新的 archived session，按下方 recovery 步骤 1-7 恢复
   - **未命中**：通过会话路由键查询 SQLite
     - **查到 active**：直接注册到映射表（自愈：映射表因重启丢失但 SQLite 保有 active 记录）
     - **查到 migrating**：等待归档完成后 status 变为 archived → 按 archived 路径恢复（等待方式同上）。等待时通知用户「会话归档中，稍后恢复…」
     - **查到 archived**：取 last_message_at 最新的一条，按以下 recovery 步骤恢复：
       1. transcript 移回活跃区
       2. status 更新为 active
       3. 返回 SessionCheckpoint
       4. SessionManager 用 checkpoint 重建 ConversationSession（重新走注入流程，保证 prompt 内容最新）
       5. 注册到映射表
       6. 执行状态初始为 Idle
       7. Session 生成「正在恢复会话…」提示语，经 Gateway 系统通知接口发送，返回恢复后的 session
     - **查不到**：双重确认该会话路由键下无 active session（防御性检查）
       - 若有 active → 注册已有 session 到映射表（自愈，不创建新 session）
       - 若无 active → 创建新 session：
         1. 构建 system prompt（注入 bootstrap、工具列表）
         2. 初始化执行状态（Idle）
         3. 首次持久化（写入 checkpoint 和 transcript）
         4. 注册到映射表

### Session 运行时

**每次 API 调用**：

1. 注入当前活跃子 Session 摘要（若有未完成的子 Session）：插入位置在用户消息之前、且在记忆摘要之前（处于消息列表最前）。摘要含「正在执行的子 Session 数量」及每个子 Session 的概要信息——Agent 标识、任务简述、已运行时长（需求 [session F4](../../requirements/session.md)）
2. 检查 memory_injection 槽位，按模式插入记忆摘要到消息列表
3. ConversationSession 将 system_prompt + messages + reasoning level 组装为 LLM 请求
4. LLM 状态设为 Requesting
5. LLM provider 调用（经 LlmCaller 抽象发出，见下游）
   - 流式模式：Session 层接收 LLM 的 [StreamEvent](../common/shared-types.md#streamevent) 流式事件，逐事件转发统一出站路径（Verbosity → Processor Chain → 出站日志）实时推送至 IM Adapter 渲染发送，完整 ContentBlock[] 按 BlockEnd 边界组装用于消息历史
   - 非流式：返回完整响应
6. Thinking 内容作为独立 block 保留在消息历史中，展示层默认过滤（不输出给用户）
7. 完整 ContentBlock[]（含 Thinking block）写入 message history
8. 更新 token/cache 统计
9. LLM 状态回到 Idle ← 日志：对话轮次追加/活跃维度变化

**工具调用**：

1. ConversationSession 注册工具进程句柄
2. 创建工具调用 → 状态先为 Pending（瞬态），进程 fork 后转为 Running(Foreground) 或 Running(Background)
   - 前台：session 阻塞等待完成 → 完成 → 注销句柄
   - 后台：session 不阻塞，进程句柄保留 → 完成时结果注入消息队列

定期：CheckpointManager 触发持久化（保存 SessionCheckpoint 和 transcript）

### 追加区变更

1. `/system add <内容>` 或 `/system clear`
2. Gateway 将指令转发给 ConversationSession
3. ConversationSession 更新内存中的追加条目列表
4. 追加区内容持久化写入 SessionCheckpoint 的 system_appends 字段
5. 下一次 API 调用时，追加条目按第三分区（AppendSection）动态拼入 system prompt 末尾，不触发注入流程重建

### Session 停止

停止支持 Graceful 和 Forceful 两种模式。Graceful 等待 in-flight 操作完成后停止，超时后报告进度不强制 kill；Forceful 立即终止所有操作。停止入口有三种（详见 [session-execution.md](session-execution.md) 停止入口节）：

- **斜杠指令**（`/stop`）：用户在 session 内输入，强制终止当前运行（固定 Forceful）：级联终止子 Session、终止全部工具进程、cancel LLM 请求、清空消息队列。停止后 session 转 idle 待命，不结束会话
- **父 session 停止**：父 session 被停时，对所有子 session 采用相同的模式和语义级联停止
- **系统关闭**：由 Daemon 触发，委托 SessionManager 统一关闭所有活跃 session。Daemon 不直接操作单个 session。首个信号为 Graceful，重复信号为 Forceful

### Session 结束路径

会话结束路径（会话结束统一走闲置归档，`/stop` 只停运行不结束会话）：
- **闲置归档**：用户不再使用后，Sweeper 检测用户不活跃超时（last_user_activity_at 超过配置的 inactive 时长）→ 检查四维活跃维度均为 false。若有活跃子 Session（child_active 为 true），父 Session 不被判定为 inactive，跳过本次归档；若因系统错误被归档，记录告警日志并丢弃该子 Session 的完成通知 → 状态置为 migrating → transcript 移入 archived_sessions/ → 状态更新为 archived。分两步写入以覆盖崩溃窗口：先置 migrating，移动文件后再置 archived，避免移动过程中崩溃导致 session 状态不可恢复。Sweeper 不通知 SessionManager——映射表在下次 lookup 命中时通过 status 校验感知到归档，自行移除已失效条目。`/stop` 不属于会话结束：它停止运行、不结束会话，session 保留待命，会话结束统一走闲置归档
- **自动清理**：Sweeper 检测 archived 超过 purge TTL → 删除元数据 + transcript 文件。
### 重启恢复

Daemon 启动时，SessionManager 首先构建映射表（扫描所有 status=active 的 session），然后执行启动恢复扫描：对存在未完成操作（PendingOperation）的 active session 注入恢复通知；同时扫描 status=migrating 的 session——其中残留未完成操作的恢复为 active 并重新注册到映射表，无未完成操作的则完成归档（transcript 移至 archived_sessions/，状态更新为 archived）。OutboundMessage 类未完成操作由系统自动重投递，ToolCall 和 SubSessionSpawn 类未完成操作注入恢复通知，由 LLM 自主决定处理。详细设计见 [session-recovery.md](session-recovery.md)。

### 后台结果注入

后台结果注入的完整定义见 [session-execution.md](session-execution.md) 统一消息队列和子 Session 完成注入节。

```
1. 后台工具或子 Session 完成
2. 生成结构化通知消息
3. 按优先级入队消息队列（now / next / later）
4. agent 在下一轮 turn 中消费该消息 ← 日志：消息注入事件
```

### Memory Injection 槽位

详见 [session-injection.md](session-injection.md) 消息级注入。

```
1. active-searcher 写入槽位（tool role 摘要 + 位置模式）
2. 下次 API 调用组装消息时消费槽位
   - BeforeNext → 摘要插入消息列表（新消息之前）
   - AfterCurrent → 摘要插入消息列表（新消息之后）
3. 清空槽位（一次性消费）
```

与通用后台消息队列独立运作，两者可共存于同一批次消息中。← 日志：消息注入事件（记忆注入）

## 模块关系

### 上游

- **Gateway**：用户消息入口，调用 SessionManager 获取/创建 session。
- **Slash Command**：以下斜杠指令类别直接操作 Session 模块（完整指令清单见 [slash/README.md](../slash/README.md) Handler 清单）：

  | 类别 | 涉及 Session 的操作 |
  |------|-------------------|
  | 会话生命周期 | `/new` 创建新 session、`/stop` 强制终止当前运行（级联终止子 Session，session 保留待命） |
  | 工作目录 | `/cd` `/pwd` `/git` 读写 working directory |
  | 模式控制 | `/plan` `/mode` `/execute` 切换对话模式，模式标记由 Session 持久化（见下） |
  | 推理控制 | `/reasoning` 设置推理深度档位或请求关闭推理输出 |
  | 展示控制 | `/verbose` 设置信息展示等级 |
  | 上下文管理 | `/compact` 压缩对话历史、`/system` 管理 system prompt 追加区 |
- **Daemon**：启动时初始化 SqliteStorage 和 SessionConfigProvider，spawn Sweeper 后台任务；系统关闭时委托 SessionManager 统一停止所有 session（详见 [daemon/README.md](../daemon/README.md) 关闭路径）；启动时创建 SessionManager 并注入 LlmCaller（见 [daemon/README.md](../daemon/README.md) 启动路径），SessionManager 在其初始化过程中自动执行恢复扫描（详见 session-recovery.md）

### 下游

- **System Prompt Builder**：注入链路依赖此模块完成 bootstrap、工具列表的组装。
- **LLM 调用器（LlmCaller）**：ConversationSession 构建 API 请求经 LlmCaller 抽象发出，由 Daemon 启动时接线注入（桥接 LLM Registry 的统一客户端，见 [daemon/README.md](../daemon/README.md)）；stop 时通过 cancel token 取消进行中的请求。接口契约见 [common/core-traits.md](../common/core-traits.md#llmcaller)。
- **ToolRegistry**：通过 [ToolRegistrar](../common/core-traits.md#toolregistrar) trait 向 ToolRegistry 注册 sessions 分组工具（sessions_spawn / sessions_steer / sessions_kill）；注入时获取工具列表（ToolsSection）。技能清单的基础部分由 System Prompt 模块在组装时注入 SkillsSection，Session 仅在条件激活时负责 per-turn 增量消息注入（详见 [session-injection.md](session-injection.md)）。
- **PersistenceService**：CheckpointManager 通过此 trait 调用具体存储后端。
- **Permission 模块**：工具调用时，tools 模块解析操作上下文后调用 Permission 引擎完成权限检查（详见 session-tools.md）。
- **Config 模块**：sweeper 和 compaction 读取 SessionConfigProvider 获取会话配置参数（idle 超时、compact 阈值等）。
- **Agent 模块**：session 创建时读取 Agent 配置档案，分发 model/workspace/tools/skills/subagents 等字段。sessions_spawn 等工具执行时读取 subagents 配置做前置检查。
- **Processor Chain（出站）**：Session 产出的 LLM 响应 ContentBlock[] 经 Gateway 调度进入出站 Processor Chain 做 DSL 解析。出站调试日志在 Processor Chain 内记录，出站交付记录由 Gateway 持久化到 session checkpoint 的 `outbound_pending` 字段——记录每条出站消息（含 Verbosity 过滤后内容与 dsl_result，及发送标记 sent），其中未发送成功的条目（sent=false）用于崩溃/停止后重投递（需求 [session F7](../../requirements/session.md)）；与 Session 对话历史（messages[]，LLM 上下文，含完整 Thinking 块）用途不同、并行不悖，也区别于 transcript 层的 `pending_messages` 字段。详见 [Gateway 出站流程](../gateway/outbound-flow.md)。非直接调用，属数据流下游依赖。
- **IM Adapter（出站）**：Session 产出的 LLM 响应 ContentBlock[] 经 Gateway 调度和 Processor Chain 处理后，由 IM Adapter 完成出站渲染和发送（含流式推送）。Session 不直接调用 IM Adapter，数据流经 Gateway 中介传递。
- **Memory 模块**：sub-agent session 结束时通过 hook 触发 memory-miner 记忆挖掘；为每条消息 spawn active-searcher 子 Session 进行记忆搜索；写入 `memory_injection` 槽位（tool role 记忆摘要），由 Session 在消息组装时消费。

### 共享类型 / 核心 trait

- [common/core-traits](../common/core-traits.md)（实现：ToolRegistrar、SessionModeQuery；消费：PermissionChecker、ToolSession、KillHandle、SkillListingProvider、StreamingSink、LlmCaller）

### 无关

- **Agent 进程生命周期**：Agent 无独立进程；执行状态由 Session 的 session-execution 机制管理。SessionStatus（Active / Migrating / Archived）描述持久化状态，与 agent 是否在运行无关。
