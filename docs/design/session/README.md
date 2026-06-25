# Session 模块

## 概述

Session 模块是 CloseClaw 的运行时载体，管理 session 的全生命周期。一个 session 代表一次独立的 agent 对话实例，其职责分两层：

- **持久化层**：对话上下文的创建、消息处理、压缩、归档与清理。Session 持有 system prompt 和对话历史，是 agent 与 LLM 交互的载体。
- **执行层**：运行时执行状态跟踪（LLM 交互、工具进程、子 session）、级联停止协调、后台任务结果注入。一个 agent 可以有多个 session 同时运行，每个 session 独立管理自己的执行状态。

## 架构

### 子功能索引

| 文档 | 内容 |
|------|------|
| [session-lifecycle.md](session-lifecycle.md) | 持久化模型：SessionCheckpoint 数据模型（含 system prompt 追加区）、SQLite + JSONL 存储、Sweeper 自动归档与恢复 |
| [session-execution.md](session-execution.md) | 执行状态：三维状态模型（LLM / Tool / 子 Session）、级联停止、后台结果注入 |
| [session-injection.md](session-injection.md) | System Prompt 注入链路（session 创建/恢复/compaction 时触发）和 memory_injection 槽位（消息级记忆摘要注入） |
| [working-directory.md](working-directory.md) | 工作目录的定义：字段、默认值、`/cd` 变更、`/pwd` 读取、system prompt 注入 |
| [compact-process.md](compact-process.md) | 会话上下文压缩：触发机制、LLM summarization、system prompt 隔离保护 |
| [llm-session-enhancements.md](llm-session-enhancements.md) | LLM 交互增强：流式输出、Reasoning Level 控制、Cache Hit 统计、Thinking 内容管理 |
| [session-tools.md](session-tools.md) | 对外工具：sessions_spawn / sessions_steer / sessions_kill 的参数、行为、向 ToolRegistry 注册 |
| [run-health.md](run-health.md) | 运行时安全网：turn 边界健康检测（硬规则 + Hook 审查）、运行快照创建与回滚 |
| [session-recovery.md](session-recovery.md) | 重启恢复：dirty 检测、恢复通知注入、工具调用失败模拟、出站消息补投、树状恢复策略 |

Session 模块由持久化层和执行层两部分组成：

```
Gateway / SessionManager  ← session 生命周期协调者
    │
    ├── 会话路由键 → session_id 映射表  ← 路由到最新 session
    │
    ├── 持久化层
    │   ├── CheckpointManager  ← 持久化协调（内存缓存 + PersistenceService）
    │   │       │
    │   │       └── SqliteStorage  ← SQLite 元数据 + JSONL transcript 文件
    │   │
    │   └── ArchiveSweeper  ← 后台定时任务（idle 归档 + 过期清理）
    │
    └── 执行层
        ├── ConversationSession  ← 运行时对话状态（system_prompt + messages）
        │       ├── llm_state     ← 当前 LLM 交互状态（Idle / Requesting / Receiving）
        │       ├── tool_handles  ← 活跃工具进程句柄（前台 + 后台）
        │       └── child_handles ← 子 session 句柄（spawn 时注册）
        │
        │  级联停止
        │       ├── 停子 session（递归）
        │       ├── 杀工具进程
        │       └── cancel LLM 请求
        │
        └── 消息队列 ← 后台结果注入（优先级 now / next / later）
```

