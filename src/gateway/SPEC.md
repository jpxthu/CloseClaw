# Gateway 模块规格说明书

> 本文件按 SPEC_CONVENTION.md v3 标准编写，描述模块的实际行为，以代码为准。

## 1. 模块概述

Gateway 是 IM 平台协议适配器的中央路由层，连接飞书等 IM 平台与内部 Agent 系统。

**子模块**：
- `src/gateway/mod.rs` — Gateway 核心：消息路由、配置
- `src/gateway/message.rs` — 空壳，仅含 doc comment
- `src/gateway/session_manager.rs` — SessionManager：会话生命周期管理（查找、创建、恢复）
- `src/gateway/session_handler.rs` — SessionMessageHandler：Gateway 层 LLM 会话管理器（busy/pending 状态机）
- `src/im/mod.rs` — IM 适配器抽象（IMAdapter trait + AdapterError）
- `src/im/feishu.rs` — 飞书协议实现

**数据流**：外部消息（Feishu webhook）→ `IMAdapter::handle_webhook` → 内部 `Message` → `Gateway::route_message` → `IMAdapter::send_message` → 外部平台。

## 2. 公开接口

### 2.1 构造

| 接口 | 功能 |
|------|------|
| `Gateway::new` | 创建 Gateway 实例（无 processor_registry 和 renderer） |
| `Gateway::with_processor_registry` | 创建 Gateway 实例并注入 `processor_registry` |
| `Gateway::with_renderer` | 创建 Gateway 实例并注入 renderer 和可选 processor_registry |
| `Gateway::with_checkpoint_manager` | 创建 Gateway 实例并注入 CheckpointManager，使 Gateway 具备出站消息 checkpoint 持久化能力（`send_outbound` 成功发送后自动保存 `PendingMessage`） |
| `Gateway::with_session_handler` | 创建 Gateway 实例并注入 SessionMessageHandler，使 Gateway 具备 busy/pending LLM 会话管理能力 |
| `SessionManager::new` | 创建 SessionManager 实例（接收 config、storage、workspace_dir、bootstrap_mode） |
| `SessionMessageHandler::new` | 创建带 output channel 的 SessionMessageHandler（LLM 响应文本发送到 channel） |
| `SessionMessageHandler::new_no_output` | 创建无 output channel 的 SessionMessageHandler（LLM 响应丢弃） |
| `FeishuAdapter::new` | 创建 Feishu 适配器实例 |

### 2.2 配置

| 接口 | 功能 |
|------|------|
| `Gateway::set_storage` | 配置持久化存储后端（代理到 SessionManager），使 Gateway 具备 archived session 恢复能力 |
| `Gateway::register_adapter` | 注册 IM 适配器（按 channel 名称）；同时代理到 SessionManager 注册 |
| `SessionManager::register_adapter` | 注册 IM 适配器到 SessionManager，用于恢复时发送通知 |
| `DmScope` | DM 会话隔离粒度枚举，含四个变体（见 3.4） |

### 2.3 主操作

