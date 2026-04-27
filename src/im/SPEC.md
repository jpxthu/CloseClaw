# IM Module Specification

## 1. Module Overview

`im` 模块提供跨平台 IM（Instant Messaging）适配器抽象。每个适配器实现 `IMAdapter` trait，将平台 webhook 事件转换为内部 `Message` 格式，并将发出的 `Message` 翻译为平台专有的 API 调用。

**模块路径**: `src/im/`

---

## 2. Public Interfaces

### `IMAdapter` Trait

| Method | Description |
|--------|-------------|
| `name(&self) -> &str` | 返回平台名称字符串（如 `"feishu"`），作为 gateway 中的适配器查找键 |
| `async fn handle_webhook(&self, payload: &[u8]) -> Result<Message, AdapterError>` | 解析原始 webhook HTTP body，返回内部 `Message`；解析失败返回 `InvalidPayload` |
| `async fn send_message(&self, message: &Message) -> Result<(), AdapterError>` | 将 `message.content` 发送至 `message.to`；API 错误返回 `SendFailed` |
| `async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool` | 验证 webhook 真实性；有效返回 `true`，无效返回 `false` |

实现必须线程安全（trait 要求 `Send + Sync`），以便通过 `Arc` 在异步任务间共享。

### `AdapterError` Enum

| Variant | Description |
|---------|-------------|
| `InvalidPayload(String)` | JSON 解析失败或 schema 不匹配 |
| `AuthFailed` | 认证或 token 错误 |
| `SendFailed(String)` | 上游 API 错误，携带上游错误信息 |
| `InvalidSignature` | Webhook 签名不匹配 |
| `IoError(std::io::Error)` | 网络或文件 IO 错误（带 `#[from]` 自动转换） |

---

## 3. `FeishuAdapter` Implementation

**文件**: `src/im/feishu.rs`

### 3.1 Construction

`FeishuAdapter::new(app_id, app_secret, verification_token) -> Self` — 创建 `FeishuAdapter`，内部共享 `HttpClient`（30s 超时），`cached_token` 初始化为空。构造时不发起任何外部 API 调用。

### 3.2 Token Management

`get_tenant_token(&self) -> Result<String, AdapterError>` — public-facing token 访问器。先检查内存 `Arc<Mutex<Option<CachedToken>>>` 缓存；缓存有效则直接返回，否则调用 `fetch_tenant_token()`。

`fetch_tenant_token(&self) -> Result<String, AdapterError>` — private。POST 到 `/auth/v3/tenant_access_token/internal`，解析 `code` / `msg` / `tenant_access_token`；`code != 0` 时返回错误。

**Cache TTL**: 7200s（飞书标准）。提前 300s 触发主动刷新（`needs_refresh()`）。

### 3.3 Webhook Handling

`async fn handle_webhook` 反序列化飞书事件 envelope（`FeishuEvent`），提取文本内容：

```
FeishuEvent.header.event_id       → Message.id
FeishuEvent.header.event_type
FeishuEvent.event.sender.sender_id.open_id  → Message.from
FeishuEvent.event.content (JSON string) → parse → .text → Message.content
FeishuEvent.event.message_type
FeishuEvent.header.app_id         → Message.metadata["account_id"]
```

`Message.to` 留空，由 gateway 根据 session context 填充。

`validate_signature` 计算 `SHA256(verification_token + payload)`，hex 编码，用常量时间比较。

### 3.4 Message Sending

`async fn send_message(message)` — 通过 `POST /im/v1/messages?receive_id_type=open_id` 发送文本消息至 `message.to`。

### 3.5 Card Operations

`async fn send_card(chat_id, card)` — 调用 `render_feishu_card` 渲染卡片，POST 至 `/im/v1/messages?receive_id_type=open_id`（`msg_type: "interactive"`），成功返回飞书 `message_id`。

`async fn update_message(message_id, patch)` — 通过 `PATCH /im/v1/messages/{message_id}` 更新已有卡片消息，携带完整卡片内容（飞书要求全量 patch）。

