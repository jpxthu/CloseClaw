# 动态层

## 概述

动态层是每次 API 请求时即时注入的 System Prompt 分区，不进持久化存储，不改变 Session 的固定 System Prompt。

## 架构

动态层由两个 Section 组成。DynamicPromptBuilder 由 Gateway 注入 Session，ConversationSession 每次 API 请求时经 [DynamicPromptBuilder](../common/core-traits.md#dynamicpromptbuilder)（common trait）构建动态层（不构成静态层 System Prompt Builder 的一部分）：

| Section | 内容 | 来源 |
|---------|------|------|
| ChannelContext | 会话/群名称（chat_name） | 入站消息元数据 |
| WorkingDirectory | 当前 Session 的工作目录路径 | ConversationSession 运行时字段 |

chat_name 为空时不注入 ChannelContext Section；WorkingDirectory 始终有值（工作目录解析顺序最终回退到系统默认目录）。

动态层位于静态层和追加区之间，通过边界标记与静态层分隔。动态层对所有 Session 类型均加载（含主 Agent Session、子 Session 与「无 Bootstrap 文件的 Session」）——需求 F4 的频道上下文与工作目录在每次 API 调用时均注入。

ChannelContext 由 Gateway 以入站上下文结构体（[RequestContext](../common/shared-types.md#requestcontext)）传入，不依赖原始 payload。WorkingDirectory 由 ConversationSession 从自身运行时字段读取；该字段在 Session 创建/恢复时按 [session §F8](../../requirements/session.md)（工作目录）的解析顺序确定，并可由 `/cd` 在 Session 存续期间改写。DynamicPromptBuilder 返回空时动态层不注入，只保留静态层与追加区（当前 WorkingDirectory 始终有值，正常路径不触发该分支）。

### KV Cache 稳定性

动态层每次 API 请求重新构建。静态区（边界标记之前）的字节稳定性由静态层保证，是前缀缓存的锚点；动态层位于边界标记之后，不参与显式前缀缓存。chat_name 在 Session 存续期间通常不变，workdir 可被 `/cd` 改写，二者变化只影响边界后的动态区，不影响静态区缓存。服务端自动前缀缓存的 provider 仍可因稳定的静态前缀获益。

## 数据流

```
API 请求到达
  →
  ConversationSession 从自身运行时字段读取 WorkingDirectory
  Gateway 提供入站上下文（chat_name）
  →
  ConversationSession 调用 DynamicPromptBuilder 组装动态层：ChannelContext + WorkingDirectory
  →
  拼接到 System Prompt（静态层 + 边界标记 + 动态层 + 追加区）
```

## 模块关系

### 上游

- **Gateway**：每次 API 请求时将入站上下文（含 chat_name）写入 [RequestContext](../common/shared-types.md#requestcontext)，供 ChannelContext 使用。

### 下游

- **Cache Adapter**：动态层位于边界标记之后，不参与前缀缓存。缓存适配层按供应商策略处理动态区（如 Anthropic 封装为不缓存的 SystemBlock，DeepSeek / OpenAI 无额外处理，前缀稳定性由静态层保证），详见 [llm/cache-adapter](../llm/cache-adapter.md)。

### 无关

- **System Prompt Builder**：动态层不走静态层 Builder 构建路径，经 [DynamicPromptBuilder](../common/core-traits.md#dynamicpromptbuilder) 接口组装。
- **SessionCheckpoint**：动态层不持久化，恢复 Session 时不恢复动态层。
