# 静态层

## 概述

静态层是 System Prompt 中在 session 生命周期内保持不变的部分，在 session 创建、archive 恢复、compaction 完成时构建，写入 ConversationSession 运行时字段，除非触发缓存失效重建否则内容不变。

## 架构

静态层由两部分组成：bootstrap 文件作为独立 Section，以及三个系统生成的 Section。

### Bootstrap 文件加载

Bootstrap 文件按文件名格式化渲染，每文件以 `## 文件名` 为标题，作为独立 Section 注入 system prompt 前缀。多文件按固定顺序注入，AGENTS.md（操作规程）排在最高优先级（最前）。按 Minimal / Full 两种模式选择文件集合：

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
| SkillsSection | 技能清单（user-invocable 技能 + 当前 session 已激活的条件技能） | SkillRegistry |
| MemorySection | 跨 session 的长期记忆 | MEMORY.md |

动态层的 ChannelContext、WorkingDirectory、ModeInstruction、GitStatus 四个 Section 不属于静态层，由每次请求即时构建，详见 [dynamic-layer.md](dynamic-layer.md)。

单个 Section 组装失败时跳过该 Section，其余继续。

ToolsSection 按分组聚合输出，常用工具注入完整行为描述，延迟工具仅注入名称和危险度标记。一级索引有总长度上限，超出时截断。ToolsSection 的实际内容从 ToolRegistry 生成。

MemorySection 仅在主 Agent 会话（Full 模式）时生成——子 Agent 会话（Minimal 模式）不加载长期记忆，MemoryFragmentProvider 返回空 Fragment。详见 [fragment-provider.md](fragment-provider.md) MemoryFragmentProvider 行为。

SkillsSection 从 SkillRegistry 获取当前可用技能并渲染为格式化清单。清单仅包含已声明 user-invocable 的技能和当前 session 已条件激活的技能（paths 匹配）。子 Agent 会话与主 Agent 会话均加载 SkillsSection。清单的过滤、排序、格式化规则见 [skills/skill-listing-injection](../skills/skill-listing-injection.md)。SkillsSection 为空时不注入对应段落。

### Section 级缓存

静态层内容走 session 级 Section 缓存。文件型 Section 基于 mtime 校验——文件未变更时命中缓存，避免重复读取和字符串拼接。工具内容通过显式缓存失效触发重建。

此缓存节省本地文件读取和字符串拼接开销，与 API 侧的 KV Cache 是独立的两层优化。

缓存失效触发：
- 文件变更（bootstrap 或 MEMORY.md）→ 对应 Section 缓存失效，下次请求重建
- 技能注册中心变更（skill 文件新增/修改/删除）→ SkillsSection 缓存失效，下次 SP 组装时重建
- `/clear` 命令 → 所有静态层缓存失效
- `/system clear` → 清空追加区的同时触发静态层缓存全部失效
- 工具定义变更 → 从 ToolRegistry 重新生成 ToolsSection
- Session 恢复 → 强制重建全部静态层，确保内容与最新文件一致
- Compaction → 触发 system prompt 重建回调，强制重建全部静态层

### 兜底与变体

当所有 Section 渲染结果为空时，使用默认 prompt："You are CloseClaw, a helpful AI assistant."。

当 session 没有对应 workspace 目录时，不加载 bootstrap 文件，MemorySection 同样跳过（MEMORY.md 属于 workspace 文件），SkillsSection 正常加载（技能来源不受 workspace 影响），静态层包含 ToolsSection 和 SkillsSection。

## 数据流

```
1. SessionManager 创建新 session / 恢复 archive / compaction 完成
2. builder 通过 Bootstrap Loader 按模式加载 bootstrap 文件
3. ToolRegistry 生成工具分组索引
4. SkillRegistry 渲染技能清单（user-invocable + 已激活的条件技能）
5. Full 模式下读取 MEMORY.md（命中缓存则跳过）；Minimal 模式跳过
6. 组装静态层：bootstrap 文件 + ToolsSection + SkillsSection + MemorySection（Minimal 模式不含 MemorySection；无 workspace 目录时不含 bootstrap 和 MemorySection，详见兜底与变体）
7. 写入 ConversationSession 的 system prompt 字段（运行时字段，不进 SessionCheckpoint）
```

## 模块关系

### 上游

- **SessionManager**：在 session 创建、archive 恢复、compaction 完成时触发静态层构建。
- **Bootstrap Loader**：提供 bootstrap 文件内容，按 Minimal/Full 模式加载。
- **ToolRegistry**：提供 ToolsSection 的分组索引。
- **Compaction 模块**：compaction 完成后通过回调触发静态层重建。

### 下游

- **ConversationSession**：构建完成后写入其 system prompt 字段，每次 API 请求时取出使用。

### 无关

（无）
