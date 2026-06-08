# System Prompt 模块

## 概述

System Prompt 是每次 API 调用时发送给 LLM 的固定前缀，承载 agent 的身份定义、能力边界和运行上下文。

## 架构

System Prompt 由多个 Section 组成，分为静态层、动态层和追加区三层，静态层与动态层之间通过边界标记分隔。追加区是独立分区，持久化在会话状态中，位于 system prompt 末尾。

### 静态层

Session 创建时组装，写入 ConversationSession 运行时字段，在 session 生命周期内保持不变（除非触发缓存失效重建）。

静态层由两部分组成：bootstrap 文件作为独立 Section，系统生成三个 Section。

Bootstrap 文件按文件名格式化渲染，每文件以 `## 文件名` 为标题，作为独立 Section 注入 system prompt 前缀。按 Minimal/Full 两种模式选择文件集合：

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

加载后的 bootstrap 文件内容按文件名格式化渲染（每文件以 `## 文件名` 为标题），作为独立 Section 注入 system prompt 前缀。Bootstrap 文件不存在时跳过，不报错。

系统生成的 Section：

| Section | 内容 | 来源 |
|---------|------|------|
| ToolsSection | 所有可用工具的分组索引（名称 + 危险度标记 + 常用工具的行为描述） | ToolRegistry |
| SkillListingSection | 可用 skill 的摘要清单（名称 + 描述 + 触发条件） | DiskSkillRegistry |
| MemorySection | 跨 session 的长期记忆 | MEMORY.md |

单个 Section 组装失败时跳过该 Section，其余继续。

ToolsSection 按分组聚合输出，常用工具注入完整行为描述，延迟工具仅注入名称和危险度标记。一级索引有总长度上限，超出时截断。ToolsSection 的实际内容从 ToolRegistry 生成。

SkillListingSection 从 DiskSkillRegistry 获取 skill 数据，按 agent 过滤可见 skill，渲染为摘要清单。若 skill 列表为空，不添加此 Section。listing 内容作为 Section 进入 system prompt，而非 session transcript 中的独立消息。

### 动态层

每次 API 请求时注入，不进持久化存储，不改变 session 的 system prompt。

| Section | 内容 | 来源 |
|---------|------|------|
| ChannelContext | 当前消息来源（chat_name、sender_id、timestamp） | 入站消息元数据 |
| WorkingDirectory | 当前 session 的工作目录路径 | ConversationSession 运行时字段 |
| GitStatus | 从 workdir 路径派生的 git 分支和变更状态（非 git 仓库时不注入） | session/working-directory 模块 |

动态层内容的约束：同一 session 内多次 API 调用间，动态层内容应尽量不变，确保 KV cache 前缀命中。每轮必然变化的信息（如后台任务状态）不放在 system prompt 中，改用消息驱动推送（task 完成 → 注入 user 消息 → LLM 下轮看到）。每轮递增的计数器（如 turn_count）通过 API metadata 字段传递。

### 追加区

AppendSection 是 system prompt 末尾的独立分区，持久化在会话状态中，不受上下文压缩影响。与 AGENTS.md 等静态层 Section 无优先级冲突——二者是独立分区。

由 `/system` 指令增删管理：
- `/system add <内容>`：追加文本（多次叠加，会话恢复时保留）
- `/system clear`：清空追加内容
- `/system list`：查看当前追加列表

详细设计见 [slash/system-append](docs/design/slash/system-append.md)。

### 边界标记

静态层和动态层之间通过 `STATIC_LAYER_END` 标记分隔。该标记是 cache adapter 的程序输入——cache adapter 以标记为切分点，标记之前的内容作为可缓存前缀（在支持前缀缓存的 provider 上标记 `cache_control`），标记之后的内容每次请求重新计算，不参与前缀缓存。追加区位于动态层之后、对话历史之前。

### 缓存策略

静态层内容走 session 级 Section 缓存。文件型 Section 基于 mtime 校验：文件未变更时命中缓存，避免重复读取和字符串拼接。工具和 skill 内容通过显式缓存失效触发重建。