| 接口 | 功能 |
|------|------|
| `Gateway::route_message` | 从 `message.metadata["session_id"]` 读取 session_id，验证存在于活跃表中，通过对应 channel 适配器发送消息 |
| `Gateway::send_outbound` | 将 agent 原始输出经 processor chain 处理后按 `msg_type` 分发到适配器（text→send_message，interactive→send_card_json）；每个发送成功路径在 `checkpoint_manager` 配置时自动调用 `persist_outbound_checkpoint` 加载/创建 checkpoint、添加 `PendingMessage`（`sent` 初始 `false` → 立即 `mark_sent` 变为 `true`）并保存 |
| `Gateway::flush_all_sessions` | 将所有活跃 session 的 checkpoint 同步持久化到存储后端，供 Daemon shutdown 时调用 |
| `Gateway::handle_inbound_message` | 通过 SessionMessageHandler 处理入站消息，返回 `Option<HandleResult>`（`LlmStarted`/`MessageQueued`），未配置 handler 时返回 `None` |
| `SessionManager::flush_all` | 遍历所有活跃 session，将 checkpoint 同步写入持久化后端；写入前从 `ConversationSession` 读取 pending_messages 填入 checkpoint；单个失败只 warn 不中断；storage 未配置时返回 Ok(0) |
| `SessionManager::find_or_create` | 按 channel + message + account_id 查找或创建 session，返回 session_id 字符串；创建 session 时同步创建 `ConversationSession` |
| `SessionManager::is_session_busy` | 检查给定 session_id 的 LLM busy 状态，不存在的 session 返回 false |
| `SessionManager::push_pending_message` | 将 `PendingMessage` 入队到指定 session 的 pending 队列 |
| `SessionManager::pop_pending_message` | 从指定 session 的 pending 队列弹出最早的消息，不存在或队列为空时返回 None |
| `IMAdapter::handle_webhook` | 解析外部平台 webhook payload，返回内部 Message |
| `IMAdapter::send_message` | 将内部 Message 发送到外部 IM 平台 |
| `IMAdapter::send_card_json` | 将 pre-serialized JSON 作为 interactive card 发送到外部 IM 平台 |
| `FeishuAdapter::send_card` | 发送飞书交互卡片，返回 message_id |
| `FeishuAdapter::update_message` | 更新已有卡片消息 |
| `SessionMessageHandler::handle_message` | 处理入站消息：idle → 设置 busy + 启动 LLM 调用；busy → 入队 pending；LLM 完成后自动 drain pending |

### 2.4 查询

| 接口 | 功能 |
|------|------|
| `SessionManager::get_agent_sessions` | 获取 Agent 关联的所有活跃会话 |
| `SessionManager::get_chat_id` | 根据 session_id 查询对应 chat_id |
| `SessionManager::has_session` | 检查给定 session_id 的会话是否存在于活跃表中 |
| `SessionManager::get_conversation_session` | 获取给定 session_id 的 `ConversationSession`，不存在时返回 None |
| `IMAdapter::name` | 返回平台名称（如 "feishu"） |

### 2.5 清理

| 接口 | 功能 |
|------|------|
| `IMAdapter::validate_signature` | 验证 webhook 请求签名 |

## 3. 类型概览

### 3.1 metadata 约定

SessionRouter 写入 metadata 的字段：
- `account_id` — 飞书等平台的 app_id / tenant_id，用于多租户 session 隔离
- `session_id` — 由 SessionRouter 解析后写入，供 route_message 验证和使用
- `from` — 发送者 open_id
- `to` — 接收者 chat_id
- `channel` — IM 平台名称

### 3.2 Session & DmScope

Session 的 key 格式由 `DmScope` 决定。`DmScope` 为 kebab-case serde 枚举，含四个变体：

| DmScope | session key 格式 |
|---------|-----------------|
| `Main` | `"channel:to"` |
| `PerPeer` | `"from:to"` |
| `PerChannelPeer` | `"channel:from:to"` |
| `PerAccountChannelPeer` | `"account_id:channel:from:to"`（无 account_id 时为 "default"）|

### 3.3 GatewayConfig

- `name` — 实例名称
- `rate_limit_per_minute` — 限速配置（字段存在，当前无实际限速逻辑）
- `max_message_size` — 消息大小上限
- `dm_scope` — DM 会话隔离粒度，控制 session key 的分区方式，默认 `PerChannelPeer`

### 3.4 processor_registry 与 renderer 集成

`Gateway` 持有 `processor_registry: Option<Arc<ProcessorRegistry>>` 和 `renderer: Option<Arc<dyn Renderer>>`（分别由 `with_processor_registry` / `with_renderer` 注入）。

**入站消息**：在 `route_message` 入口处经过 processor 链处理：

1. 若 `processor_registry` 为 `None` 或 `inbound` 链为空 → bypass，Gateway 行为与原来完全一致
2. 若 `processor_registry` 存在且 `inbound` 非空 → 将 `Message` 转为 `RawMessage`，调用 `ProcessorRegistry::process_inbound()`，将返回的 `ProcessedMessage.metadata` 合并回原 `Message.metadata`

