# 共享类型

## 概述

共享类型是跨模块传递的纯数据结构，被 2 个及以上模块共同消费。每个共享类型在本文档中唯一定义，各业务模块文档通过引用指向此处，不在自身文档中重复描述字段结构。

本文档不包含 trait 接口定义——核心 trait 见 [core-traits](core-traits.md)。

## 架构

### NormalizedMessage

NormalizedMessage 是平台无关的统一入站消息结构，屏蔽各 IM 平台（飞书、Discord、Telegram 等）和 terminal 渠道的差异。各渠道的 IM Adapter 入站解析产出此结构，Processor Chain 和 Gateway 消费。

| 字段 | 类型 | 说明 |
|------|------|------|
| `platform` | string | 平台标识，如 `"feishu"`、`"terminal"` |
| `sender_id` | string | 发送者的平台内 ID |
| `peer_id` | string | 会话对端（群聊 chat_id 或私聊对方 ID） |
| `thread_id` | string? | 话题 ID，可选。不参与 session key 计算，仅用于出站定向回复 |
| `account_id` | string | CloseClaw 本地账号标识，由 sender_id 通过身份映射得到。参与 session 路由 |
| `content` | string | 消息文本内容。非文本消息时可为空 |
| `message_type` | enum | 消息类型：text / image / file / audio |
| `media_refs` | list | 图片/文件/音频的引用列表（key + URL）。由 Adapter 负责下载到本地临时路径 |
| `quoted_message` | string? | 被引用的消息内容，可选。最多嵌套一层 |
| `timestamp` | int | 消息发送时间（毫秒级 Unix 时间戳） |

**消息过滤规则**：text 类型空 content 消息在解析阶段丢弃，不产 NormalizedMessage。非文本消息（image/file/audio）正常产 NormalizedMessage（message_type 标记类型，media_refs 存储引用，content 可为空），由下游 Gateway 统一处理。

**身份映射**：`account_id` 由 IM 插件在解析入站消息时填入。映射规则：以 sender_id 为键查询账户绑定表，找到对应的 CloseClaw 账户 ID。一个账户可绑定多个平台的 sender_id。terminal 平台恒为 "owner"，无需查表。详见 [config 模块 accounts.json](../config/README.md)。

**字段填充职责**：各字段由 IM Adapter 入站解析时填充。Processor Chain 不修改 NormalizedMessage 字段——仅读取 content 做文本标准化并产出 ProcessedMessage（含标准化后 content + metadata），session_key 写入 ProcessedMessage.metadata，不写入 NormalizedMessage。

## 数据流

NormalizedMessage 的全系统流动路径：

```
IM 平台 webhook / terminal stdin
  ↓
IM Adapter 入站解析（各平台插件）
  → 平台格式转 NormalizedMessage { platform, sender_id, peer_id, thread_id?, account_id, content, message_type, media_refs, quoted_message, timestamp }
  ↓
Processor Chain 入站
  → RawLog（记录日志）→ SessionRouter（计算 session_key）→ ContentNormalizer（文本标准化）
  → 产出 ProcessedMessage
  ↓
Gateway 路由
  → SessionManager 查找/创建 session → LLM 对话 / SlashDispatcher
```

NormalizedMessage 仅用于入站方向。出站方向使用 ContentBlock[]（LLM 输出）和 ProcessedMessage（经 Processor Chain 处理后的中间结构），与 NormalizedMessage 无关。

详见 [common 数据流](data-flow.md)。

## 模块关系

- **生产者**：IM Adapter 各平台插件（入站解析）——包括飞书、Discord、Telegram 等 IM 平台的 Adapter，以及 CLI 模块的 TerminalAdapter
- **消费者**：Processor Chain 入站（读取 NormalizedMessage 做内容标准化和 session_key 计算，产出 ProcessedMessage）、Gateway（消费 ProcessedMessage 做路由决策）
- **无关**：LLM Provider（不接触 NormalizedMessage，只消费 ContentBlock[]）、Session（通过 Gateway 间接消费路由字段，不直接接触 NormalizedMessage）、Slash Command（斜杠指令不涉及 NormalizedMessage 结构）