此缓存节省本地文件读取和字符串拼接开销，与 API 侧的 KV Cache 是独立的两层优化。API KV Cache 通过 cache adapter 层实现——静态层和动态层通过边界标记分离后，cache adapter 在静态层上标记缓存控制参数，使支持前缀缓存的 provider（Anthropic、Kimi）复用静态前缀的 KV cache，仅对动态层和新增消息计费。对于仅支持完全匹配缓存的 provider，通过精简静态层总 token 量来降低每次请求的成本。

缓存失效触发：
- 文件变更（bootstrap 或 MEMORY.md）→ 对应 Section 缓存失效，下次请求重建
- `/clear` 命令 → 所有静态层缓存失效
- Skill 文件变更 → 文件监听器使 SkillListingSection 缓存失效，下次 session 创建、archive 恢复或 compaction 发生时从 registry 获取最新 listing
- 工具定义变更 → 从 ToolRegistry 重新生成 ToolsSection
- Session 恢复 → 强制重建全部静态层，确保内容与最新文件一致
- Compaction → 触发 system prompt 重建回调，强制重建全部静态层，确保继续对话时角色定义、工具列表、skill 清单和长期记忆均为最新版本

### 默认 Prompt

当所有 Section 渲染结果为空时，使用默认 prompt："You are CloseClaw, a helpful AI assistant."。

### 无 Workspace 的 Session

当 session 没有对应 workspace 目录时，不加载 bootstrap 文件，静态层仅包含 ToolsSection 和 SkillListingSection。

## 数据流

### 构建（Session 创建时）

```
SessionManager 创建新 session
  →
  builder 通过 Bootstrap Loader 按模式加载 bootstrap 文件
  ToolRegistry 生成工具分组索引
  DiskSkillRegistry 提供 skill 列表数据
  读取 MEMORY.md（命中缓存则跳过）
  →
  组装静态层：bootstrap 文件 + ToolsSection + SkillListingSection + MemorySection
  →
  写入 ConversationSession 的 system prompt 字段（运行时字段，不进 SessionCheckpoint）
```

### 请求（每次 API 调用时）

```
API 请求到达
  →
  从 ConversationSession 取出静态层
  构建动态层：ChannelContext + WorkingDirectory + GitStatus
  从会话状态读取 AppendSection
  →
  拼接：静态层 + 边界标记 + 动态层 + AppendSection
  →
  cache adapter 以边界标记为切分点注入缓存控制参数
  →
  发送 LLM 请求
```

### 恢复

```
Archived session 被访问
  →
  从 SessionCheckpoint 重建 ConversationSession
  →
  强制重新走完整构建流程（checkpoint 不存储 system prompt，恢复时从最新文件重建）
  →
  新 system prompt 替换 ConversationSession 中的旧值
```

## 模块关系

### 上游

- **SessionManager**：在 session 创建和恢复时触发 system prompt 构建。

### 下游

- **Bootstrap Loader**：提供 bootstrap 文件内容，按 Minimal/Full 模式加载。
- **ToolRegistry**：提供 ToolsSection 的分组索引。
- **DiskSkillRegistry**：按 agent 过滤并提供 skill 列表数据，读取磁盘 skill 并合并内置 SkillRegistry 的 skill，按优先级排序。
- **Slash 模块**：`/system` 指令操作 AppendSection，详细设计见 [slash/system-append](docs/design/slash/system-append.md)。
- **Cache Adapter**：以边界标记为切分点，对静态层注入缓存控制参数。system prompt 组装完成后经 cache adapter 处理再进入 LLM client。

### 无关

- **LLM Provider**（无调用关系）：system prompt 构建完成后通过 ConversationSession 传递给 LLM provider，构建流程本身不调用 provider。
- **Compaction 模块**（间接关联）：compaction 完成后通过回调触发 system prompt 重建，确保 Section 内容与最新文件一致。system prompt 本身不参与对话消息的压缩逻辑。
- **Permission 模块**（无调用关系）：权限检查在 Gateway 层，发生在 system prompt 构建之前。
