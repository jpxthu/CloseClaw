# Session 模块

## 概述

Session 模块管理 agent 会话的完整生命周期：创建、消息处理、持久化、压缩、归档与清理。一个 session 代表一次独立的对话上下文，持有 system prompt 和对话历史，是 agent 与 LLM 交互的载体。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [session-lifecycle.md](session-lifecycle.md) | 数据模型、持久化存储、Sweeper 自动归档、状态转换与恢复 |
| [session-injection.md](session-injection.md) | new session 时系统 prompt 如何组装：bootstrap 文件、工具列表、skill 列表的注入链路 |
| [working-directory.md](working-directory.md) | 工作目录的定义：字段、默认值、`/cd` 变更、`/pwd` 读取、system prompt 注入 |
| [compact-process.md](compact-process.md) | 会话上下文压缩：触发机制、LLM summarization、system prompt 隔离保护 |
| [llm-session-enhancements.md](llm-session-enhancements.md) | LLM 交互增强：流式输出、Reasoning Level 控制、Cache Hit 统计、Thinking 内容管理 |

## 架构

Session 模块在系统中有三个层面：

```
Gateway / SessionManager  ← session 生命周期协调者
    │
    ├── CheckpointManager  ← 持久化协调（内存缓存 + PersistenceService）
    │       │
    │       └── SqliteStorage  ← SQLite 元数据 + JSONL transcript 文件
    │
    ├── ConversationSession  ← 运行时对话状态（system_prompt + messages）
    │
    └── ArchiveSweeper  ← 后台定时任务（idle 归档 + 过期清理）
```

- **SessionManager**：session 的创建、查找、恢复入口。协调各组件完成 session 初始化。
- **ConversationSession**：运行时对象，持有 system prompt、消息历史、RunningStats（token/cache 统计）。
- **CheckpointManager**：协调 SessionCheckpoint 的读写缓存和持久化。需要持久化时调用 PersistenceService。
- **SqliteStorage**：生产级持久化后端。SQLite 存元数据，JSONL 文件存 transcript。
- **ArchiveSweeper**：定时后台任务，扫描 idle session 并归档，扫描过期 archive 并清理。

各子功能的关系：
- **生命周期**是骨架：SessionCheckpoint 数据模型和 SqliteStorage 是其他所有功能的底层依赖。
- **注入**在 session 创建时发生：SessionManager 调用 system prompt builder 完成 bootstrap/tools/skills 的组装。
- **压缩**在 session 运行时发生：对过长的对话历史做 summarization。system prompt 独立于对话消息流，不参与压缩，确保角色定义在任意次压缩后完整无损。
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
                → 创建 ConversationSession
                → CheckpointManager.save（首次持久化）
                → 返回 session_id
```

### Session 运行时

```
每次 API 调用前：
  ConversationSession 组装请求（system_prompt + messages + reasoning level）
     → LLM provider 调用
       → 流式模式下：通过 StreamingSink 推送 Text chunks
       → 非流式：返回完整响应
     → extract_message_text 剥离 thinking 标签
     → 新增消息写入 message history
     → RunningStats 更新 token/cache 统计

定期：CheckpointManager 触发持久化（保存 SessionCheckpoint 和 transcript）
```

### Session 终止

两种终止路径：
- **主动结束**：用户关闭会话，SessionManager 移除运行时引用，CheckpointManager 最终保存。
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
  → 恢复完成，继续对话
```

## 模块关系

### 上游

- **Gateway**：用户消息入口，调用 SessionManager 获取/创建 session。
- **Slash Command**：`/compact` 指令触发 compaction 流程。
- **Daemon**：启动时初始化 SqliteStorage 和 SessionConfigProvider，spawn Sweeper 后台任务。

### 下游

- **System Prompt Builder**：注入链路依赖此模块完成 bootstrap/tools/skills 的组装。
- **LLM Provider**：ConversationSession 构建 API 请求发送给 provider。
- **Tool Registry / Skill Registry**：注入时获取工具列表和 skill 列表。
- **PersistenceService**：CheckpointManager 通过此 trait 调用具体存储后端。

### 无关

- **Permission 模块**（无调用关系）：权限检查发生在 Gateway 层，在 session 创建之前。
- **Config 模块**（提供 SessionConfig，但不调用 session 内部逻辑）。
