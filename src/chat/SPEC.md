# chat 模块规格书

## 模块概述

提供进程内 TCP 聊天服务，默认监听 `127.0.0.1:18889`，支持动态指定绑定地址。通过 JSON 换行分隔协议（NDJSON）与客户端交互。每个 TCP 连接对应一个独立会话，会话将用户消息委托给 LLM（带 fallback 链），完整响应后一次性写回客户端。

核心设计：per-connection session + broadcast shutdown + LLM fallback 链 + 历史窗口截断。

---

## 公开接口

### protocol.rs

- **`ClientMessage`**：客户端消息枚举，反序列化和分发用
  - `ChatStart` — 发起新会话
  - `ChatMessage` — 发送聊天内容
  - `ChatStop` — 结束会话
- **`ServerMessage`**：服务器消息枚举
  - `ChatStarted` — 会话建立确认
  - `ChatResponse` — LLM 响应内容块
  - `ChatResponseDone` — 响应流结束标记
  - `ChatError` — 协议层错误或 LLM 调用失败
- **`ServerMessage::to_json()`** — 序列化为 JSON 字符串

### server.rs

- **`ChatServer::new()`** — 构造服务器；接受可选 `bind_addr` 参数为 `None` 时默认绑定 `127.0.0.1:18889`
- **`ChatServer::run()`** — 异步主循环：bind TCP listener，循环 accept，每连接 spawn 一个 ChatSession
- **`ChatServer::shutdown()`** — 发送广播信号，通知所有 session 停止
- **`spawn_chat_server()`** — 便捷工厂函数，默认绑定 `127.0.0.1:18889`，返回 ChatServer 实例

### session.rs

- **`ChatSession::new()`** — 构造 session；从环境变量初始化 fallback_client、max_history、timeout
- **`ChatSession::run()`** — 主事件循环：读客户端消息 → 分发处理 → 写回响应；并监听 shutdown 信号
- **`ChatSession::handle_line()`** — 解析 ClientMessage，分发 ChatStart/ChatMessage/ChatStop
- **`ChatSession::handle_chat_message()`** — 处理 ChatMessage：追加历史 → 截断 → 调用 LLM → 返回响应消息
- **`ChatSession::truncate_history()`** — 裁剪 chat_history 到 max_history 条

#### 测试可见字段/方法（供测试访问）

session.rs 内 `#[cfg(test)] mod tests` 需要访问以下私有逻辑，设为 `pub` 仅用于测试：

- **`chat_history: Vec<Message>`**（pub）— 会话消息历史，测试直接构造和断言
- **`max_history: usize`**（pub）— 最大历史条数，测试直接修改以覆盖边界条件
- **`truncate_history()`**（pub）— 测试直接调用验证截断行为

#### 测试覆盖

| 测试 | 覆盖场景 |
|------|----------|
| `test_truncate_history` | history 超过/未超限/恰好/为空的所有边界场景 |
| `test_handle_line_chat_start` | ChatStart JSON → 返回 ChatStarted，agent_id 更新 |
| `test_handle_line_chat_message` | ChatMessage JSON → 返回 ChatResponse + ChatResponseDone，history 正确追加 |
| `test_handle_line_chat_stop` | ChatStop JSON → active 变 false，返回 ChatResponseDone |
| `test_handle_line_invalid_json` | 非法 JSON → 返回 ChatError |
| `test_send_message_writes_json` | send_message 写入合法 JSON + 换行 |
| `test_chat_session_new_fields` | new() 构造后 session_id、agent_id、model、max_history 字段正确 |
| `test_handle_chat_message_llm_failure`（fake-llm） | LLM 返回错误时返回含 `[error]` 的 ChatResponse |
| `test_full_session_lifecycle`（tests/） | 完整 TCP 交互：ChatStart → ChatMessage → ChatStop |
| `test_session_shutdown_signal`（tests/） | shutdown 信号 → 收到 ChatError("server shutting down") |

---

## 架构与结构

### 子模块

| 文件 | 职责 |
|------|------|
| `protocol.rs` | JSON 消息类型定义（客户端↔服务器双向枚举） |
| `server.rs` | TCP 服务器入口：绑定端口、broadcast shutdown、per-connection spawn |
| `session.rs` | 单连接会话状态：读消息、LLM 调用、写回响应、历史管理 |

### 数据流

```
TCP Client
    │
    ▼ read_line (一行 JSON)
ClientMessage ──► ChatSession::handle_line
                        │
                        ├─ ChatStart  ──► ChatSession::new agent_id  ──► ChatStarted
                        │
                        ├─ ChatMessage ──► handle_chat_message()
                        │                   ──► append user to history → truncate_history()
                        │                   ──► call_llm() via FallbackClient
                        │                   ──► append assistant to history
                        │                   ──► ChatResponse + ChatResponseDone
                        │
                        └─ ChatStop ──► active = false  ──► ChatResponseDone

ChatSession::send_message
    │
    ▼ write_all(JSON + "\n")
TCP Client
```

