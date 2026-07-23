# Session 生命周期管理

## 概述

管理 session 从创建到销毁的完整生命周期：定义数据模型（含未完成操作跟踪）、提供持久化存储、运行后台 Sweeper 自动归档空闲会话和清理过期数据、支持已归档会话的按需恢复。

## 架构

### 数据模型

**SessionCheckpoint** 是 session 持久化的核心数据结构，包含：
- 标识：session_id（格式 `{agent_id}_{timestamp}_{random_suffix}`，其中 timestamp 精确到秒、random_suffix 为 8 位小写 hex 随机字符串）、agent_id、role（主 agent / 子 agent）、last_message_id
- 会话路由键：platform（如 feishu）、sender_id（发送者平台内 ID）、peer_id（会话对端：群聊 chat_id 或私聊对方 ID）、account_id（CloseClaw 本地账号标识，由 sender_id 通过身份映射得到。一个 CloseClaw 账号可绑定多个平台的 sender_id）
- 出站定向字段：thread_id（话题 ID，可选。不参与 session_key 计算，仅用于出站时定向回复到正确的话题线）
- 生命周期状态：status（active / archived）、created_at
- 未完成操作：pending_operations（操作发起前持久化、完成后清除。详见 [session-recovery.md](session-recovery.md)）
- 运行时快照：pending_messages（transcript，含消息列表）、mode（对话模式：normal/plan/auto）、mode_state（推理步骤状态）
- system prompt 追加区：system_appends（由 `/system` 斜杠指令增删的追加条目列表。持久化在 checkpoint 中，归档/恢复时完整保留。追加区独立于对话消息流，不参与 compaction）
- 统计：last_message_at（最后消息时间，Sweeper 用来判断 idle）、message_count
- 其他：ttl_seconds、updated_at（最后 checkpoint 更新时间）

> `archived_at` 和扩展元数据（metadata JSON）作为 SQLite 表列存储，由 SqliteStorage 维护，不进入 Checkpoint struct。归档时间和自定义扩展字段通过 SQLite 层查询。

**PendingOperation** 记录了尚未确认完成的操作。每条包含：
- op_id：唯一标识
- op_type：操作类型（ToolCall / SubSessionSpawn / OutboundMessage）
- status：固定为 Running（完成即删除，不持久化完成态）
- detail：类型相关的补充信息
  - ToolCall：工具名 + 参数摘要（不存完整 JSON，transcript 中已有原始 tool_call）
  - SubSessionSpawn：子 session_id + agent 标识 + 任务摘要
  - OutboundMessage：投递目标渠道 + 消息标识 + 投递状态
- created_at：发起时间

写入时机：操作发起前，先追加到 pending_operations 并持久化，确认成功后再执行实际操作。
清除时机：操作完成确认后，从 pending_operations 移除并持久化。

**SessionStatus** 是两态枚举：
- `Active`：正常运行中或待恢复
- `Archived`：已归档，transcript 从 sessions/ 移至 archived_sessions/

### 存储架构

```
~/.closeclaw/
├── sessions.sqlite          ← 元数据（session 记录表）
├── sessions/                ← active transcript（JSONL）
│   └── <session_id>.jsonl
└── archived_sessions/       ← archived transcript（JSONL）
    └── <session_id>.jsonl
```

**SqliteStorage** 是生产级持久化后端，实现 PersistenceService 接口：
- 元数据存储在 SQLite 中：每个 session 一条记录，含状态、时间戳、统计等。
- Transcript 以 JSONL 格式存储在 `sessions/<id>.jsonl`（active）或 `archived_sessions/<id>.jsonl`（archived）。
- archive 操作：更新 SQLite status → 将 transcript 文件从 sessions/ move 到 archived_sessions/。
- restore 操作：将 transcript 文件 move 回 sessions/ → 更新 SQLite status 为 active。
- purge 操作：删除 SQLite 记录 → 删除 transcript 文件。

SQLite 访问通过线程池包装为异步调用，保证不阻塞运行时。

**CheckpointManager** 位于 SessionManager 和 SqliteStorage 之间：
- 持有内存缓存，减少读写磁盘频率。
- 代理持久化操作给 PersistenceService。
- 持有 agent_id 和 role，在保存时注入 SessionCheckpoint。

### Sweeper 机制

**ArchiveSweeper** 是 daemon 启动时 spawn 的后台任务，负责两个定时操作：

1. **Archive**：扫描 status=active 且 last_message_at 超过 idleMinutes 且 **pending_operations 为空** 的 session → 调用 archive，transcript 移入 archived_sessions/。

归档后 Sweeper 不与 SessionManager 通信——映射表同步由 SessionManager 在下次 lookup 时通过 status 校验被动完成（详见 [README.md](README.md) key_registry 自愈逻辑）。
2. **Purge**：扫描 status=archived 且 archived_at 超过 purgeAfterMinutes 的 session → 调用 purge，彻底删除。

**调度策略**：启动后延迟一个完整 interval 再执行首次扫描；Unix 系统上将 Sweeper 进程优先级降低，减少对业务逻辑的 CPU 影响。

### Session 配置

每个 agent 可独立配置 idle 和 purge 阈值：

