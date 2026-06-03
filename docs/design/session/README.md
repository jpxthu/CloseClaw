# Session 模块

## 概述

Session 模块是 CloseClaw 的运行时载体，管理 session 的全生命周期。一个 session 代表一次独立的 agent 对话实例，其职责分两层：

- **持久化层**：对话上下文的创建、消息处理、压缩、归档与清理。Session 持有 system prompt 和对话历史，是 agent 与 LLM 交互的载体。
- **执行层**：运行时执行状态跟踪（LLM 交互、工具进程、子 session）、级联停止协调、后台任务结果注入。一个 agent 可以有多个 session 同时运行，每个 session 独立管理自己的执行状态。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [session-lifecycle.md](session-lifecycle.md) | 持久化模型：SessionCheckpoint 数据模型、SQLite + JSONL 存储、Sweeper 自动归档与恢复 |
| [session-execution.md](session-execution.md) | 执行状态：三维状态模型（LLM / Tool / 子 Session）、级联停止、后台结果注入 |
| [session-injection.md](session-injection.md) | new session 时系统 prompt 如何组装：bootstrap 文件、工具列表、skill 列表的注入链路 |
| [working-directory.md](working-directory.md) | 工作目录的定义：字段、默认值、`/cd` 变更、`/pwd` 读取、system prompt 注入 |
| [compact-process.md](compact-process.md) | 会话上下文压缩：触发机制、LLM summarization、system prompt 隔离保护 |
| [llm-session-enhancements.md](llm-session-enhancements.md) | LLM 交互增强：流式输出、Reasoning Level 控制、Cache Hit 统计、Thinking 内容管理 |

## 架构

Session 模块由持久化层和执行层两部分组成：

```
Gateway / SessionManager  ← session 生命周期协调者
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
  - **SessionManager**：session 的创建、查找、恢复入口。协调各组件完成 session 初始化。
  - **ConversationSession**：运行时对象，持有 system prompt、消息历史、RunningStats（token/cache 统计）。同时持有执行状态句柄（LLM 状态、工具进程、子 session 引用）。
  - **CheckpointManager**：协调 SessionCheckpoint 的读写缓存和持久化。需要持久化时调用 PersistenceService。
  - **SqliteStorage**：生产级持久化后端。SQLite 存元数据，JSONL 文件存 transcript。
  - **ArchiveSweeper**：定时后台任务，扫描 idle session 并归档，扫描过期 archive 并清理。

- **执行层组件**：
  - **三维执行状态**：LLM 状态、Tool 状态（per-invocation）、子 Session 状态三者独立跟踪，组合判定 session 当前是否空闲。执行状态为纯内存数据，不进持久化——resume 后 session 回到 Idle。
  - **级联停止**：停止一个 session 时，递归停止其所有子 session，杀死该 session 的所有工具进程，取消该 session 正在进行的 LLM 请求。
  - **后台结果注入**：后台工具完成或子 session 完成时，结果通过优先级消息队列（now > next > later）作为消息注入对话流，agent 在下一轮 turn 中消费。

各子功能的关系：
- **生命周期**是持久化骨架：SessionCheckpoint 数据模型和 SqliteStorage 是其他持久化功能的底层依赖。SessionStatus（Active / Archived）描述持久化状态，与执行状态无关。
- **执行状态**是运行时骨架：LLM、Tool、子 Session 三维状态跟踪贯穿每次会话交互，级联停止依赖执行状态做决策，后台结果注入依赖消息队列调度。
- **注入**在 session 创建时发生：SessionManager 调用 system prompt builder 完成 bootstrap/tools/skills 的组装。
- **压缩**在 session 运行时发生：对过长的对话历史做 summarization。支持手动触发（`/compact`）和自动触发（token 用量阈值），内含熔断保护和分级告警。system prompt 独立于对话消息流，不参与压缩，确保角色定义在任意次压缩后完整无损。
- **LLM 增强**贯穿每次 API 调用：流式推送、reasoning level 控制、cache hit 统计在每次会话交互中生效。

## 数据流

### Session 创建

```
用户消息到达
  → SessionManager 查找或创建 session
    → active session 命中？→ 直接返回
    → archived session？→ SqliteStorage.restore → 重建 ConversationSession
    → 新 session？→ 读取 bootstrap 文件 → 组装 system prompt
                                    → ToolRegistry 生成工具描述
                                    → SkillRegistry 生成 skill 列表
                → 创建 ConversationSession（执行状态初始为 Idle）
                → CheckpointManager.save（首次持久化）
                → 返回 session_id
