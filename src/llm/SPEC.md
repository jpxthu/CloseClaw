# LLM 模块规格说明书

> 描述系统当前是什么，而非应该如何工作。

## 模块概述

`src/llm/` 提供多供应商 LLM 抽象层，支持 OpenAI（GPT）、Anthropic（Claude）、MiniMax（M2 系列）三大提供商，以及测试用 StubProvider 和生产重试降级逻辑。

**模块职责**：
- 定义 `LLMProvider` trait，统一各提供商的调用接口
- 通过 `LLMRegistry` 管理多个 provider 实例
- 通过 `FallbackClient` 实现自动重试 + 降级链
- 通过 `CooldownManager` 管理 per-(provider, model) 的退避冷却

**公开导出**（`mod.rs` re-export）：
```rust
pub use anthropic::AnthropicProvider;
pub use minimax::MiniMaxProvider;
pub use openai::OpenAIProvider;
pub use stub::StubProvider;
```

---

## 核心类型

### Message

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    pub role: String,    // "system" | "user" | "assistant"
    pub content: String,
}
```

JSON 字段顺序固定（role 在前），确保跨 provider 的序列化兼容性。

### ChatRequest

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default)]           // 不填默认为 0.0
    pub temperature: f32,
    #[serde(default)]           // 不填表示无上限
    pub max_tokens: Option<u32>,
}
```

### ChatResponse

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatResponse {
    pub content: String,        // 模型原始输出（Anthropic thinking tag 已剥离）
    pub model: String,          // 实际响应模型名
    pub usage: Usage,
}
```

### Usage

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}
```

---

## LLMProvider Trait

```rust
#[async_trait]
pub trait LLMProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError>;
    fn models(&self) -> Vec<&str>;
    fn is_stub(&self) -> bool { false }
}
```

- `Send + Sync`：允许跨线程共享
- `is_stub()`：返回 `true` 时 caller 应视为配置错误（测试/演示环境）
- `chat()`：各 provider 负责自己的 HTTP 逻辑、错误映射、超时处理

---

## LLMError 与 ErrorKind

### LLMError 变体

```rust
pub enum LLMError {
    AuthFailed(String),        // 401 — 凭证无效，不重试
    RateLimitExceeded,        // 429 — 可重试
    ModelNotFound(String),    // 400/404 — 模型不存在
    InvalidRequest(String),   // 400/422 — 请求格式错误
    ApiError(String),         // 5xx — 服务端错误
    NetworkError(String),     // 连接超时/断开
}
```

### ErrorKind 分类（用于重试策略仲裁）

```rust
pub enum ErrorKind {
    Transient,    // 429/5xx/timeout — 指数退避重试
    Auth,         // 401/403 — 换凭证，不重试
    Billing,      // 402/配额耗尽 — 长时间冷却
    InvalidRequest, // 400/422 — 修请求，不重试
    Unknown,      // 未知错误 — 有限重试
}
```

**仲裁规则**：
- `AuthFailed` → `Auth`
- `RateLimitExceeded` → `Transient`
- `InvalidRequest` / `ModelNotFound` → `InvalidRequest`
- `ApiError`：根据状态码消息内容进一步细分（包含 500/502/503/504 → Transient，400/422 → InvalidRequest，401/403 → Auth）
- `NetworkError` → `Transient`

---

## LLMRegistry

```rust
pub struct LLMRegistry {
    providers: tokio::sync::RwLock<HashMap<String, Arc<dyn LLMProvider>>>,
}
```

线程安全，支持异步并发读写。通过 `Arc<dyn LLMProvider>` 共享 provider 实例。

### 公开方法

| 方法 | 签名 | 说明 |
|------|------|------|
| `new` | `() -> Self` | 创建空 Registry |
| `register` | `async fn(&self, name: String, provider: Arc<dyn LLMProvider>)` | 注册 provider |
| `get` | `async fn(&self, name: &str) -> Option<Arc<dyn LLMProvider>>` | 按名称获取 |
| `list` | `async fn(&self) -> Vec<String>` | 列出所有已注册名称 |

---

## FallbackClient

负责生产级重试 + 降级链。封装 `LLMRegistry` + 冷切管理。

### ModelEntry

```rust
pub struct ModelEntry {
    pub provider: String,  // 对应 LLMRegistry 中的名称
    pub model: String,    // 具体模型名（如 "MiniMax-M2.7"）
}
```

### 构造方式

| 方法 | 说明 |
|------|------|
| `new(registry, Vec<ModelEntry>)` | 同步构造（启动阶段） |
| `new_async(registry, Vec<ModelEntry>)` | 异步构造（运行时） |
| `from_strings(registry, Vec<String>)` | 从 `"provider/model"` 字符串解析 |
| `with_timeout(u64)` | 设置单次调用超时（默认 30s） |

### 降级策略（两阶段）

**阶段 1 — 当前模型重试**：
- `Transient` / `Unknown` 错误：指数退避，最多重试 3 次（Unknown 1 次）
- 退避基础延迟：60s，上限：1h
- 计算公式：`base * 2^(attempt-1) + jitter(0~10%)`

**阶段 2 — 切换下一个模型**：
- 重试耗尽后，切换 fallback chain 中的下一个模型
- `InvalidRequest` / `Billing` / `Auth`：立即切换，不重试当前模型

