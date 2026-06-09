# 静态层

## 概述

静态层是 System Prompt 中在 session 生命周期内保持不变的部分，在 session 创建、archive 恢复、compaction 完成时构建，写入 ConversationSession 运行时字段，除非触发缓存失效重建否则内容不变。

## 架构

静态层由两部分组成：bootstrap 文件作为独立 Section，以及三个系统生成的 Section。

### Bootstrap 文件加载

Bootstrap 文件按文件名格式化渲染，每文件以 `## 文件名` 为标题，作为独立 Section 注入 system prompt 前缀。按 Minimal / Full 两种模式选择文件集合：

| 文件 | Minimal | Full |
|------|---------|------|
| AGENTS.md | ✅ | ✅ |
| SOUL.md | ✅ | ✅ |
| IDENTITY.md | ✅ | ✅ |
| USER.md | ✅ | ✅ |
| TOOLS.md | ✅ | ✅ |
| BOOTSTRAP.md | ❌ | ✅ |
| MEMORY.md | ❌ | ❌ |
| HEARTBEAT.md | ❌ | ❌ |

HEARTBEAT.md 不属于 bootstrap 集合——它是 cron 触发时由 agent 按需读取的动态上下文，不注入 system prompt。Bootstrap 文件不存在时跳过，不报错。

### 系统生成的 Section

| Section | 内容 | 来源 |
|---------|------|------|
| ToolsSection | 所有可用工具的分组索引（名称 + 危险度标记 + 常用工具的行为描述） | ToolRegistry |
| SkillListingSection | 可用 skill 的摘要清单（名称 + 描述 + 触发条件） | DiskSkillRegistry |
| MemorySection | 跨 session 的长期记忆 | MEMORY.md |

单个 Section 组装失败时跳过该 Section，其余继续。

ToolsSection 按分组聚合输出，常用工具注入完整行为描述，延迟工具仅注入名称和危险度标记。一级索引有总长度上限，超出时截断。ToolsSection 的实际内容从 ToolRegistry 生成。

SkillListingSection 从 DiskSkillRegistry 获取 skill 数据，按 agent 过滤可见 skill，渲染为摘要清单。若 skill 列表为空，不添加此 Section。listing 内容作为 Section 进入 system prompt，而非 session transcript 中的独立消息。

### Section 级缓存

静态层内容走 session 级 Section 缓存。文件型 Section 基于 mtime 校验——文件未变更时命中缓存，避免重复读取和字符串拼接。工具和 skill 内容通过显式缓存失效触发重建。

此缓存节省本地文件读取和字符串拼接开销，与 API 侧的 KV Cache 是独立的两层优化。

缓存失效触发：
- 文件变更（bootstrap 或 MEMORY.md）→ 对应 Section 缓存失效，下次请求重建
- `/clear` 命令 → 所有静态层缓存失效
- `/system clear` → 清空追加区的同时触发静态层缓存全部失效
- Skill 文件变更 → 文件监听器使 SkillListingSection 缓存失效，下次注入触发时从 registry 获取最新 listing
- 工具定义变更 → 从 ToolRegistry 重新生成 ToolsSection
- Session 恢复 → 强制重建全部静态层，确保内容与最新文件一致
- Compaction → 触发 system prompt 重建回调，强制重建全部静态层

### 兜底与变体

当所有 Section 渲染结果为空时，使用默认 prompt："You are CloseClaw, a helpful AI assistant."。

当 session 没有对应 workspace 目录时，不加载 bootstrap 文件，静态层仅包含 ToolsSection 和 SkillListingSection。

## 数据流

```
SessionManager 创建新 session / 恢复 archive / compaction 完成
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

## 模块关系

### 上游

- **SessionManager**：在 session 创建、archive 恢复、compaction 完成时触发静态层构建。

### 下游

- **Bootstrap Loader**：提供 bootstrap 文件内容，按 Minimal/Full 模式加载。
- **ToolRegistry**：提供 ToolsSection 的分组索引。
- **DiskSkillRegistry**：按 agent 过滤并提供 skill 列表数据。

### 无关

- **LLM Provider**：静态层构建完成后通过 ConversationSession 传递给 LLM provider，构建流程本身不调用 provider。
- **Compaction 模块**：compaction 完成后通过回调触发重建，静态层本身不参与对话消息的压缩逻辑。