- **持久化层组件**：
  - **SessionManager**：session 的创建、查找、恢复入口。维护会话路由键 → session_id 映射表。查询时：命中返回已有 session，未命中创建新 session 并写入映射。`/new` 指令创建新 session 后覆盖映射。协调各组件完成 session 初始化。session_id 格式为 `{agent_id}_{timestamp}_{random_suffix}`，其中 timestamp 精确到秒（`YYYYMMDDhhmmss`），random_suffix 为 8 位小写 hex 随机字符串。

  **session_key 与会话路由键**：
  - session_key = {timestamp}-{hash}，算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session-key-算法)
  - session_key 是消息级标识，用于日志追踪。SessionManager 内部从消息路由字段中提取稳定的**会话路由键**（platform + sender_id + peer_id + account_id）用于 registry 查找——session_key 本身不直接参与路由
  - 会话路由键是稳定的 lookup 键。同一会话路由键下可以有多个 session（`/new` 指令创建新 session 后覆盖映射）

  **key registry 生命周期**：
  - 启动时：SessionManager 扫描所有 status=active 的 session，按会话路由键（platform + sender_id + peer_id + account_id）分组，取各会话路由键下最新 session_id 写入映射表。archived session 不加载。同时执行数据一致性校验（详见 [session-lifecycle.md](session-lifecycle.md) 数据一致性校验节）
  - 运行时：SessionManager 收到 resolve(session_key) 调用后，从消息路由字段中提取会话路由键，查映射表获取已有 session
    - 命中 → 校验 session status 仍为 active。若 status 已变为 archived（如被 Sweeper 归档），从映射表移除该条目 → 走未命中回退路径
    - 未命中 → 通过会话路由字段查询 SQLite → 查到 archived 则取 last_message_at 最新的一条恢复并注册 → 查不到则创建新 session 并注册
    - 创建新 session 前，做一次 SQLite 双重确认：该会话路由键下是否已有 active session（防御性检查，正常不应发生）。若有 → 直接注册已有 session（自愈），不再创建新的
  - 创建新 session 后覆盖映射。`/new` 指令同理
  - 映射表为纯内存数据结构，不单独持久化——重建依赖 SessionCheckpoint 中的会话路由键字段
  - SessionManager 对每个 agent_id 串行处理请求，确保同一会话路由键的 lookup、恢复、创建操作不会并发竞态
  - **CheckpointManager**：协调 SessionCheckpoint 的读写缓存和持久化。需要持久化时调用 PersistenceService。
  - **SqliteStorage**：生产级持久化后端。SQLite 存元数据，JSONL 文件存 transcript。
  - **ArchiveSweeper**：定时后台任务，扫描 idle session 并归档，扫描过期 archive 并清理。

- **执行层组件**：
  - **ConversationSession**：运行时对象，持有 system prompt、消息历史、system prompt 追加区（system_appends）、RunningStats（token/cache 统计）、Verbosity 等级（控制出站信息块过滤，详见 [slash 模块 verbose 指令](../slash/verbose.md)）。同时持有执行状态句柄（LLM 状态、工具进程、子 session 引用）。
  - **三维执行状态**：LLM 状态、Tool 状态（per-invocation）、子 Session 状态三者独立跟踪，组合判定 session 当前是否空闲。执行状态为纯内存数据，不进持久化——resume 后 session 回到 Idle。
  - **级联停止**：停止一个 session 时，递归停止其所有子 session，杀死该 session 的所有工具进程，取消该 session 正在进行的 LLM 请求。
  - **后台结果注入**：后台工具完成或子 session 完成时，结果通过优先级消息队列（now > next > later）作为消息注入对话流，agent 在下一轮 turn 中消费。
  - **Session 忙碌队列**：Session 正忙（LLM 运行中或工具执行中）时，Gateway 路由来的新用户消息进入 FIFO 待处理队列。Session 空闲后自动取出队首消息，按原路由分派：普通消息送入 LLM，斜杠指令分派给 SlashDispatcher。入队时 Gateway 回复"⏳ 正在排队..."通知用户。Immediate 斜杠指令（/stop、/status、/help 等）绕过此队列。

各子功能的关系：
- **生命周期**是持久化骨架：SessionCheckpoint 数据模型和 SqliteStorage 是其他持久化功能的底层依赖。SessionStatus（Active / Archived）描述持久化状态，与执行状态无关。
- **执行状态**是运行时骨架：LLM、Tool、子 Session 三维状态跟踪贯穿每次会话交互，级联停止依赖执行状态做决策，后台结果注入依赖消息队列调度。
- **注入**是 session 生命周期事件——决定何时构建 system prompt。触发时机（详见 session-injection.md）包括：session 创建、archive 恢复、compaction 完成。注入链路不关心 system prompt 的 Section 组装细节，只负责在正确时机调用 builder 并存储结果。
- **压缩**在 session 运行时发生：对过长的对话历史做 summarization。支持手动触发（`/compact`）和自动触发（token 用量阈值），内含熔断保护和分级告警。system prompt 独立于对话消息流，不参与压缩，确保角色定义在任意次压缩后完整无损。
- **LLM 增强**贯穿每次 API 调用：流式推送、reasoning level 控制、cache hit 统计在每次会话交互中生效。

