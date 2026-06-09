# System Prompt

## 概述

System Prompt 是每次 API 调用时发送给 LLM 的固定前缀，承载 agent 的身份定义、能力边界和运行上下文，由静态层、动态层、追加区三个独立分区组成，静态层与动态层之间通过边界标记分隔。

## 架构

System Prompt 按内容生命周期分为三个内容分区和一个边界标记：

```
Session 创建 / 恢复 / Compaction
  →
  System Prompt Builder
       │
       ├── Bootstrap Loader ── bootstrap 文件（AGENTS.md / SOUL.md 等）
       ├── ToolRegistry ── 工具分组索引
       ├── DiskSkillRegistry ── skill 清单
       └── MEMORY.md ── 长期记忆（MemorySection 来源）
       │
       ▼
  ╔════════════════════════╗
  ║  静态层（session 持久）  ║  ← 走 Section 级缓存
  ╠════════════════════════╣
  ║  __SYSTEM_PROMPT_DYNAMIC_BOUNDARY__      ║  ← cache adapter 的切分点
  ╠════════════════════════╣
  ║  动态层（每请求即时构建）  ║  ← 不参与前缀缓存
  ╠════════════════════════╣
  ║  追加区（/system 管理）  ║  ← 独立分区，持久化
  ╚════════════════════════╝
       │
       ▼
  Cache Adapter → LLM API
```

子功能文档：

| 文档 | 内容 |
|------|------|
| [static-layer.md](static-layer.md) | 静态层：bootstrap 文件、系统生成的 Section、Section 级缓存与失效规则 |
| [dynamic-layer.md](dynamic-layer.md) | 动态层：每请求即时注入的 ChannelContext / WorkingDirectory / GitStatus，KV Cache 稳定性约束 |
| [kv-cache.md](kv-cache.md) | 边界标记 `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 的位置和语义，作为 cache adapter 的切分合约 |
| [appends.md](appends.md) | 追加区：`/system` 指令管理的独立分区，持久化不受压缩影响 |

## 数据流

### 构建（创建 / 恢复 / 压缩后）

静态层在 session 创建、archive 恢复或 compaction 完成时由 System Prompt Builder 从 bootstrap 文件和系统 Section 组装而成，构建结果写入 ConversationSession 运行时字段。完整构建流程见 [static-layer.md](static-layer.md) 数据流。

### 请求（每次 API 调用）

```
API 请求到达
  →
  ConversationSession 取出静态层（构建时缓存的完整结果）
  ConversationSession 即时构建动态层（ChannelContext + WorkingDirectory + GitStatus）
  ConversationSession 从运行时字段读取追加条目
  →
  拼接：静态层 + __SYSTEM_PROMPT_DYNAMIC_BOUNDARY__ + 动态层 + 追加区
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
- **Slash 模块**：`/system` 指令向 Session 写入 system_appends，system prompt 在每次 API 请求时从 Session 读取并拼入追加区。详细交互见 [slash/system-append](docs/design/slash/system-append.md)。

### 下游

- **Bootstrap Loader**：提供 bootstrap 文件内容，按 Minimal/Full 模式加载。
- **ToolRegistry**：提供 ToolsSection 的分组索引。
- **DiskSkillRegistry**：按 agent 过滤并提供 skill 列表数据。
- **Cache Adapter**：以边界标记为切分点，对静态层注入缓存控制参数。详见 [llm/cache-adapter](docs/design/llm/cache-adapter.md)。

### 无关

- **LLM Provider**：构建流程本身不调用 provider，构建完成后通过 ConversationSession 传递。
- **Compaction 模块**：压缩完成后通过回调触发重建，但 system prompt 不参与消息压缩逻辑。
- **Permission 模块**：权限检查在 Gateway 层，发生在 system prompt 构建之前。