### 冷却管理

`FallbackClient` 持有 `Arc<CooldownManager>`，每次失败调用 `record_failure()`，成功调用 `record_success()`。处于冷却中的模型会被跳过。

---

## CooldownManager

```rust
pub struct CooldownManager {
    cooldowns: RwLock<HashMap<String, CooldownEntry>>,
    persist_path: PathBuf,
}

pub struct CooldownEntry {
    pub attempts: u32,           // 连续失败次数
    pub cooldown_until: String,  // RFC3339 UTC 时间戳
    pub reason: String,          // "transient" | "billing" | "auth"
}
```

### 冷却延迟

| ErrorKind | 基础延迟 | 上限 |
|-----------|---------|------|
| `Transient` | 60s × 2^attempts | 1h |
| `Billing` | 5h × 2^attempts | 24h |
| `Auth` | 1h | 1h |
| `Unknown` | 30s | 30s |
| `InvalidRequest` | 不设置冷却 | — |

### 持久化

- 冷却状态写入 `~/.closeclaw/llm_cooldowns.json`（可通过 `LLM_COOLDOWN_FILE` 环境变量覆盖路径）
- `load_sync()`：同步版本，用于无运行时启动阶段（使用一次性 runtime）
- `load()`：异步版本，用于运行时
- `save()`：每次变更后异步写盘

### 关键行为

- `is_in_cooldown()`：解析 RFC3339 时间戳比较当前时间
- `record_success()`：清除对应条目并写盘
- `InvalidRequest` 不设置冷却（立即切换模型即可）

---

## Provider 实现

### OpenAIProvider

- HTTP 端点：`https://api.openai.com/v1/chat/completions`
- 认证：`Authorization: Bearer {api_key}`
- 请求体：符合 OpenAI Chat Completions API 格式
- 错误映射：401 → `AuthFailed`，429 → `RateLimitExceeded`，400 → `InvalidRequest`，5xx → `ApiError`

#### 构造器

```rust
pub fn new(api_key: String) -> Self;
```

### AnthropicProvider

- HTTP 端点：`https://api.anthropic.com/v1/messages`
- 认证：`x-api-key: {api_key}` + `anthropic-version: 2023-06-01`
- 请求体：Anthropic Messages API 格式（`max_tokens` 必填，`messages` 数组）
- 错误映射：401 → `AuthFailed`，429 → `RateLimitExceeded`，400 → `InvalidRequest`，5xx → `ApiError`
- streaming 支持（future extension 标记）

#### 构造器

```rust
pub fn new(api_key: String) -> Self;
```

### MiniMaxProvider

- HTTP 端点：构造函数注入（`base_url` 字段）
- 认证：`Authorization: Bearer {api_key}`
- 模型列表：`["MiniMax-M2", "MiniMax-M2.1", "MiniMax-M2.5", "MiniMax-M2.7"]`
- **特殊处理**：响应 `content` 中的 `<think>...</think>` thinking tag 会自动剥离后再返回
- 错误映射：401 → `AuthFailed`，429 → `RateLimitExceeded`，400 → `InvalidRequest`

#### 构造器

```rust
pub fn new(api_key: String) -> Self;
pub fn from_env() -> Option<Self>;  // 从环境变量 MINIMAX_API_KEY 构造，存在则返回 Some
```

### StubProvider

- 测试用，始终返回固定响应
- `is_stub()` → `true`，caller 应将此视为配置错误
- `with_response(s)` 可自定义响应内容

---

## Retry 常量

文件：`src/llm/retry.rs`

以下为公开常量，用于重试策略配置：

```rust
pub const MAX_TRANSIENT_RETRIES: u32 = 3;    // Transient 错误最大重试次数
pub const MAX_UNKNOWN_RETRIES: u32 = 1;      // Unknown 错误最大重试次数
pub const TRANSIENT_BASE_DELAY: Duration = Duration::from_secs(60);    // 60s
pub const TRANSIENT_MAX_DELAY: Duration = Duration::from_secs(3600);  // 1h
pub const BILLING_MAX_DELAY: Duration = Duration::from_secs(86400);   // 24h
```

### CooldownManager 构造

```rust
impl CooldownManager {
    pub fn new() -> Self;                    // 内存版本（无持久化路径）
    pub fn load_sync(&self);                 // 同步加载持久化状态
}
```

### LLMError 方法

```rust
impl LLMError {
    pub fn kind(&self) -> ErrorKind;        // 将 LLMError 分类为 ErrorKind
}
```

---

## 已知空白（代码有、文档无）

1. **`ErrorKind` 枚举**：整个错误分类体系 docs/llm/README.md 未提及
2. **`FallbackClient` / `CooldownManager`**：生产重试降级逻辑完全未文档化
3. **`LLMRegistry` 并发安全设计**：RwLock 机制未说明
4. **`StubProvider`**：README 完全未提及
5. **冷却持久化机制**：`~/.closeclaw/llm_cooldowns.json` 文件路径约定未说明
6. **`07-multi-provider-cache-adapter.md`（workspace 设计）** 提及 MiniMax 缓存机制为"待确认"，当前代码无任何缓存机制