## 数据流

### Session 创建与查找

```
用户消息到达 Gateway
  → Gateway 提取 metadata 中的会话路由信息
  → SessionManager 查找或创建 session（per agent_id 串行）
    → 查映射表
    → 命中 → 校验 session status 仍为 active
      → 是 active → 返回已有 session
      → 非 active → 从映射表移除该条目 → 走未命中路径
    → 未命中 → 通过会话路由字段查询 SQLite
      → 查到一条或多条 archived session → 取 last_message_at 最新的一条
        → transcript 移回活跃区
        → status 更新为 active
        → 返回 SessionCheckpoint
        → SessionManager 用 checkpoint 重建 ConversationSession（重新走注入流程，保证 prompt 内容最新）
        → 注册到映射表
        → 执行状态初始为 Idle
        → Gateway 通知用户 → 返回恢复后的 session
      → 查不到 archived → 双重确认该会话路由键下无 active session
        → 若有 → 注册已有 session 到映射表（自愈，不创建新 session）
        → 若无 → 创建新 session → 注册到映射表
          → 构建 system prompt（注入 bootstrap、工具列表、skill 列表）
          → 初始化执行状态（Idle）
          → 首次持久化（写入 checkpoint 和 transcript）
```

### Session 运行时

```
每次 API 调用前：
  ConversationSession 组装请求（system_prompt + messages + reasoning level）
     → 检查 memory_injection 槽位，按模式插入记忆摘要到消息列表
     → LLM 状态设为 Requesting
     → LLM provider 调用
       → 流式模式下：通过 StreamingSink 推送 Text chunks，状态 Receiving
       → 非流式：返回完整响应
     → 剥离 thinking 标签，提取纯文本
     → 新增消息写入 message history
     → 更新 token/cache 统计
     → LLM 状态回到 Idle（若无其他 pending 操作）

工具调用：
  ConversationSession 注册工具进程句柄
     → Tool 状态设为 Running(Foreground) 或 Running(Background)（创建后先进入 Pending 瞬态，进程 fork 后转为 Running）
     → 前台：session 阻塞等待完成 → 完成 → 注销句柄
     → 后台：session 不阻塞，进程句柄保留 → 完成时结果注入消息队列

定期：CheckpointManager 触发持久化（保存 SessionCheckpoint 和 transcript）
```

### 追加区变更

```
/system add <内容> 或 /system clear
  ↓
Gateway 将指令转发给 ConversationSession
  ↓
ConversationSession 更新内存中的追加条目列表
  ↓
追加区变更触发持久化，system_appends 写入 SessionCheckpoint
  ↓
下一次 API 调用时，追加条目拼入 system prompt 的追加区
```

### Session 停止

停止支持 Graceful 和 Forceful 两种模式。Graceful 等待 in-flight 操作完成后停止，超时后报告进度不强制 kill；Forceful 立即终止所有操作。详细设计见 [session-execution.md](session-execution.md) 停止入口。

停止入口有三种：

- **斜杠指令**（`/stop`）：用户在 session 内输入，停当前 session。支持 `--cascade`（级联子 session）和 `--force`（强制终止）标记
- **父 session 停止**：父 session 被停时，对子 session 采用相同的停止模式
- **系统关闭**：由 Daemon 触发，委托 SessionManager 统一关闭所有活跃 session。Daemon 不直接操作单个 session。首次信号为 graceful，重复信号为 forceful

### Session 结束路径

两种结束路径：
- **主动结束**：用户关闭会话或 `/stop`，SessionManager 移除运行时引用，CheckpointManager 最终保存。
- **自动归档**：Sweeper 检测 idle 超时 → 检查无未完成操作 → 标记为 archived（更新 SQLite status + 记录归档时间）→ transcript 移入 archived_sessions/。Sweeper 不通知 SessionManager——映射表在下次 lookup 命中时通过 status 校验感知到归档，自行移除已失效条目。
- **自动清理**：Sweeper 检测 archived 超过 purge TTL → 删除元数据 + transcript 文件。

### 重启恢复