**出站消息**：`send_outbound` 按以下优先级分发：

1. **`renderer` 路径**（优先）：当 `renderer` 存在时，调用 processor 链处理，从 `processed.metadata["dsl_result"]` 反序列化 `DslParseResult`，调用 `renderer.render(clean_content, dsl_result)`，根据 `RenderedOutput.msg_type` 分发（`"text"` → `send_message`，`"interactive"` → `send_card_json`）。此时 processor_registry 必须存在。
2. **`processor_registry` 路径**（次优先）：当无 renderer 但有 `processor_registry` 时，调用 `process_outbound`，解析 `msg_type` JSON 分发。
3. **bypass 路径**：两者均不存在时，直接将 `raw_output` 作为纯文本发出。

以上所有路径中：若 `suppress == true` → 不发送任何消息，直接返回 `Ok`。

**MarkdownToCard 向后兼容**：`ProcessorConfig::MarkdownToCard` 仍可反序列化，但在 `build_processor` 中被跳过（不注册到 registry）。

此设计保证：默认（未配置 `processor_chain` 和 renderer）Gateway 行为不受影响。

### 3.5 IMAdapter Trait

协议适配器需实现四个方法：`name`（平台标识）、`handle_webhook`（解析入站消息）、`send_message`（发送出站消息）、`validate_signature`（验签）。

### 3.6 AdapterError

适配器错误类型，含：InvalidPayload、AuthFailed、SendFailed、InvalidSignature、IoError。

### 3.7 GatewayError

网关错误类型，含：UnknownChannel、MessageTooLarge、AdapterError（From<AdapterError>）、RateLimitExceeded、**MissingSessionId**（route_message/send_outbound 从 metadata/session_id 读取失败）、**OutboundError**（出站 processor 处理失败或未知 msg_type）。

### 3.8 SessionManager

从 Gateway 提取的独立会话管理组件，负责会话的全生命周期：查找、创建、恢复、flush。Daemon shutdown 时 `flush_all()` 将所有活跃 session 的 checkpoint 同步持久化到存储后端。

`SessionManager` 还持有 `workspace_dir: Option<PathBuf>` 和 `bootstrap_mode: BootstrapMode`，以及 `conversation_sessions: RwLock<HashMap<String, Arc<RwLock<ConversationSession>>>>`。`find_or_create` 创建 session 时，若 `workspace_dir` 存在，则通过 `load_bootstrap_files` 加载 bootstrap 文件集合并作为 system prompt 注入到 `ConversationSession`；否则创建无 system prompt 的 `ConversationSession`。通过 `get_conversation_session`、`is_session_busy`、`push_pending_message`、`pop_pending_message` 方法提供 busy/pending 管理。

### 3.9 CheckpointManager 集成

`Gateway` 持有 `checkpoint_manager: Option<Arc<CheckpointManager<dyn PersistenceService>>>`（由 `with_checkpoint_manager` 注入）。

**出站消息 checkpoint 持久化**：`send_outbound` 每次成功发送后（text/interactive/plain text fallback 三条路径），若 `checkpoint_manager` 已配置，则调用私有方法 `persist_outbound_checkpoint` 写入 checkpoint：

1. 加载或创建 `SessionCheckpoint`
2. 调用 `PendingMessage::new(message_id, content)` 创建待确认消息（`sent=false`）
3. 调用 `pending.mark_sent()` 将 `sent` 置为 `true`
4. 调用 `checkpoint.add_pending_message(pending)` 添加记录
5. 调用 `checkpoint.touch()` 更新 `last_message_at`
6. 调用 `cm.save(cp)` 持久化

`checkpoint_manager` 为 `None` 时行为不变（向后兼容）。

### 3.10 SessionMessageHandler

Gateway 层的 LLM 会话管理器，实现 busy/pending 状态机。持有 `Arc<SessionManager>` 和 `Arc<FallbackClient>`，通过 `handle_message(session_id, content)` 处理入站消息：

