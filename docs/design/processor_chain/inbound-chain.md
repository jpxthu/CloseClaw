# 入站链路

## 概述

入站 Processor 链在所有 IM 平台的归一化之后运行。各 IM Adapter 的入站部分先将平台特定格式转为 NormalizedMessage，链再统一处理日志、会话路由、内容清洗和格式标准化。

Processor 链不感知平台差异——平台特有的解析和适配逻辑由 IM Adapter 负责。

## 架构

```
IM 平台消息到达
  ↓
IM Adapter（入站）— 平台特定解析，产 NormalizedMessage
  ↓
NormalizedMessage（统一中间结构）
  ┌─────────────────────────────┐
  │ platform    — 来源平台标识   │
  │ sender_id   — 发送者标识     │
  │ peer_id     — 会话对端标识   │
  │ thread_id   — 话题标识（可选）│
  │ account_id  — 租户标识（可选）│
  │ content     — 消息文本内容    │
  │ timestamp   — 消息时间       │
  └─────────────────────────────┘
  ↓
Processor 链（按 priority 升序执行）
  ├── RawLogProcessor（priority 10）
  │     → 原始消息写入日志
  │
  ├── SessionRouter（priority 20）
  │     → 根据 (platform, sender_id, peer_id) 计算 session key
  │     → account_id 在 DmScope 配置为 PerAccount 时参与计算
  │     → thread_id 不参与 session key 计算（仅用于出站定向回复）
  │     → 查找或创建 session，session_id 写入 metadata
  │
  ├── MessageCleaner（priority 30）
  │     → 清洗消息内容（各平台 IM Adapter 可能残留的元数据）
  │     → 富文本展开为 markdown
  │
  └── MarkdownNormalizer（priority 40）
        → 标准化 markdown 格式
  ↓
ProcessedMessage → Gateway 路由
```

SessionRouter 不区分私聊和群聊。Session 的粒度和隔离策略由 IM Adapter 通过 peer_id 的构造方式控制——Adapter 决定什么构成一个"会话对端"，Session 机制本身是通用的。

## 数据流

```
IM Adapter 产出 NormalizedMessage { platform, sender_id, peer_id, thread_id?, account_id?, content, ... }
  → RawLogProcessor：记录原始内容到日志 → 透传
    → SessionRouter：计算 session key = f(platform, sender_id, peer_id[, account_id]) → 查找/创建 session → session_id 写入 metadata
      → MessageCleaner：清洗残留元数据，富文本转 markdown → 更新 content
        → MarkdownNormalizer：标准化 → 更新 content
          → ProcessedMessage { content, metadata }
            → Gateway 路由
```

## 模块关系

- **上游**：IM Adapter（各平台提供自己的适配器，产 NormalizedMessage）
- **下游**：Gateway（接收 ProcessedMessage 并路由）
- **链内**：
  - RawLogProcessor — 日志记录
  - SessionRouter — session 路由，使用 (platform, sender_id, peer_id) 三元组；account_id 在 DmScope 为 PerAccount 时参与；thread_id 不参与 session key 计算（仅用于出站定向）
  - MessageCleaner — 内容清洗
  - MarkdownNormalizer — 格式标准化
- **无关**：出站 Processor 链（独立链路，与入站互不干扰）
