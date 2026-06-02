# System Prompt 模块

## 概述

System Prompt 是每次 API 调用时发送给 LLM 的固定前缀，承载 agent 的身份定义、能力边界和运行上下文。

## 架构

System Prompt 由多个 Section 组成，分为静态层、动态层和追加区三层，静态层与动态层之间通过边界标记分隔。追加区是独立分区，持久化在会话状态中，位于 system prompt 末尾。

### 静态层

Session 创建时组装，进入持久化存储，在 session 生命周期内保持不变（除非触发缓存失效重建）。

| Section | 内容 | 来源 |
|---------|------|------|
| RoleSection | Agent 角色定义、身份标识 | IDENTITY.md + SOUL.md |
| ToolsSection | 所有可用工具的分组索引（名称 + 危险度标记 + 常用工具的行为描述） | ToolRegistry |
| SkillListingSection | 可用 skill 的摘要清单（名称 + 描述 + 触发条件） | SkillRegistry / DiskSkillRegistry |
| MemorySection | 跨 session 的长期记忆 | MEMORY.md |

Bootstrap 文件通过 `load_bootstrap_files` 加载，按 Minimal/Full 两种模式选择文件集合：

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

HEARTBEAT.md 不属于 bootstrap 集合——它是 cron 触发时由 agent 按需读取的动态上下文，不注入 system prompt。

加载后的 bootstrap 文件内容按文件名格式化渲染（每文件以 `## 文件名` 为标题），作为独立前缀拼入 system prompt。RoleSection 在此基础上额外整合 IDENTITY.md + SOUL.md 的内容，提供结构化的角色定义（与 bootstrap 文件中的同名文件形成互补：bootstrap 文件保证原始内容完整性，RoleSection 提供 Section 层的语义标注和缓存管理）。

Bootstrap 文件不存在时跳过，不报错，其余 Section 正常组装。

ToolsSection 按分组聚合输出，常用工具注入完整行为描述，延迟工具仅注入名称和危险度标记。一级索引有总长度上限，超出时截断。ToolsSection 的实际内容通过 `build_tools_section` 方法从 ToolRegistry 异步生成。

SkillListingSection 由 SkillRegistry/DiskSkillRegistry 的 `generate_listing` 方法生成。若 skill 列表为空，不添加此 Section。listing 内容作为 Section 进入 system prompt，而非 session transcript 中的独立消息。

单个 Section 组装失败时跳过该 Section，其余继续。

### 动态层

每次 API 请求时注入，不进持久化存储，不改变 session 的 system prompt。

| Section | 内容 | 来源 |
|---------|------|------|
| ChannelContext | 当前消息来源（chat_name、sender_id、timestamp） | 入站消息元数据 |
| SessionState | 运行时状态（turn_count、pending_tasks） | ConversationSession 运行时字段 |
| GitStatus | 从 workdir 路径派生的 git 分支和变更状态（非 git 仓库时不注入） | workdir 模块 |

### 追加区

AppendSection 是 system prompt 末尾的独立分区，由 `/system` 子指令增删管理。追加区持久化在会话状态中，会话恢复时保留。多次 `/system add` 叠加（accumulate），不覆盖。追加区不受上下文压缩影响。与 AGENTS.md 等静态层 Section 无优先级冲突——二者是独立分区。

追加区的详细设计见 [slash/system-append.md](../slash/system-append.md)。

### 边界标记

静态层和动态层之间通过标记分隔，使 API 层可以区分可缓存前缀和必须每次重新计算的后续内容。追加区位于动态层之后、对话历史之前。

### 缓存策略

静态层内容走 session 级 Section 缓存。文件型 Section 基于 mtime 校验：文件未变更时命中缓存，避免重复读取和字符串拼接。工具和 skill 内容通过显式缓存失效触发重建。

注意：此缓存节省的是本地文件读取和字符串拼接开销，而非 API 侧的 KV Cache。API KV Cache 命中要求请求的完整 token 序列完全相同——对话历史每次增长，完整 payload 每次都不同，KV Cache 无法命中。静态层精简的意义在于直接减少每次发送的 token 数量，降低 API 费用。

缓存失效触发：
- 文件变更（bootstrap 或 MEMORY.md）→ 对应 Section 缓存失效，下次请求重建
- `/clear` 命令 → 所有静态层缓存失效
- Skill 文件变更 → 文件监听器使 SkillListingSection 缓存失效，下次 session 重建时从 registry 获取最新 listing
- 工具定义变更 → 重建 ToolsSection（通过 `build_tools_section` 从 ToolRegistry 异步生成新内容）
- Session 恢复 → 强制重建全部静态层，确保内容与最新文件一致

### 优先级 Prompt

System Prompt 构建时按以下优先级检查上层配置的 prompt：

1. overrideSystemPrompt（若设置，完全替代所有 Section 渲染）
2. agentSystemPrompt（若设置）
3. customSystemPrompt（若设置）
4. 默认 prompt（"You are CloseClaw, a helpful AI assistant."）

优先级 prompt 命中后跳过 Section 渲染，仍追加 AppendSection。动态层的 ChannelContext、SessionState、GitStatus 在优先级 prompt 场景下均不注入。

### 无 Workspace 的 Session

当 session 没有对应 workspace 目录时，不加载 bootstrap 文件，静态层仅包含 ToolsSection 和 SkillListingSection。

## 数据流

### 构建（Session 创建时）

```
SessionManager 创建新 session
  →
  load_bootstrap_files 按模式加载文件
  ToolRegistry 生成工具分组索引
  SkillRegistry/DiskSkillRegistry 生成 skill 摘要清单
  读取 MEMORY.md（命中缓存则跳过）
  →
  组装静态层：RoleSection + ToolsSection + SkillListingSection + MemorySection
  →
  写入 ConversationSession.system_prompt
```

### 请求（每次 API 调用时）

```
API 请求到达
  →
  检查优先级 prompt（override → agent → custom → default）
  →
  从 ConversationSession 取出静态层
  构建动态层：ChannelContext + SessionState + GitStatus
  从会话状态读取 AppendSection
  →
  拼接：静态层 + 边界标记 + 动态层 + AppendSection
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
- **ToolRegistry**：提供 ToolsSection 的分组索引。
- **SkillRegistry / DiskSkillRegistry**：提供 SkillListingSection 的 skill 摘要清单。DiskSkillRegistry 管理磁盘加载的 skill，SkillRegistry 管理内置 skill，两者共同作为 skill 数据来源。

### 无关

- **LLM Provider**（无调用关系）：system prompt 构建完成后通过 ConversationSession 传递给 LLM provider，构建流程本身不调用 provider。
- **Compaction 模块**（无调用关系）：system prompt 独立于对话消息流存储，不参与压缩。保护策略在 compaction 模块定义。
- **Permission 模块**（无调用关系）：权限检查在 Gateway 层，发生在 system prompt 构建之前。
