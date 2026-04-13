# chat 模块规格书

## 模块职责

提供进程内 TCP 聊天服务，监听 `127.0.0.1:18889`，通过 JSON 换行分隔协议（NDJSON）与客户端交互，为每个 TCP 连接建立独立的聊天会话，并将用户消息委托给 LLM 处理。

---

## 网络配置

| 项目 | 值 |
|------|-----|
| 绑定地址 | `127.0.0.1:18889` |
| 协议 | TCP + JSON-NL（每条消息以 `\n` 分隔） |
| 默认 agent_id | `"guide"` |

---

## 协议格式

### 客户端 → 服务器

| type | 字段 | 说明 |
|------|------|------|
| `chat.start` | `agent_id: String`, `id: String` | 发起新会话 |
| `chat.message` | `content: String`, `id: String` | 发送聊天内容 |
| `chat.stop` | `id: String` | 结束会话 |

### 服务器 → 客户端

| type | 字段 | 说明 |
|------|------|------|
| `chat.started` | `session_id: String`, `id: String` | 确认会话建立 |
| `chat.response` | `content: String`, `done: bool`, `id: String` | LLM 响应内容块 |
| `chat.response.done` | `id: String` | 响应流结束标记 |
| `chat.error` | `message: String`, `id: String` | 协议层错误或 LLM 调用失败 |

---

## 公开接口

### `protocol.rs`

```rust
// 客户端消息（服务器端反序列化）
pub enum ClientMessage {
    ChatStart { agent_id: String, id: String },
    ChatMessage { content: String, id: String },
    ChatStop { id: String },
}

// 服务器消息（客户端反序列化）
pub enum ServerMessage {
    ChatStarted { session_id: String, id: String },
    ChatResponse { content: String, done: bool, id: String },
    ChatResponseDone { id: String },
    ChatError { message: String, id: String },
}

impl ServerMessage {
    pub fn to_json(&self) -> anyhow::Result<String>
}
```

### `server.rs`

```rust
pub const DEFAULT_MAX_HISTORY: usize = 100;  // 超过后触发 compact

pub struct ChatServer { ... }

impl ChatServer {
    pub fn new(llm_registry: Arc<LLMRegistry>) -> Self
    pub async fn run(&self, shutdown_rx: broadcast::Receiver<()>) -> anyhow::Result<()>
    pub fn shutdown(&self)
}

pub fn spawn_chat_server(llm_registry: Arc<LLMRegistry>) -> ChatServer
```

### `session.rs`

```rust
pub struct ChatSession {
    pub session_id: String,
    pub agent_id: String,
    // ...
}

impl ChatSession {
    pub fn new(
        session_id: String,
        agent_id: String,
        stream: TcpStream,
        shutdown_rx: broadcast::Receiver<()>,
        llm_registry: Arc<LLMRegistry>,
    ) -> Self

    pub async fn run(mut self)
}
```

---

## 核心数据结构

### `ChatSession`

| 字段 | 类型 | 说明 |
|------|------|------|
| `session_id` | `String` | 服务器生成的 UUID V4，全局唯一 |
| `agent_id` | `String` | 当前会话的 agent 标识，可被 `ChatStart` 更新 |
| `active` | `bool` | 会话是否活跃（`ChatStop` 后置为 `false`） |
| `chat_history` | `Vec<Message>` | 累积对话历史 |
| `max_history` | `usize` | 历史上限，默认 100 |
| `fallback_client` | `Arc<FallbackClient>` | LLM 调用客户端（含重试与模型回退链） |
| `model` | `String` | 当前使用的模型名称 |

### `Message`（来自 `crate::llm::Message`）

| 字段 | 类型 | 说明 |
|------|------|------|
| `role` | `String` | `"user"` 或 `"assistant"` |
| `content` | `String` | 消息正文 |

---

## 行为规范

### 连接与生命周期

