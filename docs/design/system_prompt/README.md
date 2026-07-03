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
       │  PromptFragmentProvider（统一 trait，详见 [fragment-provider.md](fragment-provider.md)）
       │
       ├── BootstrapFragmentProvider ── bootstrap 文件（AGENTS.md / SOUL.md 等）
       ├── ToolsFragmentProvider ── 工具分组索引
       ├── SkillsFragmentProvider ── skill 清单
       └── MemoryFragmentProvider ── 长期记忆（MemorySection 来源）
       │
       ▼
  ╔════════════════════════╗
  ║  静态层（session 持久）  ║  ← 走 Section 级缓存
  ╠════════════════════════╣
  ║  __SYSTEM_PROMPT_DYNAMIC_BOUNDARY__      ║  ← 切分点（system_prompt 模块切分）
  ╠════════════════════════╣
  ║  动态层（每请求即时构建）  ║  ← 默认内容不变，不参与显式前缀缓存标记
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
| [dynamic-layer.md](dynamic-layer.md) | 动态层：每请求即时注入的 ChannelContext / WorkingDirectory / GitStatus（可配置关闭） |
| [kv-cache.md](kv-cache.md) | 边界标记 `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 的位置和语义，作为静态/动态区切分的接口合约 |
| [appends.md](appends.md) | 追加区：`/system` 指令管理的独立分区，持久化不受压缩影响 |
| [fragment-provider.md](fragment-provider.md) | PromptFragmentProvider trait：静态层各数据来源的统一抽象接口 |

## 数据流

### 构建（创建 / 恢复 / 压缩后）

静态层在 session 创建、archive 恢复或 compaction 完成时由 System Prompt Builder 从 bootstrap 文件和系统 Section 组装而成，构建结果写入 ConversationSession 运行时字段。完整构建流程见 [static-layer.md](static-layer.md) 数据流。

### 请求（每次 API 调用）

```
API 请求到达
  →
  ConversationSession 取出静态层（构建时缓存的完整结果）
  ConversationSession 即时构建动态层（ChannelContext + WorkingDirectory + [GitStatus，默认关闭]）
  ConversationSession 从运行时字段读取追加条目
  →
  拼接：静态层 + __SYSTEM_PROMPT_DYNAMIC_BOUNDARY__ + 动态层 + 追加区
  →
  以边界标记为切分点，分离静态区和动态区为独立字段，传入 InternalRequest
  →
  cache adapter 接收已分离的字段，注入缓存控制参数
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
- **Memory 模块**：提供 MEMORY.md，作为 static system prompt 的长期记忆段来源。
- **Slash 模块**：`/system` 指令向 Session 写入 system_appends，system prompt 在每次 API 请求时从 Session 读取并拼入追加区。详细交互见 [slash/system-append](docs/design/slash/system-append.md)。

### 下游

- **PromptFragmentProvider**（定义见 [common/core-traits](../common/core-traits.md#promptfragmentprovider)）：System Prompt Builder 通过此 trait 获取各来源的片段。各 Provider 实现分别调用 Bootstrap Loader、ToolRegistry、DiskSkillRegistry 和 MEMORY.md。
- **Cache Adapter**：接收已切分的静态区和动态区字段，对静态层注入缓存控制参数。详见 [llm/cache-adapter](docs/design/llm/cache-adapter.md)。

### 无关

- **LLM Provider**：构建流程本身不调用 provider，构建完成后通过 ConversationSession 传递。
- **Compaction 模块**：压缩完成后通过回调触发重建，但 system prompt 不参与消息压缩逻辑。
- **Permission 模块**：权限检查在 Gateway 层，发生在 system prompt 构建之前。
