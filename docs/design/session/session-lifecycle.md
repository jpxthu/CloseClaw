# Session 生命周期管理

## 概述

管理 session 从创建到销毁的完整生命周期：定义数据模型、提供持久化存储、运行后台 Sweeper 自动归档空闲会话和清理过期数据、支持已归档会话的恢复。

## 架构

### 数据模型

**SessionCheckpoint** 是 session 持久化的核心数据结构，包含：
- 标识：session_id、agent_id、role（主 agent / 子 agent）、last_message_id
- 路由信息：channel（如 feishu）、chat_id
- 生命周期状态：status（active / archived）、created_at、archived_at
- 运行时快照：pending_messages（transcript，含消息列表）、mode（对话模式：direct/plan/stream）、mode_state（推理步骤状态）
- 统计：last_message_at（最后消息时间，Sweeper 用来判断 idle）、message_count
- 其他：metadata（JSON 扩展字段）、ttl_seconds、updated_at（最后 checkpoint 更新时间）

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

**SqliteStorage** 是生产级持久化后端，实现 PersistenceService trait：
- 元数据存储在 SQLite 中：每个 session 一条记录，含状态、时间戳、统计等。
- Transcript 以 JSONL 格式存储在 `sessions/<id>.jsonl`（active）或 `archived_sessions/<id>.jsonl`（archived）。
- archive 操作：更新 SQLite status → 将 transcript 文件从 sessions/ move 到 archived_sessions/。
- restore 操作：将 transcript 文件 move 回 sessions/ → 更新 SQLite status 为 active。
- purge 操作：删除 SQLite 记录 → 删除 transcript 文件。

SQLite 访问通过线程池包装为异步调用，保证不阻塞运行时。

**CheckpointManager** 位于 SessionManager 和 SqliteStorage 之间：
- 持有内存缓存（RwLock<HashMap>），减少读写磁盘频率。
- 代理 save/load/delete 操作给 PersistenceService。
- 持有 agent_id 和 role，在保存时注入 SessionCheckpoint。

### Sweeper 机制

**ArchiveSweeper** 是 daemon 启动时 spawn 的后台任务，负责两个定时操作：

1. **Archive**：扫描 status=active 且 last_message_at 超过 idleMinutes 的 session → 调用 archive，transcript 移入 archived_sessions/。
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

配置文件独立存储在 `session_config.json`，不混入 agents.json。关键参数：
- `sweeperIntervalSeconds`：Sweeper 扫描间隔（默认 300 秒）。
- `idleMinutes`：最后消息后多久标记为 idle、触发 archive。
- `purgeAfterMinutes`：归档后多久彻底删除。设为 0 表示永不过期。
- `compact`：Compaction 相关配置（阈值等）。

子 agent 的 session 有独立的 session_id、独立的生命周期配置，不与主 agent session 混淆。

## 数据流

### Active → Archived 转换

```
Sweeper 定时触发 run_once
  → 遍历各 agent 配置
    → 查询 SQLite：status=active AND last_message_at < now - idleMinutes
    → 对每条结果：
        → SqliteStorage.archive_session(session_id)
          → 更新 SQLite：status='archived', archived_at=now
          → 文件 move：sessions/<id>.jsonl → archived_sessions/<id>.jsonl
```

### Archived → Active 恢复

```
用户再次访问已归档 session
  → SessionManager 或 Gateway 检测到 status=archived
  → Gateway 发送 "正在恢复会话..." 通知
  → SqliteStorage.restore_session(session_id)
    → 文件 move：archived_sessions/<id>.jsonl → sessions/<id>.jsonl
    → 更新 SQLite：status='active', archived_at=NULL
    → 返回 SessionCheckpoint
  → SessionManager 用 checkpoint 重建 ConversationSession
    → 重新走注入流程（build_from_workspace），保证 system prompt 内容最新
```

### 恢复时重建策略

恢复时 SqliteStorage 只负责数据层（文件移回 + DB 状态更新），返回 SessionCheckpoint。SessionManager 拿到 checkpoint 后用其数据重建 ConversationSession 运行时对象，并重新走注入流程以保证 system prompt 内容最新。

### Archived → Purge 清理

```
Sweeper 在 run_once 中（archive 扫描完成后）
  → 遍历各 agent 配置
    → 查询 SQLite：status=archived AND archived_at < now - purgeAfterMinutes
    → 对每条结果：
        → SqliteStorage.purge_session(session_id)
          → DELETE FROM sessions WHERE id=?
          → 删除 archived_sessions/<id>.jsonl 文件
```

### CheckpointManager 写入流程

```
SessionManager 需要持久化
  → CheckpointManager.save(checkpoint)
    → 更新内存缓存（RwLock）
    → 调用 PersistenceService.save_checkpoint(checkpoint)
      → SqliteStorage：线程池执行
        → INSERT OR REPLACE INTO sessions (...)
        → 写入 sessions/<id>.jsonl（完整 transcript）
```

## 模块关系

### 上游

- **Daemon**：启动时初始化 SqliteStorage、SessionConfigProvider、Sweeper。
- **SessionManager**：session 创建/切换时通过 CheckpointManager 触发持久化。
- **Gateway**：访问 session 时检查 status，若为 archived 则触发 restore 流程并发送通知。

### 下游

- **SqliteStorage**（PersistenceService trait 的实现）：SQLite + 文件系统读写。
- **SessionConfigProvider**：读取 session_config.json，提供 per-agent 配置。

### 无关

- **Compaction 流程**（无调用关系）：压缩流程通过 Slash Command 触发，修改 ConversationSession 的消息历史并通过 CheckpointManager.save 持久化。压缩不影响生命周期状态机（不改变 status、不触发 archive/restore），但依赖生命周期管理的持久化能力完成 transcript 更新。
- **LLM Session Enhancements**（无调用关系）：流式输出和 reasoning level 通过 ConversationSession 处理，不经过生命周期管理。
