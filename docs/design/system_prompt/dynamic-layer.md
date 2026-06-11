# 动态层

## 概述

动态层是每次 API 请求时即时注入的 System Prompt 后缀，不进持久化存储，不改变 session 的固定 system prompt。

## 架构

动态层由三个 Section 组成，每次 API 请求时由 ConversationSession 直接构建（不走 System Prompt Builder）：

| Section | 内容 | 来源 |
|---------|------|------|
| ChannelContext | 当前会话名称（chat_name） | 入站消息元数据 |
| WorkingDirectory | 当前 session 的工作目录路径 | ConversationSession 运行时字段 |
| GitStatus | 从 workdir 路径派生的 git 分支和变更状态（可通过 `session.git_status` 配置开关控制，默认关闭不注入；非 git 仓库时不注入） | session/working-directory 模块 |

动态层位于静态层和追加区之间，通过边界标记与静态层分隔。

ChannelContext 由 Gateway 以入站上下文结构体传入，不依赖原始 payload。WorkingDirectory 由 ConversationSession 从自身运行时字段读取。GitStatus 由配置开关控制，默认关闭。

### KV Cache 稳定性

动态层是 session 级别内容，默认情况下（GitStatus 关闭时）内容在 session 生命周期内不变——chat_name 和 workdir 在 session 存续期间保持恒定。GitStatus 开启时，内容随工作目录的 git 状态变化而变动。内容恒定时，DeepSeek 等服务端自动前缀缓存的 provider 在连续请求中能持续命中缓存前缀。

## 数据流

```
API 请求到达
  →
  ConversationSession 从自身运行时字段读取 WorkingDirectory
  检查 GitStatus 配置开关 → 开启时派生 git 分支和变更状态
  Gateway 提供入站上下文（chat_name）
  →
  即时组装动态层：ChannelContext + WorkingDirectory + [GitStatus]
  →
  拼接到 system prompt（静态层 + 边界标记 + 动态层）
```

## 模块关系

### 上游

- **Gateway**：每次 API 请求时提供 ChannelContext 所需的 chat_name。
- **Session Config**：提供 `session.git_status` 开关的值，控制 GitStatus 是否注入。

### 下游

- **Cache Adapter**：动态层位于边界标记之后，不参与前缀缓存。cache adapter 对动态层内容透传不做修改。

### 无关

- **System Prompt Builder**：动态层不走 Builder 构建路径，由 ConversationSession 直接组装。
- **SessionCheckpoint**：动态层不持久化，恢复 session 时不恢复动态层。