1. 服务器绑定 `127.0.0.1:18889`，接受 TCP 连接。
2. 每条连接生成 UUID V4 作为 `session_id`，独立 `shutdown_rx` 订阅。
3. 服务器关闭信号（`shutdown()`）送达时，给所有活跃会话发送 `ChatError{message: "server shutting down"}`，然后终止。

### 会话循环（`run`）

1. 从 TCP 连接读取一行（`read_line`），去掉末尾换行。
2. 将原始行 JSON 反序列化为 `ClientMessage`；失败返回 `ChatError`。
3. 根据消息类型分发：
   - `ChatStart`：更新 `self.agent_id`，返回 `ChatStarted`。
   - `ChatMessage`：追加 `user` 消息到历史 → 调用 LLM → 追加 `assistant` 消息到历史 → 返回 `ChatResponse{done:true}` + `ChatResponseDone`；LLM 失败时 `content` 字段填充 `[error]` 信息。
   - `ChatStop`：设置 `active = false`，返回 `ChatResponseDone`。
4. 写响应 JSON 加 `\n` 后 flush 到客户端。

### 历史管理

- `chat_history` 超过 `max_history`（默认 100 条）时，从头部删除最旧的消息。
- 环境变量 `CHAT_MAX_HISTORY` 可覆盖上限。

### LLM 调用（`call_llm`）

- 使用 `FallbackClient`，从环境变量 `LLM_FALLBACK_CHAIN` 读取模型回退链（格式 `"minimax/MiniMax-M2.5,dashscope/qwen3-max"`），逗号分隔。
- 未配置时，从 `LLM_PROVIDER` + `LLM_MODEL` 构建单模型链。
- 超时从 `LLM_TIMEOUT_SECS` 读取，默认 30 秒。
- 请求参数：`temperature=0.7`，`max_tokens=2048`。

### 错误处理

| 场景 | 行为 |
|------|------|
| 收到非法 JSON | 返回 `ChatError{message: "invalid message: <解析错误>"}` |
| LLM 调用失败 | 返回 `ChatResponse{content: "[error] LLM call failed: ...", done:true}` + `ChatResponseDone` |
| 客户端断开（read 返回 0） | 会话正常退出 |
| 收到服务器关闭信号 | 发送 `ChatError{message: "server shutting down"}`，退出 |

---

## 模块边界

- **依赖 `crate::llm`**：`LLMRegistry`（查询可用模型）、`FallbackClient`（执行 LLM 调用）、`Message`/`ChatRequest`（请求结构）。
- **不依赖其他业务模块**（agent、session 等），独立运行。
- **不实现流式输出**：LLM 响应完整返回后一次性发送，`done` 固定为 `true`。

---

## 环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `CHAT_MAX_HISTORY` | `100` | 最大历史消息条数 |
| `LLM_FALLBACK_CHAIN` | `"<LLM_PROVIDER>/<LLM_MODEL>"` | 逗号分隔的模型回退链 |
| `LLM_PROVIDER` | `"minimax"` | LLM 提供商 |
| `LLM_MODEL` | `"MiniMax-M2.5"` | 默认模型 |
| `LLM_TIMEOUT_SECS` | `30` | LLM 调用超时（秒） |

---

## 已知行为

1. **`agent_id` 不做验证**：服务器不检查 `agent_id` 是否在 `LLMRegistry` 中注册，任何字符串均接受。
2. **`done` 字段固定为 `true`**：目前没有流式输出支持，`ChatResponse` 的 `done` 始终为 `true`。
3. **`model` 字段未用于路由**：`ChatSession.model` 从环境变量初始化，但仅在构造 `FallbackClient` 时使用，LLM 调用时直接传入 `ChatRequest` 的 `model` 字段与 `fallback_chain` 中的模型不关联。
4. **`ChatResponse` 无独立反序列化测试**：`protocol.rs` 测试套件仅覆盖 `ChatResponseDone` 的 JSON round-trip，未单独测试 `ChatResponse` 反序列化。

---

## 变更历史

- 2026-04-11：初版编写，基于源码分析。
