# 动态层

## 概述

动态层是每次 API 请求时即时注入的 System Prompt 后缀，不进持久化存储，不改变 session 的固定 system prompt。

## 架构

动态层由三个 Section 组成，每次 API 请求时由 ConversationSession 直接构建（不走 System Prompt Builder）：

| Section | 内容 | 来源 |
|---------|------|------|
| ChannelContext | 当前消息来源（chat_name、sender_id、timestamp） | 入站消息元数据 |
| WorkingDirectory | 当前 session 的工作目录路径 | ConversationSession 运行时字段 |
| GitStatus | 从 workdir 路径派生的 git 分支和变更状态（非 git 仓库时不注入） | session/working-directory 模块 |

动态层位于静态层和追加区之间，通过边界标记与静态层分隔。

ChannelContext 由 Gateway 以入站上下文结构体传入，不依赖原始 payload。WorkingDirectory 和 GitStatus 由 ConversationSession 从自身运行时字段读取。

### KV Cache 稳定性约束

同一 session 内多次 API 调用间，动态层内容应尽量不变——虽然 cache adapter 不对动态层标记显式缓存控制参数（边界标记之后的内容不参与显式前缀缓存），但支持服务端自动前缀缓存的 provider 仍可因稳定前缀获得更高命中率。每轮必然变化的信息不放在 system prompt 中，改用消息驱动推送——后台任务完成时注入 user 消息，LLM 下轮看到。每轮递增的计数器通过 API metadata 字段传递。

## 数据流

```
API 请求到达
  →
  ConversationSession 从自身运行时字段读取 WorkingDirectory + GitStatus
  Gateway 提供入站上下文（platform、chat_name、sender_id、timestamp）
  →
  即时组装动态层：ChannelContext + WorkingDirectory + GitStatus
  →
  拼接到 system prompt（静态层 + 边界标记 + 动态层）
```

## 模块关系

### 上游

- **Gateway**：每次 API 请求时提供 ChannelContext 所需的入站消息元数据。

### 下游

- **Cache Adapter**：动态层位于边界标记之后，不参与前缀缓存。cache adapter 对动态层内容透传不做修改。

### 无关

- **System Prompt Builder**：动态层不走 Builder 构建路径，由 ConversationSession 直接组装。
- **SessionCheckpoint**：动态层不持久化，恢复 session 时不恢复动态层。
