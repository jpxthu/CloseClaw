# IM Module Specification

## 1. Module Responsibility

The `im` module provides cross-platform IM (Instant Messaging) adapter abstractions. Each adapter implements the `IMAdapter` trait for a specific messaging platform, converting inbound webhook events into internal `Message` format and translating outbound `Message`s into platform-specific API calls.

**Module root**: `src/im/`

---

## 2. Public Interface

### 2.1 `IMAdapter` Trait

```rust
#[async_trait]
pub trait IMAdapter: Send + Sync {
    fn name(&self) -> &str;
    async fn handle_webhook(&self, payload: &[u8]) -> Result<Message, AdapterError>;
    async fn send_message(&self, message: &Message) -> Result<(), AdapterError>;
    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool;
}
```

| Method | Description |
|--------|-------------|
| `name()` | Returns platform name string (e.g. `"feishu"`). Used as adapter lookup key in the gateway. |
| `handle_webhook(payload)` | Parses a raw webhook HTTP body (`&[u8]`), returns an internal `Message`. Returns `AdapterError::InvalidPayload` on parse failure. |
| `send_message(message)` | Sends `message.content` as a platform message to `message.to`. Returns `AdapterError::SendFailed` on API error. |
| `validate_signature(signature, payload)` | Verifies webhook authenticity. Returns `true` if valid, `false` otherwise. |

**Implementations must be thread-safe**: the trait requires `Send + Sync` so adapters may be shared across async tasks via `Arc`.

### 2.2 `AdapterError` Enum

```rust
pub enum AdapterError {
    InvalidPayload(String),   // JSON parse failure or schema mismatch
    AuthFailed,               // Authentication / token error
    SendFailed(String),       // Upstream API error, includes error message
    InvalidSignature,         // Webhook signature mismatch
    IoError(std::io::Error),  // Network / file I/O error
}
```

All variants carry sufficient context for logging. `SendFailed` embeds the upstream error message verbatim.

---

## 3. `FeishuAdapter` Implementation

**File**: `src/im/feishu.rs`

### 3.1 Construction

```rust
impl FeishuAdapter {
    pub fn new(app_id: String, app_secret: String, verification_token: String) -> Self
}
```

Creates a `FeishuAdapter` with a shared `HttpClient` (30 s timeout) and an empty token cache.

### 3.2 Internal Token Management

```rust
async fn get_tenant_token(&self) -> Result<String, AdapterError>
async fn fetch_tenant_token(&self) -> Result<String, AdapterError>
```

- `get_tenant_token()` is the public-facing token accessor. It checks an in-memory `Arc<Mutex<Option<CachedToken>>>` cache before fetching.
- **Cache TTL**: 7200 s (2 hours, Feishu standard). Proactive refresh triggers when `< 300 s` remaining.
- `fetch_tenant_token()` calls `POST /auth/v3/tenant_access_token/internal` and returns `AdapterError::SendFailed` on non-zero `code` or missing `tenant_access_token`.

### 3.3 Webhook Handling

`handle_webhook` deserializes the Feishu event envelope (`FeishuEvent`) and extracts the text content:

```text
FeishuEvent.schema
FeishuEvent.header.event_id       → Message.id
FeishuEvent.header.event_type
FeishuEvent.header.create_time
FeishuEvent.header.token
FeishuEvent.header.app_id
FeishuEvent.event.sender.sender_id.open_id  → Message.from
FeishuEvent.event.chat_id
FeishuEvent.event.content (JSON string) → parse → .text field → Message.content
FeishuEvent.event.message_type
```

`Message.to` is set to `String::new()` — the gateway is responsible for filling it based on session context.

### 3.4 Signature Validation

```rust
async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool
```

Computes `SHA256(verification_token + payload)`, hex-encodes, compares with `signature` using constant-time equality.

### 3.5 Sending Text Messages

```rust
async fn send_message(&self, message: &Message) -> Result<(), AdapterError>
```

Sends `message.content` as a Feishu `text` message to `message.to` via `POST /im/v1/messages?receive_id_type=open_id`.

### 3.6 Card Operations

