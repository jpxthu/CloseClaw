# Gateway 模块规格说明书

> 本文件描述 Gateway 模块的当前实际行为，以代码为准。

## 1. 模块概述

**职责**：Gateway 是 IM 平台协议适配器的中央路由层，连接飞书、Discord 等 IM 平台与内部 Agent 系统。

**位置**：`src/gateway/` + `src/im/`

**子模块**：
- `src/gateway/mod.rs` — Gateway 核心（路由、会话管理）
- `src/gateway/message.rs` — Gateway 消息类型（空壳，仅含 doc）
- `src/im/mod.rs` — IM 适配器抽象（IMAdapter trait）
- `src/im/feishu.rs` — 飞书协议实现

## 2. 核心类型

### 2.1 Gateway

```rust
pub struct Gateway {
    config: GatewayConfig,
    adapters: RwLock<HashMap<String, Arc<dyn IMAdapter>>>,
    sessions: RwLock<HashMap<String, Session>>,
}
```

**公开方法**：

| 方法 | 签名 | 说明 |
|------|------|------|
| `new` | `fn new(config: GatewayConfig) -> Self` | 创建 Gateway 实例 |
| `register_adapter` | `async fn register_adapter(&self, name: String, adapter: Arc<dyn IMAdapter>)` | 注册 IM 适配器 |
| `route_message` | `async fn route_message(&self, channel: &str, message: Message) -> Result<(), GatewayError>` | 路由消息到对应适配器，含消息大小校验和自动建会话 |
| `get_agent_sessions` | `async fn get_agent_sessions(&self, agent_id: &str) -> Vec<Session>` | 获取 Agent 关联的所有活跃会话 |

### 2.2 Session

```rust
pub struct Session {
    pub id: String,        // 格式："channel:to"
    pub agent_id: String,
    pub channel: String,
    pub created_at: i64,
}
```

### 2.3 Message（内部消息格式）

```rust
pub struct Message {
    pub id: String,
    pub from: String,         // 发送者 ID
    pub to: String,           // 接收者 ID（webhook 入方向为 String::new()，由 Gateway 填充）
    pub content: String,
    pub channel: String,
    pub timestamp: i64,
    pub metadata: HashMap<String, String>,
}
```

### 2.4 GatewayConfig

```rust
pub struct GatewayConfig {
    pub name: String,
    pub rate_limit_per_minute: u32,  // 字段存在，当前未实现实际限速
    pub max_message_size: usize,
}
```

## 3. IMAdapter Trait

**位置**：`src/im/mod.rs`

```rust
#[async_trait]
pub trait IMAdapter: Send + Sync {
    fn name(&self) -> &str;
    async fn handle_webhook(&self, payload: &[u8]) -> Result<Message, AdapterError>;
    async fn send_message(&self, message: &Message) -> Result<(), AdapterError>;
    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool;
}
```

### 3.1 AdapterError

```rust
pub enum AdapterError {
    InvalidPayload(String),
    AuthFailed,
    SendFailed(String),
    InvalidSignature,
    IoError(std::io::Error),
}
```

## 4. FeishuAdapter

**位置**：`src/im/feishu.rs`

**构造**：`FeishuAdapter::new(app_id, app_secret, verification_token) -> Self`

**实现 IMAdapter 全部方法**：
- `name()` → `"feishu"`
- `handle_webhook(payload)` — 解析 Feishu 事件 payload，返回内部 `Message`
- `send_message(message)` — 通过 Feishu API 发送文本消息
- `validate_signature(sig, payload)` — SHA256 验签：`SHA256(verification_token + payload) == sig`

**额外公开方法**（非 IMAdapter trait）：

| 方法 | 说明 |
|------|------|
| `send_card(chat_id, card)` | 发送 Feishu 交互卡片，返回 message_id |
| `update_message(message_id, patch)` | 更新已有卡片消息（需提供完整 patch JSON） |

**内部机制**：
- **Tenant Token 缓存**：缓存在 `Arc<Mutex<Option<CachedToken>>>` 中，TTL 2小时，**提前 5 分钟主动刷新**（1.5h 后刷新）
- **HTTP Client**：单例 Client，超时 30 秒

## 5. 错误类型

```rust
pub enum GatewayError {
    UnknownChannel(String),       // 未知 channel，无适配器注册
    MessageTooLarge,              // 消息超过 max_message_size
    AdapterError(String),         // 适配器错误（From<AdapterError>）
    RateLimitExceeded,            // 限速（定义但未实现）
}
```

## 6. 偏差记录

> 代码与 docs/gateway/README.md 不一致处，以代码为准。

| 偏差项 | 类型 | 说明 |
|--------|------|------|
| `IMAdapter` 定义位置 | 冲突 | 文档写 gateway 模块下，实际在 `src/im/mod.rs` |
| `FeishuAdapter` 定义位置 | 冲突 | 文档写 gateway 下，实际在 `src/im/feishu.rs` |
| `Gateway::register_adapter` | 少了 | 文档无此方法，代码有 |
| `Gateway::get_agent_sessions` | 多了 | 文档无此方法，代码有 |
| `GatewayError::RateLimitExceeded` | 多了 | 文档无此变体，代码有 |
| `From<AdapterError> for GatewayError` impl | 多了 | 文档无，代码有 |
| Session 表结构（`HashMap<"channel:to"→Session>`） | 多了 | 文档无，代码有完整实现 |
| `CachedToken` 主动刷新策略（1.5h） | 多了 | 文档无，代码有 |
| `FeishuAdapter::send_card` / `update_message` | 多了 | 文档无，代码有公开方法 |
| `rate_limit_per_minute` 字段 | 说明 | 字段存在但无实际限速逻辑，属已知未实现 |

## 7. 设计注记

- `message.rs` 仅含模块 doc comment，无任何独立类型定义
- `Gateway::route_message` 会自动创建 Session（key = `"channel:to"`）
- webhook 入向 `Message.to = String::new()`，由 Gateway 路由时填充
- `FeishuAdapter` 克隆共享同一个 `Arc<Mutex<Option<CachedToken>>>`，保证 token 跨克隆实例唯一刷新
