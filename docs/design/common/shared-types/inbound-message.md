# NormalizedMessage

## 概述

NormalizedMessage 是平台无关的统一入站消息结构，屏蔽各 IM 平台（飞书、Discord、Telegram 等）和 terminal 渠道的差异。各渠道的 IM Adapter 入站解析产出此结构，Processor Chain 消费。

> **本文档定义的 NormalizedMessage、MediaRef、MessageType 在 common crate 中实现。引用本模块的下游文档通过 [ContentBlock](content-block.md)、[ProcessedMessage](processed-message.md) 等链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### NormalizedMessage

NormalizedMessage 是入站消息的核心载体。各渠道的 IM Adapter 入站解析产出 NormalizedMessage，Processor Chain 消费（读取内容做标准化和 session_key 计算）。Gateway 消费的是 Processor Chain 产出的 ProcessedMessage，不直接接触 NormalizedMessage。

| 字段 | 类型 | 说明 |
|------|------|------|
| `platform` | string | 平台标识，如 `"feishu"`、`"terminal"` |
| `sender_id` | string | 发送者的平台内 ID |
| `peer_id` | string | 会话对端（群聊 chat_id 或私聊对方 ID） |
| `thread_id` | string? | 话题 ID，可选。不参与 session key 计算，仅用于出站定向回复 |
| `account_id` | string | CloseClaw 本地账号标识，由 sender_id 通过身份映射得到。参与 session 路由 |
| `content` | string | 消息文本内容。非文本消息时可为空 |
| `message_type` | enum | 消息类型：text / image / file / audio |
| `media_refs` | list(MediaRef) | 图片/文件/音频的引用列表，每个元素为 MediaRef 结构（含 `key` 资源标识和 `url` 访问地址）。由 Adapter 负责下载到本地临时路径 |
| `timestamp` | int | 消息发送时间（毫秒级 Unix 时间戳） |

**引用/回复消息处理**：IM Adapter 在解析被引用的消息时，将其内容渲染为 markdown blockquote（`> 引用内容`），截断至 500 字符（超出追加 `...`），拼接在 `content` 字段之前。不传递独立的引用消息字段——LLM 在对话文本中直接看到 blockquote。

**消息过滤规则**：text 类型空 content 消息在解析阶段丢弃，不产 NormalizedMessage。非文本消息（image/file/audio）正常产 NormalizedMessage（message_type 标记类型，media_refs 存储引用，content 可为空），由下游 Gateway 统一处理。非文本消息 media_refs 为空列表时，消息仍正常传递——content 和 media_refs 均为空，下游 Gateway 根据 message_type 判断类型后构造错误回复。

**身份映射**：`account_id` 由 IM Adapter 在解析入站消息时填入。与其他字段（platform、sender_id 等直接从消息 payload 提取）不同，account_id 需通过 sender_id 查询账户绑定表获取，非直接取值。映射规则：以 sender_id 为键查询账户绑定表，找到对应的 CloseClaw 账户 ID。一个账户可绑定多个平台的 sender_id。terminal 平台恒为 "owner"，无需查表。详见 [config 模块 accounts.json](../../config/README.md)。

**字段填充职责**：各字段由 IM Adapter 入站解析时填充。Processor Chain 不修改 NormalizedMessage 字段——ContentNormalizer 仅读取 content 做文本标准化，SessionRouter 读取 platform/sender_id/peer_id/account_id 计算 session_key。session_key 写入 ProcessedMessage 的 metadata，不写入 NormalizedMessage。

**message_type 与 media_refs**：message_type 由 ContentNormalizer 消费（非 text 跳过标准化）。media_refs 为多模态支持预留，入站链路不消费。

**建模边界**：NormalizedMessage 建模用户主动发送的消息（文本、图片、文件、音频）。卡片交互事件——用户点击消息中嵌入的按钮、选择器等交互控件——属于工具调用的回执，走 tool_result 通道注入对话，不经过 NormalizedMessage 入站通路。各 IM 平台在 Adapter 解析阶段须区分消息事件和交互事件，仅将消息事件转为 NormalizedMessage。

### MediaRef

MediaRef 是图片/文件/音频的资源引用，由 IM Adapter 下载到本地临时路径后填充。

| 字段 | 类型 | 说明 |
|------|------|------|
| `key` | string | 资源标识，平台内的唯一 key |
| `url` | string | 资源访问地址，Adapter 据此下载到本地临时路径 |

### MessageType

MessageType 是消息类型的枚举。

| 值 | 说明 |
|----|------|
| `text` | 纯文本消息 |
| `image` | 图片消息 |
| `file` | 文件消息 |
| `audio` | 音频消息 |

## 数据流

NormalizedMessage 的全系统流动路径：

```
IM 平台 webhook / terminal stdin
  ↓
IM Adapter 入站解析（各平台插件）
  → 平台格式转 NormalizedMessage { platform, sender_id, peer_id, thread_id?, account_id, content, message_type, media_refs, timestamp }
  ↓
Processor Chain 入站
  → RawLog（记录日志）→ SessionRouter（计算 session_key）→ ContentNormalizer（文本标准化）
  → 产出 ProcessedMessage
  ↓
Gateway 路由
  → SessionManager 查找/创建 session → LLM 对话 / SlashDispatcher
```

NormalizedMessage 仅用于入站方向。出站方向使用 ContentBlock[]（LLM 输出）和 [ProcessedMessage](processed-message.md)（经 Processor Chain 处理后的中间结构），与 NormalizedMessage 无关。

## 模块关系

- **生产者**：IM Adapter 各平台插件（入站解析）——包括飞书、Discord、Telegram 等 IM 平台的 Adapter，以及 CLI 模块的 TerminalAdapter
- **消费者**：Processor Chain 入站（读取 NormalizedMessage 做内容标准化和 session_key 计算，产出 [ProcessedMessage](processed-message.md)）
- **无关**：LLM Provider（不接触 NormalizedMessage，只消费 ContentBlock[]）、Session（通过 Gateway 间接消费路由字段，不直接接触 NormalizedMessage）、Slash Command（斜杠指令不涉及 NormalizedMessage 结构）