Daemon 启动时，SessionManager 扫描所有 status=active 的 session，对存在未完成操作（PendingOperation）的 session 注入恢复通知，告知 LLM 网关重启前的未完成任务。恢复策略完全由 LLM 自主决定。详细设计见 [session-recovery.md](session-recovery.md)。

### 后台结果注入

```
后台工具或子 session 完成
  → 生成结构化通知消息（含任务标识、完成状态、结果或输出路径）
  → 根据优先级入队消息队列：
      now  ── 立即注入（用户输入前）
      next ── 下一轮对话尽早注入（如卡死警告、子 session 完成）
      later ── 合适时机注入（如普通工具完成）
  → agent 在下一轮 turn 中消费该消息
  → 带去重保护：同一任务只注入一次
```

### Memory Injection 槽位

详见 [session-injection.md](session-injection.md) 消息级注入。

```
active-searcher 写入槽位（tool role 摘要 + 位置模式）
  → 下次 API 调用组装消息时消费槽位
    ├── BeforeNext → 摘要插入消息列表（新消息之前）
    └── AfterCurrent → 摘要插入消息列表（新消息之后）
  → 清空槽位（一次性消费）
```

与通用后台消息队列独立运作，两者可共存于同一批次消息中。

## 模块关系

### 上游

- **Gateway**：用户消息入口，调用 SessionManager 获取/创建 session。
- **Slash Command**：以下斜杠指令类别直接操作 Session 模块（完整指令清单见 slash/README.md Handler 清单）：

  | 类别 | 涉及 Session 的操作 |
  |------|-------------------|
  | 会话生命周期 | `/new` 创建新 session、`/stop` 终止运行（含级联终止子 session） |
  | 工作目录 | `/cd` `/pwd` `/git` 读写 working directory |
  | 模式控制 | `/plan` `/mode` 切换对话模式 |
  | 推理控制 | `/reasoning` 设置推理深度 |
  | 上下文管理 | `/compact` 压缩对话历史、`/system` 管理 system prompt 追加区 |
- **Daemon**：启动时初始化 SqliteStorage 和 SessionConfigProvider，spawn Sweeper 后台任务；系统关闭时委托 SessionManager 统一停止所有 session（详见 daemon/README 关闭路径）；启动时创建 SessionManager，SessionManager 在其初始化过程中自动执行恢复扫描（详见 session-recovery.md）

### 下游

- **System Prompt Builder**：注入链路依赖此模块完成 bootstrap/tools/skills 的组装。
- **LLM Provider**：ConversationSession 构建 API 请求发送给 provider；stop 时通过 cancel token 取消进行中的请求。
- **ToolRegistry**：初始化时向 ToolRegistry 注册 sessions 分组工具（sessions_spawn / sessions_steer / sessions_kill）；注入时获取工具列表和 skill 列表。
- **PersistenceService**：CheckpointManager 通过此 trait 调用具体存储后端。
- **Permission 模块**：Session 向 ToolRegistry 注册 sessions 分组工具（sessions_spawn / sessions_steer / sessions_kill）。工具调用时，tools 模块解析操作上下文后调用 Permission 引擎完成权限检查（详见 session-tools.md）。
- **Config 模块**：sweeper 和 compaction 读取 SessionConfigProvider 获取会话配置参数（idle 超时、compact 阈值等）。
- **Agent 模块**：session 创建时读取 Agent 配置档案，分发 model/workspace/tools/skills/subagents 等字段。sessions_spawn 等工具执行时读取 subagents 和 communication 配置做前置检查。
- **Processor Chain（出站）**：Session 产出的 LLM 响应 ContentBlock[] 经 Gateway 调度进入出站 Processor Chain 做 DSL 解析和日志记录。非直接调用，属数据流下游依赖。
- **Memory 模块**：sub-agent session 结束时通过 hook 触发 memory-miner 记忆挖掘；为每条消息 spawn active-searcher 子 session 进行记忆搜索；在消息组装时消费 `memory_injection` 槽位中的 tool role 记忆摘要。

### 无关

- **Agent 进程生命周期**：Agent 无独立进程；执行状态由 Session 的 session-execution 机制管理。SessionStatus（Active / Archived）描述持久化状态，与 agent 是否在运行无关。
- **IM Adapter**：Session 不参与外部消息路由。