```rust
pub async fn send_card(&self, chat_id: &str, card: &RichCard) -> Result<String, AdapterError>
pub async fn update_message(&self, message_id: &str, patch: &serde_json::Value) -> Result<(), AdapterError>
```

- `send_card`: Sends an interactive card (`msg_type: "interactive"`) and returns the Feishu `message_id` on success.
- `update_message`: Patches an existing card message identified by `message_id` with full card content via `PATCH /im/v1/messages/{message_id}`.

Both require a valid tenant token (acquired via `get_tenant_token()`).

---

## 4. Key Data Structures

### 4.1 `FeishuEvent` / `FeishuHeader` / `FeishuMessageEvent` / `FeishuSender`

Deserialized from Feishu webhook HTTP body. All fields are `#[allow(dead_code)]` (not all fields are consumed by `handle_webhook`).

### 4.2 `CachedToken`

```rust
struct CachedToken {
    token: String,
    expires_at: Instant,
}
```

Cache entry storing the raw token string and its absolute expiry time. `needs_refresh()` returns `true` when `Instant::now() > expires_at - 300 s`.

### 4.3 `FeishuAdapter`

```rust
pub struct FeishuAdapter {
    app_id: String,
    app_secret: String,
    verification_token: String,
    http_client: Client,
    cached_token: Arc<Mutex<Option<CachedToken>>>,
}
```

All fields are private. Cloning shares the `Arc<Mutex<Option<CachedToken>>>` cache across clones.

---

## 5. Module Boundaries

```
gateway/mod.rs  ──uses──▶  im/mod.rs (IMAdapter trait)
                              │
                              └── im/feishu.rs (FeishuAdapter)

im/mod.rs imports from gateway: Message
card/mod.rs (separate crate) ──used by──▶ im/feishu.rs (send_card / update_message via RichCard)
```

- **Depends on**: `gateway::Message` (imported as `use crate::gateway::Message`)
- **Depends on**: `card::render_feishu_card` and `card::RichCard`
- **Exports**: `IMAdapter` trait, `AdapterError` enum, `FeishuAdapter` struct, `feishu` module
- **Does not own**: `Message` struct (defined in `gateway`)

---

## 6. Constants

| Name | Value | Description |
|------|-------|-------------|
| `FEISHU_API_BASE` | `"https://open.feishu.cn/open-apis"` | Base URL for all Feishu Open API calls |

---

## 7. Deviation Analysis: Code vs. `FEISHU_STREAM_FALLBACK.md`

> `FEISHU_STREAM_FALLBACK.md` (docs/agent/) describes a **planned** feature (Issue #161). The current codebase implements only the base Feishu adapter. The following items exist in the design doc but are **not implemented** in the code:

| Design Doc Element | Status in Code |
|--------------------|----------------|
| `should_fallback(mode)` method | ❌ Not implemented |
| `execute_fallback()` method | ❌ Not implemented |
| `PlatformCapabilityService` | ❌ Not implemented |
| `CardService` trait | ❌ Not implemented |
| `FeishuMessageService` trait | ❌ Not implemented |
| `FallbackStep` / `FallbackAction` types | ❌ Not implemented |
| `PlanCardConfig` / `PlanSection` / `StepStatus` types | ❌ Not implemented |
| `build_initial_sections()` function | ❌ Not implemented |
| `is_high_complexity()` / `HighComplexityConfig` | ❌ Not implemented |
| `FeishuAdapterError` enum | ❌ Not implemented |
| `feishu.stream_fallback.enabled` config | ❌ Not implemented |
| `fallback_delay_threshold_ms` config | ❌ Not implemented |
| File layout under `src/platform/feishu/` | ❌ File structure does not match; actual layout is `src/im/` |

### What IS implemented (card sending):

The only card-related functionality currently present is `send_card()` and `update_message()`, which send arbitrary `RichCard` payloads to Feishu. The `RichCard` type and `render_feishu_card()` are defined in a separate `card` crate and consumed here.

---

## 8. Test Coverage

The module includes one unit test:

- `test_feishu_adapter_name`: constructs a `FeishuAdapter` via `new()` and asserts `name() == "feishu"`.

No tests exist for token caching, signature validation, webhook parsing, or message sending.
