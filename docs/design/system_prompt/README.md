# System Prompt

## 概述

- 关联需求文档：[requirements/system_prompt.md](../../requirements/system_prompt.md)

System Prompt 是每次 API 调用时发送给 LLM 的固定前缀，承载 Agent 的身份定义、能力边界和运行上下文，由静态层、动态层、追加区三个独立分区组成，静态层与动态层之间通过边界标记分隔。

构建内容由 Session 类型决定：主 Agent Session 加载必须身份文件、可选内容（自定义引导指令、长期记忆）与工具/技能清单；子 Session 只加载必须身份文件与工具/技能清单；无 Bootstrap 文件的 Session 只加载工具/技能清单。判定规则与降级处理见 [static-layer.md](static-layer.md) §Session 类型与 Bootstrap 文件加载。

## 架构

System Prompt 按内容生命周期分为三个内容分区和一个边界标记：

```
Session 创建 / 恢复 / Compaction / Owner 清除追加指令
  →
  System Prompt Builder
       │
       │  PromptFragmentProvider（统一 trait，详见 [fragment-provider.md](fragment-provider.md)）
       │
       ├── BootstrapFragmentProvider ── Bootstrap 文件（AGENTS.md / SOUL.md 等）
       ├── ToolsFragmentProvider ── 工具分组索引
       ├── SkillsFragmentProvider ── 技能清单（SkillsSection 来源）
       └── MemoryFragmentProvider ── 长期记忆（MemorySection 来源）
       │
       ▼
  ╔══════════════════════════════════════╗
  ║  静态层（Session 生命周期内不变）      ║  ← 走 Session 级 Section 缓存
  ╠══════════════════════════════════════╣
  ║  __SYSTEM_PROMPT_DYNAMIC_BOUNDARY__  ║  ← 切分点（system_prompt 模块切分）
  ╠══════════════════════════════════════╣
  ║  动态层（每请求即时构建）              ║  ← 不参与前缀缓存
  ╠══════════════════════════════════════╣
  ║  追加区（/system+系统注入内容）        ║  ← Owner 内容持久化 / 系统注入不重装
  ╚══════════════════════════════════════╝
       │
       ▼
  Cache Adapter → LLM API
```

子功能文档：

| 文档 | 内容 |
|------|------|
| [static-layer.md](static-layer.md) | 静态层：Bootstrap 文件、系统生成的 Section、Section 级缓存与失效规则 |
| [dynamic-layer.md](dynamic-layer.md) | 动态层：每请求即时注入的 ChannelContext / WorkingDirectory |
| [kv-cache.md](kv-cache.md) | 边界标记 `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 的位置和语义，作为静态/动态区切分的接口合约 |
| [appends.md](appends.md) | 追加区：Owner 动态指令（`/system` 管理）与系统注入内容（如 workflow 上下文）两部分独立分区，Owner 内容持久化不受压缩影响 |
| [fragment-provider.md](fragment-provider.md) | PromptFragmentProvider trait：静态层各数据来源的统一抽象接口 |

## 数据流

### 构建（创建 / 恢复 / 压缩 / Owner 清除追加指令后）

静态层在 Session 创建、归档恢复、压缩完成或 Owner 清除追加指令时由 System Prompt Builder 从 Bootstrap 文件和系统 Section 组装而成，构建结果写入 ConversationSession 运行时字段。完整构建流程见 [static-layer.md](static-layer.md) 数据流。

### 请求（每次 API 调用）

1. API 请求到达
2. ConversationSession 取出静态层（构建时缓存的静态层文本）
3. ConversationSession 即时构建动态层（ChannelContext + WorkingDirectory）
4. ConversationSession 从运行时字段读取追加条目
5. 按序组合：静态层 + `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` + 动态层 + 追加区
6. 以边界标记为界确定静态区与动态区两个字段，传入 [InternalRequest](../common/shared-types.md#internalrequest--internalmessage--systemblock--tooldefinition)（实现可直接分别产出两个字段，不要求先拼接为单串）
7. Cache Adapter 接收已分离的静态区与动态区字段，按供应商策略处理（静态区注入缓存控制参数、动态区标记不可缓存）
8. 发送 LLM 请求

### 恢复

1. 归档 Session（Archived Session）被访问
2. 从 SessionCheckpoint 重建 ConversationSession
3. 强制重新走完整构建流程（checkpoint 不存储组装后的静态层文本，恢复时从最新文件重建；追加区内容从 checkpoint 的 system_appends 字段恢复，不参与重建，详见 [appends.md](appends.md)）
4. 新 System Prompt（重建的静态层 + 边界标记 + 动态层 + 从 checkpoint 恢复的 Owner 动态指令；系统注入内容由功能模块重新注入）替换 ConversationSession 中的旧值

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

### 上游

- **SessionManager**：在 Session 创建、归档恢复、压缩完成与 Owner 清除追加指令时触发 System Prompt 构建。
- **Agent 模块**：提供 agent 配置中的 `bootstrapMode`、`agentDir` 与 `noBootstrap` 字段，用于定位 Bootstrap 文件并判定 Session 类型（bootstrap 模式经 [AgentLookup](../common/core-traits.md#agentlookup) 按 agent_id 查询，agent 配置数据源详见 [agent 模块](../agent/README.md)）。
- **Bootstrap Loader**：提供 Bootstrap 文件内容，按 Session 类型选择文件集。
- **Tools 模块**：经 [ToolRegistryQuery](../common/core-traits.md#toolregistryquery) 提供工具分组索引（ToolsSection）。
- **Memory 模块**：提供 MEMORY.md，作为静态层 System Prompt 的长期记忆段来源。
- **Skills 模块**：经 [SkillListingProvider](../common/core-traits.md#skilllistingprovider) 提供已过滤、排序、格式化的技能清单；system_prompt 模块的 SkillsFragmentProvider 结合 Session 激活状态纳入已激活条件技能并产出 SkillsSection。清单生成规则见 [skills/skill-listing-injection](../skills/skill-listing-injection.md)。
- **Slash 模块**：`/system` 指令向 Session 写入 system_appends（仅 Owner 可用），System Prompt 在每次 API 请求时从 Session 读取并拼入追加区。详细交互见 [slash/system-append](../slash/system-append.md)。
- **Workflow 模块**：流程启动/结束时向追加区写入/移除系统注入上下文。详见 [workflow/session-integration](../workflow/session-integration.md)。
- **Session 压缩流程（Compaction）**：压缩完成后通过回调触发 System Prompt 重建。

### 下游

- **Cache Adapter**：接收已切分的静态区和动态区字段，按供应商策略处理（静态层注入缓存控制参数、动态区标记不可缓存）。详见 [llm/cache-adapter](../llm/cache-adapter.md)。

### 共享类型 / 核心 trait

- [common/core-traits](../common/core-traits.md)（实现：PromptFragmentProvider、SystemPromptBuilder、DynamicPromptBuilder；消费：ToolRegistryQuery、SkillListingProvider、AgentLookup）

### 无关

- **LLM Provider**：构建流程本身不调用 provider，构建完成后通过 ConversationSession 传递。
- **Permission 模块**：System Prompt 构建本身不经过权限检查；工具调用等权限检查在 System Prompt 构建之后由对应链路处理。