两者均需有效 tenant token（通过 `get_tenant_token()` 获取）。

---

## 4. `processor` — Message Cleaning

**文件**: `src/im/processor.rs`

将飞书 webhook 原始事件（text / post 两种 message type）清洗为干净的 `ProcessedMessage`，去除敏感字段（tenant_key、app_id、open_id、chat_id、message_type）。

### 4.1 Module Overview

`processor` 负责飞书入站消息的"清洗"层。它从 `clean_feishu_message()` 入口接收完整的飞书 webhook JSON（`event` envelope），提取内层 `message` 字段，按 `message_type` 分派到对应的清洗函数。

Text 消息：解析内层 JSON 的 `content` 字段，提取 `text`，将 `@_user_N` 占位符替换为实际用户名。

Post 消息：将 `title`（非空时）和 `content`（blocks 数组）渲染为多行纯文本，处理样式嵌套（bold / italic / underline / lineThrough），图片段输出 `[图片]`。

### 4.2 Public Interfaces

| Interface | Description |
|-----------|-------------|
| `ProcessedMessage { content, metadata }` | 清洗结果；content 为纯文本，metadata 仅在 group 聊天时含 `chat_type` |
| `async fn clean_feishu_message(raw: &Value) -> ProcessedMessage` | 主入口；解析 webhook JSON，分派到 text / post 清洗逻辑 |

### 4.3 Internal Functions

| Function | Description |
|----------|-------------|
| `clean_text_message` | 从 text message 的 `content` JSON 中提取 `text`，调用 `replace_mentions` |
| `clean_post_message` | 渲染 post message 的 title + blocks；空 block 输出空行；title 为空时不输出标题行 |
| `render_post_block` | 将一个 block（text/img segments 数组）渲染为一行字符串 |
| `render_segment` | 渲染单个 segment：text 段应用样式嵌套，img 段输出 `[图片]` |
| `apply_styles` | 处理样式标签包裹；underline+bolds / lineThrough+underline / lineThrough+underline+bold 三个组合使用固定顺序直接匹配 fixture 输出；其余按 reverse-order（最后出现的样式在最外层） |
| `wrap_style` | 将单个样式标签包裹文本，返回 `**text**` / `_text_` / `<u>text</u>` / `~~text~~` |
| `replace_mentions` | 将 `@_user_N` 占位符替换为 `@用户名` |

### 4.4 Style Nesting

样式嵌套顺序：
- `underline` + `bold` → `**<u>{text}</u>**`
- `lineThrough` + `underline` → `~~<u>{text}</u>~~`
- `lineThrough` + `underline` + `bold` → `**<u>~~{text}~~</u>**`
- 其他组合：按 reverse-order，即最后出现的样式在最外层

### 4.5 Post Rendering Special Cases

- **空 block**（空数组）：`render_post_block` 输出空字符串，最终输出空行
- **仅 img block**：`[图片]` 前插入空行
- **heading block**（渲染后文本以 `#` 开头）：在该 block 后插入空行
- **引用 text**（`text == "引用"`）：输出为 `> {styled_text}` blockquote 格式

---

## 5. Architecture

```
gateway/mod.rs  ──uses──▶  im/mod.rs (IMAdapter trait)
                              │
                              ├── im/feishu.rs (FeishuAdapter)
                              └── im/processor.rs (clean_feishu_message)

card crate  ──used by──▶  im/feishu.rs (render_feishu_card / RichCard)
```

**依赖**：`gateway::Message`（定义在 `gateway` 模块，不在本模块内）
**依赖**：`card::render_feishu_card` 和 `card::RichCard`
**导出**：`IMAdapter` trait、`AdapterError` enum、`FeishuAdapter`、`feishu` 子模块

---

## 6. Constants

| Name | Value | Description |
|------|-------|-------------|
| `FEISHU_API_BASE` | `"https://open.feishu.cn/open-apis"` | 所有飞书 Open API 调用的 Base URL |