### 关键设计模式

- **Broadcast Shutdown**：`tokio::sync::broadcast`（容量 1）驱动全服务器优雅退出
- **Spawn-per-Connection**：每 accepted TCP 连接 spawn 一个 async task，不阻塞主循环
- **FallbackClient 链**：从 `LLM_FALLBACK_CHAIN` 环境变量解析模型列表，自动重试与降级
- **TCP split + BufReader**：`into_split()` 分离读写半连接，读端套 BufReader 后 `read_line`
- **历史窗口截断**：超出 `max_history` 时从头部移除最旧消息

---

## 行为规范

### 连接与生命周期

1. 服务器绑定地址（可配置，默认 `127.0.0.1:18889`），接受 TCP 连接。
2. 每条连接生成 UUID V4 作为 `session_id`，独立 `shutdown_rx` 订阅。
3. 服务器 `shutdown()` 调用时，给所有活跃 session 发送 `ChatError{message: "server shutting down"}`，然后终止。

### 会话循环

1. 从 TCP 连接读取一行（`read_line`），去掉末尾换行；空行跳过。
2. 将原始行 JSON 反序列化为 `ClientMessage`；失败返回 `ChatError`。
3. 根据消息类型分发：
   - `ChatStart`：更新 `self.agent_id`，返回 `ChatStarted`。
   - `ChatMessage`：委托给 `handle_chat_message`：追加 user 消息 → truncate →
     call LLM → 追加 assistant 消息 → 返回 `ChatResponse` + `ChatResponseDone`；
     LLM 失败时 content 字段填充 `[error]` 信息。
   - `ChatStop`：设置 `active = false`，返回 `ChatResponseDone`。
4. `send_message` 写响应 JSON 加 `\n` 后 flush 到客户端；写入失败则退出循环。

### LLM 调用

- 使用 `FallbackClient`，从 `LLM_FALLBACK_CHAIN` 读取模型回退链（格式 `"provider/model,provider/model"`），逗号分隔。
- 未配置时，从 `LLM_PROVIDER` + `LLM_MODEL` 构建单模型链。
- 超时从 `LLM_TIMEOUT_SECS` 读取，默认 30 秒。
- 请求参数：`temperature=0.7`，`max_tokens=2048`。

### 错误处理

| 场景 | 行为 |
|------|------|
| 收到非法 JSON | `ChatError{message: "invalid message: <解析错误>"}` |
| LLM 调用失败 | `ChatResponse{content: "[error] LLM call failed: ..."} + ChatResponseDone` |
| 客户端断开（read 返回 0） | 会话正常退出 |
| 服务器关闭信号 | `ChatError{message: "server shutting down"}` |

---

## 模块边界

- **依赖 `crate::llm`**：`LLMRegistry`（查询可用模型）、`FallbackClient`（LLM 调用）、`Message`/`ChatRequest`（请求结构）
- **不依赖其他业务模块**（agent、session 等），独立运行
- **不实现流式输出**：LLM 响应完整返回后一次性发送，`done` 固定为 `true`

---

## 环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `CHAT_MAX_HISTORY` | `100` | 最大历史消息条数 |
| `CHAT_SERVER_ADDR` | `127.0.0.1:18889` | CLI 连接 chat server 的地址（优先级：CLI参数 > 环境变量 > 默认值） |
| `LLM_FALLBACK_CHAIN` | `"<LLM_PROVIDER>/<LLM_MODEL>"` | 逗号分隔的模型回退链 |
| `LLM_PROVIDER` | `"minimax"` | LLM 提供商 |
| `LLM_MODEL` | `"MiniMax-M2.5"` | 默认模型 |
| `LLM_TIMEOUT_SECS` | `30` | LLM 调用超时（秒） |

---

## 已知行为

1. **`done` 字段固定为 `true`**：目前没有流式输出支持，`ChatResponse` 的 `done` 始终为 `true`。
2. **`model` 字段仅用于 FallbackClient 构造**：`ChatSession.model` 从环境变量初始化，但只在构造 `FallbackClient` 时使用，不参与 LLM 请求路由。
3. **`agent_id` 不做验证**：服务器不检查 `agent_id` 是否在 `LLMRegistry` 中注册。
4. **`active` 字段不影响循环**：`ChatStop` 设置 `active = false` 但 run loop 并不依赖此标志。

---

*变更历史*
- 2026-04-20：#226 提取 `handle_chat_message` 方法，同步 SPEC
- 2026-04-14：按 v3 标准重写，精简接口签名、补充架构/数据流章节