```
SessionConfigProvider
  ↓ 按 agent_id + role 查询
  ├── per-agent 配置（最高优先级）
  ├── defaults.mainAgent / defaults.subAgent（回退）
  └── 硬编码 fallback（idleMinutes=30, purgeAfterMinutes=0 表示永不过期）
```

配置文件存储在 `config/session.json`，不混入 agents.json。关键参数：
- `sweeperIntervalSeconds`：Sweeper 扫描间隔（默认 300 秒）。
- `idleMinutes`：最后消息后多久标记为 idle、触发 archive。
- `purgeAfterMinutes`：归档后多久彻底删除。设为 0 表示永不过期。
- `compact`：Compaction 相关配置（阈值等）。

子 agent 的 session 有独立的 session_id、独立的生命周期配置，不与主 agent session 混淆。

## 数据流

### Active → Archived 转换

```
Sweeper 定时触发
  → 遍历各 agent 配置，获取 idle 阈值
    → 查询持久化存储：状态为 active、最后消息时间超过 idle 阈值、且 pending_operations 为空的 session
    → 对每条结果：
        → 执行归档
          → 更新 session 状态为 archived，记录归档时间
          → transcript 从活跃区移至归档区
```

归档完成后不通知 SessionManager。SessionManager 在下次 lookup 命中该 key 时，通过 status 校验发现 session 已归档，自行从映射表移除并走未命中回退路径。

### Archived → Active 恢复

```
入站消息到达，映射表未命中
  → SessionManager 通过会话路由字段查询 SQLite
  → 查到一条或多条 archived session → 取 last_message_at 最新的一条
    → transcript 从归档区移回活跃区
    → session 状态更新为 active，清除归档时间
    → 返回 SessionCheckpoint
  → SessionManager 用 checkpoint 重建 ConversationSession
    → 工作目录重新初始化为默认值（不进持久化存储）
    → 重新走注入流程，保证 system prompt 内容最新
  → 注册到映射表
```

多条 archived session 匹配同一 key 时，取 last_message_at 最大的那条——最近活跃的 session 承载最新上下文，对用户最有价值。未命中最新的 session 不会被恢复，随 purgeAfterMinutes 到期后由 Sweeper 清理。

### 恢复时重建策略

上列流程中各组件的职责划分：
- **SqliteStorage**：数据层（文件移回 + 数据库状态更新），输出 SessionCheckpoint
- **SessionManager**：运行时层（重建 ConversationSession + 触发注入 + 注册映射表），向 Gateway 返回恢复标志
- **Gateway**：用户通知（发送「正在恢复会话…」消息）

### Archived → Purge 清理

```
Sweeper（archive 扫描完成后）
  → 遍历各 agent 配置，获取清理阈值
    → 查询持久化存储：状态为 archived 且归档时间超过清理阈值的 session
    → 对每条结果：
        → 执行清理
          → 删除该 session 的数据库记录
          → 删除该 session 的 transcript 文件
```

### CheckpointManager 写入流程

```
SessionManager 需要持久化
  → CheckpointManager 更新内存缓存
  → 委托 PersistenceService 执行持久化
    → 写入或更新 session 元数据到数据库
    → 写入完整 transcript 到文件
```

### 数据一致性校验

SessionManager 在启动时和定期维护中执行 SQLite 与文件系统的双向一致性校验：

**SQLite → 文件系统**：SQLite 中有记录但 transcript 文件不存在 → 视为损坏。删除 SQLite 记录，不注册到映射表。不尝试恢复一个没有 transcript 的 session。

**文件系统 → SQLite**：transcript 文件存在但 SQLite 无对应记录 → 视为孤儿文件。删除文件，不尝试从文件反推元数据（文件不包含 status、路由字段等关键信息）。

**定期维护**：启动时执行一次完整扫描，之后以可配置的间隔（默认每小时）执行增量扫描。扫描优先级低于业务逻辑，不阻塞正常请求处理。

## 模块关系

### 上游

- **Daemon**：启动时初始化 SqliteStorage、SessionConfigProvider、Sweeper。数据一致性校验由 SessionManager 在其初始化过程中自动执行。
- **SessionManager**：session 创建/切换时通过 CheckpointManager 触发持久化。
- **Gateway**：通过 SessionManager 的返回结果获知 session 是否由归档恢复，如是则向用户发送「正在恢复会话...」通知。

### 下游

- **SqliteStorage**（PersistenceService trait 的实现）：SQLite + 文件系统读写。
- **SessionConfigProvider**：读取 `config/session.json`，提供 per-agent 配置。
- **SessionManager（映射表）**：映射表不单独持久化——重建依赖 SessionCheckpoint 中的会话路由键字段。Sweeper 归档后不通知映射表，SessionManager 在下次 lookup 时通过 status 校验自行移除已归档的映射条目。

### 无关

- **Compaction 流程**（无调用关系）：压缩流程通过 Slash Command 触发，修改 ConversationSession 的消息历史并通过 CheckpointManager 触发持久化。压缩不影响生命周期状态机（不改变 status、不触发 archive/restore），但依赖生命周期管理的持久化能力完成 transcript 更新。
- **LLM Session Enhancements**（无调用关系）：流式输出和 reasoning level 通过 ConversationSession 处理，不经过生命周期管理。
