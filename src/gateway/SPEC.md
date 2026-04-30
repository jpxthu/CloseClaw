# Gateway 模块规格说明书

> 本文件按 SPEC_CONVENTION.md v3 标准编写，描述模块的实际行为，以代码为准。

## 1. 模块概述

Gateway 是 IM 平台协议适配器的中央路由层，连接飞书等 IM 平台与内部 Agent 系统。

**子模块**：
- `src/gateway/mod.rs` — Gateway 核心：消息路由、配置
- `src/gateway/message.rs` — 空壳，仅含 doc comment
- `src/gateway/session_manager.rs` — SessionManager：会话生命周期管理（查找、创建、恢复）
- `src/im/mod.rs` — IM 适配器抽象（IMAdapter trait + AdapterError）
- `src/im/feishu.rs` — 飞书协议实现

**数据流**：外部消息（Feishu webhook）→ `IMAdapter::handle_webhook` → 内部 `Message` → `Gateway::route_message` → `IMAdapter::send_message` → 外部平台。

## 2. 公开接口

### 2.1 构造

| 接口 | 功能 |
|------|------|
| `Gateway::new` | 创建 Gateway 实例 |
| `SessionManager::new` | 创建 SessionManager 实例 |
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
| `SessionManager::find_or_create` | 按 channel + message + account_id 查找或创建 session，返回 session_id 字符串 |
| `IMAdapter::handle_webhook` | 解析外部平台 webhook payload，返回内部 Message |
| `IMAdapter::send_message` | 将内部 Message 发送到外部 IM 平台 |
| `FeishuAdapter::send_card` | 发送飞书交互卡片，返回 message_id |
| `FeishuAdapter::update_message` | 更新已有卡片消息 |

### 2.4 查询

| 接口 | 功能 |
|------|------|
| `SessionManager::get_agent_sessions` | 获取 Agent 关联的所有活跃会话 |
| `SessionManager::has_session` | 检查给定 session_id 的会话是否存在于活跃表中 |
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

### 3.4 IMAdapter Trait

协议适配器需实现四个方法：`name`（平台标识）、`handle_webhook`（解析入站消息）、`send_message`（发送出站消息）、`validate_signature`（验签）。

### 3.5 AdapterError

适配器错误类型，含：InvalidPayload、AuthFailed、SendFailed、InvalidSignature、IoError。

### 3.6 GatewayError

网关错误类型，含：UnknownChannel、MessageTooLarge、AdapterError（From<AdapterError>）、RateLimitExceeded、**MissingSessionId**（route_message 从 metadata 读取 session_id 失败）。

### 3.7 SessionManager

从 Gateway 提取的独立会话管理组件，负责会话的全生命周期：查找、创建、归档恢复。

## 4. 架构细节

### 4.1 会话管理分工

Gateway 不再直接持有 sessions/storage/dm_scope，改为持有 `session_manager: Arc<SessionManager>`。SessionManager 负责：
- `find_or_create`：查找或创建 session（计算 key → 查活跃表 → 尝试归档恢复 → 创建新 session）
- `try_restore_archived_session`：从存储恢复 Archived checkpoint
- `get_agent_sessions` / `has_session`：查询接口

### 4.2 Gateway::route_message 简化

`route_message` 从 metadata 读取 `session_id`（不再计算 session key），验证存在于活跃表，然后转发到适配器。流程：读 session_id → MissingSessionId 则报错 → 验证 has_session → 调用 adapter.send_message。

### 4.3 Archived Session 恢复

当 `storage` 被配置后，`SessionManager::find_or_create` 在创建新 Session 前会调用 `try_restore_archived_session` 检查存储中是否存在该 session_id 的 Archived checkpoint：若 status 为 Archived，则通过对应 channel 的 adapter 发送 "正在恢复会话..." 通知，调用 `storage.restore_checkpoint` 恢复 session，恢复后重新加载 checkpoint，并用 checkpoint 的 `chat_id` 填充新 Session 的 `agent_id` 字段。通知发送失败仅 warn，不阻塞消息路由。

### 4.4 Feishu Token 缓存

Feishu 的 tenant_access_token 有效期约 2 小时，`FeishuAdapter` 在 `Arc<Mutex<Option<CachedToken>>>` 中缓存，**提前 5 分钟主动刷新**（1.5h 后触发）。

### 4.5 FeishuAdapter HTTP Client

单例 `reqwest::Client`，超时 30 秒，所有克隆共享同一个 client 实例。

### 4.6 已知行为约束

- `handle_webhook`：仅处理 text 类型消息，非 text（图片/文件等）content 字段被静默置为空字符串
- `send_message`：返回 `Result<()>`，不返回 message_id，无法对文本消息做后续编辑/删除
- `Gateway::route_message` 消息大小校验：默认 `max_message_size = 0`，任何非空消息均触发 `MessageTooLarge`，需业务侧显式配置

## 5. 已知偏差（待 EDA 处理）

| 偏差项 | 类型 | 说明 |
|--------|------|------|
| `rate_limit_per_minute` 未实现 | 说明 | 字段存在但无实际限速逻辑 |
| `GatewayError::RateLimitExceeded` 未使用 | 说明 | 定义但无调用处 |
