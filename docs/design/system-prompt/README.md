# System Prompt 模块

## 概述

System Prompt 是每次 API 调用时发送给 LLM 的固定前缀，承载 agent 的身份定义、能力边界和运行上下文。

## 架构

System Prompt 由六个 Section 组成，分为静态层和动态层，两层之间通过边界标记分隔。

### 静态层

Session 创建时组装，写入会话持久化存储，在 session 生命周期内保持不变（除非触发重建）。

| Section | 内容 | 来源 |
|---------|------|------|
| RoleSection | Agent 操作规程、角色定义、身份标识、Owner 信息、工具使用规范 | Bootstrap 文件（AGENTS.md、SOUL.md、IDENTITY.md、USER.md、TOOLS.md） |
| ToolsSection | 所有可用工具的分组索引（名称 + 危险度标记 + 常用工具的行为描述） | ToolRegistry |
| SkillListingSection | 可用 skill 的摘要清单（名称 + 描述 + 触发条件） | SkillRegistry |
| MemorySection | 跨 session 的长期记忆 | MEMORY.md |

Bootstrap 文件按固定顺序注入 RoleSection：AGENTS.md（操作规程）→ SOUL.md（角色）→ IDENTITY.md（身份）→ USER.md（Owner）→ TOOLS.md（工具规范）。按 Minimal/Full 两种模式加载，文件集合如下：

| 文件 | Minimal | Full |
|------|---------|------|
| AGENTS.md | ✅ | ✅ |
| SOUL.md | ✅ | ✅ |
| IDENTITY.md | ✅ | ✅ |
| USER.md | ✅ | ✅ |
| TOOLS.md | ✅ | ✅ |
| BOOTSTRAP.md | ❌ | ✅ |
| MEMORY.md | ❌ | ✅ |
| HEARTBEAT.md | ❌ | ❌ |

HEARTBEAT.md 不在此体系内，由 agent 按需单独读取。

ToolsSection 按分组聚合输出，常用工具注入完整行为描述，延迟工具仅注入名称和危险度标记。一级索引有总长度上限，超出时截断。

SkillListingSection 按来源优先级排列，标注条件激活标记。

单个 Section 组装失败时跳过该 Section，其余继续。

### 动态层

每次 API 请求时注入，不进持久化存储，不改变 session 的 system prompt。

| Section | 内容 | 来源 |
|---------|------|------|
| ChannelContext | 当前消息来源（channel 类型、chat_id、发送者） | 入站消息元数据 |
| SessionState | 运行时状态（turnCount、pendingTasks） | ConversationSession |
| AppendSection | Owner 通过 `/system` 命令追加的临时指令，当次生效后自动清除，最大 500 字 | `/system` 命令 |
| GitStatus | 当前工作目录的 git 分支和变更状态 | 工作目录 |

### 边界标记

静态层和动态层之间通过标记分隔，使 API 层可以区分可缓存前缀和必须每次重新计算的后续内容。

### 缓存策略

静态层内容走 session 级缓存。注意：此缓存节省的是本地文件读取和字符串拼接开销，而非 API 侧的 KV Cache。API KV Cache 命中要求请求的完整 token 序列完全相同——对话历史每次增长，完整 payload 每次都不同，KV Cache 无法命中。静态层精简的意义在于直接减少每次发送的 token 数量，降低 API 费用。

缓存失效触发：
- 文件变更（bootstrap 或 MEMORY.md）→ 对应 Section 缓存失效，下次请求重建
- `/clear` 命令 → 所有静态层缓存失效
- Skill 文件变更 → SkillRegistry 通知重建 SkillListingSection
- 工具定义变更 → 重建 ToolsSection
- Session 恢复 → 强制重建全部静态层，确保内容与最新文件一致

Bootstrap 文件不存在时跳过，不报错，其余 Section 正常组装。

### 无 Workspace 的 Session

当 session 没有对应 workspace 目录时，不加载 bootstrap 文件，静态层仅包含 ToolsSection 和 SkillListingSection。

## 数据流

### 构建（Session 创建时）

```
SessionManager 创建新 session
  →
  Bootstrap Loader 按模式加载文件
  ToolRegistry 生成工具分组索引
  SkillRegistry 生成 skill 摘要清单
  读取 MEMORY.md（命中缓存则跳过）
  →
  组装静态层：RoleSection + ToolsSection + SkillListingSection + MemorySection
  →
  写入 ConversationSession
```

### 请求（每次 API 调用时）

```
API 请求到达
  →
  从 ConversationSession 取出静态层
  构建动态层：ChannelContext + SessionState
  →
  拼接：静态层 + 边界标记 + 动态层
  →
  发送 LLM 请求
```

### 恢复

```
Archived session 被访问
  →
  从存储重建 ConversationSession
  →
  强制重新走构建流程（不恢复旧的 system prompt）
  →
  新 system prompt 替换旧的
```

## 模块关系

### 上游

- **SessionManager**：在 session 创建和恢复时触发 system prompt 构建。

### 下游

- **Bootstrap Loader**：提供 bootstrap 文件内容，按 Minimal/Full 模式加载。
- **ToolRegistry**：提供 ToolSection 的分组索引。
- **SkillRegistry**：提供 SkillListingSection 的 skill 摘要清单。

### 无关

- **LLM Provider**（无调用关系）：system prompt 构建完成后通过 ConversationSession 传递给 LLM provider，构建流程本身不调用 provider。
- **Compaction 模块**（无调用关系）：compaction 时 system prompt 独立于对话消息流，不参与压缩。保护策略在 compaction 模块定义。
- **Permission 模块**（无调用关系）：权限检查在 Gateway 层，发生在 system prompt 构建之前。