```

### Session 运行时

```
每次 API 调用前：
  ConversationSession 组装请求（system_prompt + messages + reasoning level）
     → LLM 状态设为 Requesting
     → LLM provider 调用
       → 流式模式下：通过 StreamingSink 推送 Text chunks，状态 Receiving
       → 非流式：返回完整响应
     → extract_message_text 剥离 thinking 标签
     → 新增消息写入 message history
     → RunningStats 更新 token/cache 统计
     → LLM 状态回到 Idle（若无其他 pending 操作）

工具调用：
  ConversationSession 注册工具进程句柄
     → Tool 状态设为 Running(Foreground) 或 Running(Background)
     → 前台：session 阻塞等待完成 → 完成 → 注销句柄
     → 后台：session 不阻塞，进程句柄保留 → 完成时结果注入消息队列

定期：CheckpointManager 触发持久化（保存 SessionCheckpoint 和 transcript）
```

### Session 停止

停止入口有三种：

- **斜杠指令**（`/stop`）：用户在 session 内输入，停当前 session。可选级联（`/stop --cascade`）。
- **父 session 停止**：父 session 被停时，级联停止其所有子 session。
- **优雅关闭**：系统关闭时遍历所有活跃 session 逐个停止。

停止流程：

```
session.stop(cascade)
  ↓
  取得该 session 的执行句柄
  ├── 级联（若 cascade=true）：对所有 child session 递归调用 stop(true)
  ├── 杀工具进程：遍历 tool_handles → 前台进程 kill、后台进程 kill
  ├── cancel LLM：若 llm_state 为 Requesting 或 Receiving → cancel token
  └── 注销句柄：清空 tool_handles、child_handles、llm_state 置 Idle
  ↓
  结束后：SessionManager 移除运行时引用 → CheckpointManager 最终保存
```

### Session 结束路径

两种结束路径：
- **主动结束**：用户关闭会话或 `/stop`，SessionManager 移除运行时引用，CheckpointManager 最终保存。
- **自动归档**：Sweeper 检测 idle 超时 → 标记为 archived → transcript 移入 archived_sessions/ → SQLite 更新状态。
- **自动清理**：Sweeper 检测 archived 超过 purge TTL → 删除元数据 + transcript 文件。

### Session 恢复

```
再次访问已归档 session
  → SessionManager 发现 status=archived
  → 通知用户 "正在恢复会话..."
  → SqliteStorage.restore_session
    → transcript 移回 sessions/
    → SQLite status 更新为 active
    → 重建 ConversationSession（重新走注入流程，保证 prompt 内容最新）
    → 执行状态初始为 Idle
  → 恢复完成，继续对话
```

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

## 模块关系

### 上游

- **Gateway**：用户消息入口，调用 SessionManager 获取/创建 session。
- **Slash Command**：`/compact` 指令触发 compaction 流程，`/stop` 指令触发执行停止。
- **Daemon**：启动时初始化 SqliteStorage 和 SessionConfigProvider，spawn Sweeper 后台任务；优雅关闭时遍历 session 执行停止。

### 下游

- **System Prompt Builder**：注入链路依赖此模块完成 bootstrap/tools/skills 的组装。
- **LLM Provider**：ConversationSession 构建 API 请求发送给 provider；stop 时通过 cancel token 取消进行中的请求。
- **Tool Registry / Skill Registry**：注入时获取工具列表和 skill 列表。
- **PersistenceService**：CheckpointManager 通过此 trait 调用具体存储后端。
- **Config 模块**：sweeper 和 compaction 读取 SessionConfigProvider 获取会话配置参数（idle 超时、compact 阈值等）。
- **Agent 模块**：session 创建时读取 Agent 配置档案，分发 model/workspace/tools/skills/subagents 等字段。

### 无关

- **Permission 模块**（无调用关系）：权限检查发生在 Gateway 层（入站门控），工具调用权限评估由 tools 模块触发，Session 自身不直接调用 Permission。
- **Agent 进程生命周期**：Agent 无独立进程；执行状态由 Session 的 session-execution 机制管理。SessionStatus（Active / Archived）描述持久化状态，与 agent 是否在运行无关。
- **Processor Chain / Renderer**：消息出站处理不经过 Session 模块。
