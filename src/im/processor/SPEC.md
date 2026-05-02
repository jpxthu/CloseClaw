# processor 子模块规格说明书

> 本文件按 SPEC_CONVENTION.md v3 标准编写，描述模块的实际行为，以代码为准。

## 1. 模块概述

`processor` 子模块提供可扩展的 Processor 链式架构，分 phase（Inbound/Outbound）按 priority 排序执行。Inbound 处理器清洗飞书 webhook 事件，Outbound 处理器解析 LLM 输出中的 DSL 指令。

**文件**：
- `src/im/processor/mod.rs` — ProcessPhase、MessageProcessor trait、MessageContext、ProcessError、ProcessorRegistry、ProcessedMessage
- `src/im/processor/cleaner.rs` — FeishuMessageCleaner（inbound）
- `src/im/processor/dsl_parser.rs` — DslParser（outbound）
- `src/im/processor/session_router.rs` — SessionRouter（inbound，priority 20）

## 2. 公开接口

### 2.1 ProcessorRegistry

| 接口 | 功能 |
|------|------|
| `ProcessorRegistry::new` | 创建注册表，自动注册 SessionRouter（inbound, prio 20）、FeishuMessageCleaner（inbound, prio 30） |
| `ProcessorRegistry::register` | 按 processor 的 phase 和 priority 注册，手动调用以注册其他 processor（如 DslParser） |
| `ProcessorRegistry::process_inbound` | 执行 Inbound 链，按 priority 升序调用各 processor |
| `ProcessorRegistry::process_outbound` | 执行 Outbound 链，按 priority 升序调用各 processor |

### 2.2 SessionRouter

| 接口 | 功能 |
|------|------|
| `SessionRouter::new` | 创建 SessionRouter，持有 `Arc<SessionManager>` |
| `MessageProcessor::priority` | 返回 20（runs before FeishuMessageCleaner priority 30） |
| `MessageProcessor::phase` | 返回 Inbound |
| `MessageProcessor::process` | 从 raw webhook 提取 feishu 字段，调用 SessionManager.find_or_create，将 session_id 等字段写入 metadata |

### 2.3 FeishuMessageCleaner

| 接口 | 功能 |
|------|------|
| `FeishuMessageCleaner` | Inbound Processor，priority=30；清洗飞书 webhook JSON 为纯文本 |
| `clean_feishu_message` | 兼容旧调用方的入口，内部委托给 FeishuMessageCleaner |

### 2.4 DslParser

| 接口 | 功能 |
|------|------|
| `DslParser` | Outbound Processor，priority=10；解析 `::button[...]` DSL 指令 |
| `DslParser::parse` | 纯函数，将 markdown 内容解析为 DslParseResult |

### 2.5 类型

| 类型 | 说明 |
|------|------|
| `ProcessPhase` | 枚举：Inbound / Outbound |
| `ProcessError` | 处理器错误，含：MissingMessage、UnsupportedMessageType、ProcessingFailed、RegistryError、JsonError、SessionNotSupportedForChannel |
| `ProcessedMessage { content, metadata }` | 处理结果；content 为清洗后文本/markdown，metadata 由各 Processor 累积 |
| `MessageContext { metadata }` | Processor 链中传递的上下文 |
| `DslParseResult { clean_content, instructions }` | DSL 解析结果 |
| `DslInstruction::Button { label, action, value }` | DSL 按钮指令 |

## 3. SessionRouter 架构

### 3.1 职责

SessionRouter 是 **Inbound** Processor（priority=20），在 FeishuMessageCleaner（priority=30）之前运行。负责：
1. 从飞书 webhook JSON 原始数据中提取 `account_id`（tenant_key/app_id）、`from`（sender.open_id）、`to`（message.chat_id）、`channel`
2. 调用 `SessionManager::find_or_create` 解析或创建 session
3. 将 `session_id`、`account_id`、`from`、`to`、`channel` 写入 `ProcessedMessage.metadata`
4. 群聊（chat_type=group）直接返回 `ProcessError::SessionNotSupportedForChannel`

### 3.2 数据流

```
Raw Feishu webhook JSON
    │
    ▼
SessionRouter (priority=20, phase=Inbound)
    ├─ 检测群聊 → SessionNotSupportedForChannel 错误
    ├─ 提取 sender.open_id → from
    ├─ 提取 message.chat_id → to
    ├─ 提取 channel / tenant_key / app_id
    ├─ 调用 SessionManager.find_or_create(channel, Message, account_id)
    └─ 写入 metadata: session_id, account_id, from, to, channel
    │
    ▼
ProcessedMessage { content: 原始content, metadata: {...} }
    │
    ▼
FeishuMessageCleaner (priority=30, phase=Inbound)
    └─ 清洗 content 为纯文本
```

### 3.3 metadata 字段约定

SessionRouter 写入 ProcessedMessage.metadata 的字段：

| 字段 | 来源 | 说明 |
|------|------|------|
| `session_id` | SessionManager.find_or_create 返回值 | session 唯一标识 |
| `account_id` | webhook.tenant_key 或 webhook.app_id | 多租户标识，无时默认为 "default" |
| `from` | webhook.sender.sender_id.open_id | 发送者 open_id |
| `to` | webhook.message.chat_id | 接收者 chat_id |
| `channel` | webhook.channel 或固定 "feishu" | IM 平台名称 |

SessionRouter **保留**上游已有的所有 metadata 字段（MessageContext.metadata），仅做追加。

### 3.4 SessionManager.find_or_create 行为

1. 计算 session_id（格式由 DmScope 决定）
2. 查活跃表 → 已存在则直接返回
3. 不存在则尝试从 storage 恢复 Archived checkpoint
4. 恢复成功 → 用 checkpoint.chat_id 构建 Session 并注册
5. 恢复失败 → 创建新 Session，以 message.to 为 agent_id

### 3.5 ProcessError 新增变体

`ProcessError::SessionNotSupportedForChannel(String)` — channel 名称。群聊时由 SessionRouter 返回。