- **idle 状态**：设置 busy，spawn 异步任务调用 `FallbackClient::chat()`（非流式），LLM 调用完成（成功或失败）后清除 busy 并 drain pending 队列
- **busy 状态**：消息入队 pending queue，返回 `MessageQueued`
- **LLM 完成后**：自动 drain pending 队列（loop 方式），按 FIFO 顺序消费

提供两个构造方法：`new`（带 output channel，LLM 响应文本发送到 channel）和 `new_no_output`（无 output channel，响应丢弃）。

### 3.11 SessionMessageHandler 集成

`Gateway` 持有 `session_handler: Option<Arc<SessionMessageHandler>>`（通过 `with_session_handler()` 注入）。`handle_inbound_message(session_id, content)` 委托给 handler，返回 `Option<HandleResult>`（`LlmStarted` 或 `MessageQueued`）。未配置 handler 时 Gateway 行为不变（向后兼容）。

## 4. 架构细节

### 4.1 会话管理分工

Gateway 不再直接持有 sessions/storage/dm_scope，改为持有 `session_manager: Arc<SessionManager>`。SessionManager 负责：
- `find_or_create`：查找或创建 session（计算 key → 查活跃表 → 尝试归档恢复 → 创建新 session）
- `try_restore_archived_session`：从存储恢复 Archived checkpoint
- `get_agent_sessions` / `has_session`：查询接口

### 4.2 Gateway::route_message 简化

`route_message` 从 metadata 读取 `session_id`（不再计算 session key），验证存在于活跃表，然后转发到适配器。流程：读 session_id → MissingSessionId 则报错 → 验证 has_session → 调用 adapter.send_message。

### 4.3 Archived Session 恢复

当 `storage` 被配置后，`SessionManager::find_or_create` 在创建新 Session 前会调用 `try_restore_archived_session` 检查存储中是否存在该 session_id 的 Archived checkpoint：若 status 为 Archived，则通过对应 channel 的 adapter 发送 "正在恢复会话..." 通知，调用 `storage.restore_checkpoint` 恢复 session，恢复后重新加载 checkpoint，并用 checkpoint 的 `chat_id` 填充新 Session 的 `agent_id` 字段。恢复时同时将 checkpoint 中 `sent=false` 的 pending_messages 重新注入 `ConversationSession` 队列（`sent=true` 的跳过）。通知发送失败仅 warn，不阻塞消息路由。

### 4.4 busy/pending 状态机

`SessionMessageHandler` 实现 Gateway 层的 LLM 并发控制：

- **idle → LLM 调用**：收到消息时 `is_session_busy == false`，设置 `llm_busy = true`，spawn 异步任务调用 `FallbackClient::chat()`
- **busy → 入队**：收到消息时 `is_session_busy == true`，将消息 `push_pending_message` 入队
- **LLM 结束 → 清 busy + drain**：异步任务完成后 `set_llm_busy(false)`，然后 `pop_pending` 直到队列空，每次 pop 都会重新走 idle → LLM 调用流程

### 4.5 Feishu Token 缓存

Feishu 的 tenant_access_token 有效期约 2 小时，`FeishuAdapter` 在 `Arc<Mutex<Option<CachedToken>>>` 中缓存，**提前 5 分钟主动刷新**（1.5h 后触发）。

### 4.6 FeishuAdapter HTTP Client

单例 `reqwest::Client`，超时 30 秒，所有克隆共享同一个 client 实例。

### 4.7 已知行为约束

- `handle_webhook`：仅处理 text 类型消息，非 text（图片/文件等）content 字段被静默置为空字符串
- `send_message`：返回 `Result<()>`，不返回 message_id，无法对文本消息做后续编辑/删除
- `Gateway::route_message` 消息大小校验：默认 `max_message_size = 0`，任何非空消息均触发 `MessageTooLarge`，需业务侧显式配置

## 5. 已知偏差（待 EDA 处理）

| 偏差项 | 类型 | 说明 |
|--------|------|------|
| `rate_limit_per_minute` 未实现 | 说明 | 字段存在但无实际限速逻辑 |
| `GatewayError::RateLimitExceeded` 未使用 | 说明 | 定义但无调用处 |
