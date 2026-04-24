# Gateway 模块规格说明书

> 本文件按 SPEC_CONVENTION.md v3 标准编写，描述模块的实际行为，以代码为准。

## 1. 模块概述

Gateway 是 IM 平台协议适配器的中央路由层，连接飞书等 IM 平台与内部 Agent 系统。

**子模块**：
- `src/gateway/mod.rs` — Gateway 核心：消息路由、会话管理、配置
- `src/gateway/message.rs` — 空壳，仅含 doc comment
- `src/im/mod.rs` — IM 适配器抽象（IMAdapter trait + AdapterError）
- `src/im/feishu.rs` — 飞书协议实现

**数据流**：外部消息（Feishu webhook）→ `IMAdapter::handle_webhook` → 内部 `Message` → `Gateway::route_message` → `IMAdapter::send_message` → 外部平台。

## 2. 公开接口

### 2.1 构造

| 接口 | 功能 |
|------|------|
| `Gateway::new` | 创建 Gateway 实例 |
| `FeishuAdapter::new` | 创建 Feishu 适配器实例 |

### 2.2 配置

| 接口 | 功能 |
|------|------|
| `Gateway::register_adapter` | 注册 IM 适配器（按 channel 名称） |
| `DmScope` | DM 会话隔离粒度枚举，含四个变体（见 3.4） |

### 2.3 主操作

| 接口 | 功能 |
|------|------|
| `Gateway::route_message` | 将消息路由到对应 channel 的适配器，自动创建会话；若 `account_id` 参数为 `None`，自动从 `message.metadata["account_id"]` 提取；显式参数优先级高于 metadata |
| `IMAdapter::handle_webhook` | 解析外部平台 webhook payload，返回内部 Message |
| `IMAdapter::send_message` | 将内部 Message 发送到外部 IM 平台 |
| `FeishuAdapter::send_card` | 发送飞书交互卡片，返回 message_id |
| `FeishuAdapter::update_message` | 更新已有卡片消息 |

### 2.4 查询

| 接口 | 功能 |
|------|------|
| `Gateway::get_agent_sessions` | 获取 Agent 关联的所有活跃会话 |
| `IMAdapter::name` | 返回平台名称（如 "feishu"） |

### 2.5 清理

| 接口 | 功能 |
|------|------|
| `IMAdapter::validate_signature` | 验证 webhook 请求签名 |

## 3. 类型概览

### 3.1 Message（内部消息格式）

所有 IM 消息转换为统一内部格式：
- `id` — 消息唯一标识
- `from` / `to` — 发送者和接收者 ID
- `content` — 消息文本内容
- `channel` — IM 平台名称
- `timestamp` — 时间戳
- `metadata` — 附加键值对

**metadata 约定**：
- `account_id` — 飞书等平台的 app_id / tenant_id，由 adapter 的 `handle_webhook` 填充，供 `route_message` 自动提取用于 session 隔离

### 3.2 Session

表示一个活跃会话，key 格式由 `dm_scope` 决定：

| DmScope | session key 格式 | 说明 |
|---------|-----------------|------|
| `Main` | `"channel:to"` | 向后兼容，所有飞书用户共享同一 session（见 3.4） |
| `PerPeer` | `"from:to"` | 每个发送方-接收方对独占 session（见 3.4） |
| `PerChannelPeer` | `"channel:from:to"` | 每个 channel + 发送方-接收方对独占 session（见 3.4） |
| `PerAccountChannelPeer` | `"account_id:channel:from:to"` 或 `"default:channel:from:to"` | 多租户支持；无 account_id 时 account 部分为 "default"（见 3.4） |

### 3.2.1 CachedToken

内部结构体，封装 `tenant_access_token` 和过期时间点，提供 `needs_refresh()` 方法。

### 3.2.2 DmScope

DM 会话隔离粒度枚举（kebab-case serde），四个变体：

| 变体 | session key 格式 |
|------|-----------------|
| `Main` | `"channel:to"` |
| `PerPeer` | `"from:to"` |
| `PerChannelPeer` | `"channel:from:to"` |
| `PerAccountChannelPeer` | `"account_id:channel:from:to"`（无 account_id 时用 "default"）|

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

网关错误类型，含：UnknownChannel、MessageTooLarge、AdapterError（From<AdapterError>）、RateLimitExceeded。

## 4. 架构细节

### 4.1 会话管理

`Gateway` 内部维护 `HashMap<String, Session>`（key = session key，格式由 `DmScope` 决定）。`route_message` 在首次收到某 session key 对应的消息时自动创建 Session。

### 4.2 Feishu Token 缓存

Feishu 的 tenant_access_token 有效期约 2 小时，`FeishuAdapter` 在 `Arc<Mutex<Option<CachedToken>>>` 中缓存，**提前 5 分钟主动刷新**（1.5h 后触发）。

### 4.3 FeishuAdapter HTTP Client

单例 `reqwest::Client`，超时 30 秒，所有克隆共享同一个 client 实例。

### 4.4 Token 缓存实现

`CachedToken` 封装 tenant_access_token + 过期时间，`needs_refresh()` 在过期前 5 分钟返回 true。缓存用 `Arc<Mutex<Option<CachedToken>>>` 双层锁，支持多个 FeishuAdapter clone 共享同一缓存节点。

### 4.5 已知行为约束

- `handle_webhook`：仅处理 text 类型消息，非 text（图片/文件等）content 字段被静默置为空字符串
- `send_message`：返回 `Result<()>`，不返回 message_id，无法对文本消息做后续编辑/删除
- `Gateway::route_message` 消息大小校验：默认 `max_message_size = 0`，任何非空消息均触发 `MessageTooLarge`，需业务侧显式配置

## 5. 已知偏差（待 EDA 处理）

以下差异已记录，按 v3 标准不影响理解模块做什么，暂列于此：

| 偏差项 | 类型 | 说明 |
|--------|------|------|
| `rate_limit_per_minute` 未实现 | 说明 | 字段存在但无实际限速逻辑 |
| `GatewayError::RateLimitExceeded` 未使用 | 说明 | 定义但无调用处 |
