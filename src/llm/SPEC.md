# LLM 模块

## 模块概述

LLM 模块为 CloseClaw 提供统一的多 Provider LLM 调用抽象。通过 `LLMProvider` trait + `LLMRegistry` 实现插件式架构，支持同时注册多个 Provider（MiniMax、OpenAI、Anthropic、Stub）。核心设计：Provider 只负责 HTTP 调用和响应解析，错误分类由 `ErrorKind` 统一处理；重试和 fallback 逻辑由 `FallbackClient` 包装，对调用方透明。

`model_info.rs` 定义模型元数据类型（`ModelInfo`、`InputType`），承载模型标识、上下文窗口、温度等元数据。`knowledge.rs` 提供内嵌知识库（`ProviderModelKnowledge`），手填 MiniMax、GLM、VolcEngine、DeepSeek 四个 Provider 的推荐参数，供 Model Discovery & Auto-Config Wizard 使用。`model_cache.rs` 提供本地缓存层，按 (provider, SHA256hex(provider:token_prefix)) 组织，TTL 3600 秒自动过期，避免每次启动调 API 获取模型列表；缓存文件损坏或 key 不存在时静默返回 None。

边界：模块不负责 prompt 工程、不做缓存、不做 token 估算（Usage 数字由 Provider 透传）。冷却状态通过 `CooldownManager` 持久化到 `~/.closeclaw/llm_cooldowns.json`。

---

## 公开接口

### 数据类型（核心类型）

- **`Message`** — LLM 对话消息（role + content）
- **`ChatRequest`** — 聊天补全请求（model、messages、temperature、max_tokens）
- **`ChatResponse`** — 聊天补全响应（content、model、usage，不含 cooldown_retry_after）
- **`Usage`** — token 用量统计（prompt_tokens、completion_tokens、total_tokens）
- **`GlmQuotaResponse`** — GLM Quota API 响应（code、msg、data、success）
- **`GlmQuotaData`** — GLM Quota 数据（limits 数组、level）
- **`GlmLimit`** — 单条配额上限条目（type、unit、number、usage、remaining、percentage、next_reset_time）
- **`LLMError`** — 错误枚举：AuthFailed、RateLimitExceeded、ModelNotFound、InvalidRequest、ApiError、NetworkError
- **`ErrorKind`** — 错误可恢复性分类：Transient、Auth、Billing、InvalidRequest、Unknown
- **`Scenario`** — `FakeProvider` 场景枚举：Ok（成功响应）、Err（错误响应）、Delay（sleep 后执行内部场景，内部场景可为 Ok/Err/另一个 Delay，嵌套递归解析）
- **`StreamingResponse`** — 流式聊天响应接收器，`tokio::sync::mpsc::Receiver<ChatStreamChunk>`，调用方通过 `recv().await` 逐块消费
- **`ChatStreamChunk`** — 流式响应中的单个 chunk：`Text(String)`（文本片段）、`Done { model, usage }`（流结束元数据）、`Error(LLMError)`（流式过程中的错误）

### 数据类型（统一响应层）

- **`ContentBlockType`** — 内容块类型枚举：Text（纯文本）、Thinking（思考链）、ToolUse（工具调用）
- **`ContentBlock`** — 统一内容块枚举（`#[serde(tag = "type")]`，JSON 含 `type` 字段）：Text(String)、Thinking(String)、ToolUse{id/name/input}、ToolResult{tool_call_id/content}
- **`UnifiedResponse`** — 统一响应结构（content_blocks + usage + finish_reason）
- **`UnifiedUsage`** — 统一 token 用量（prompt_tokens、completion_tokens、total_tokens、reasoning_tokens）

### 数据类型（流式事件层）

- **`StreamEvent`** — 流式事件枚举：BlockStart、BlockDelta、BlockEnd、MessageEnd、Error
- **`ContentDelta`** — 流式增量内容枚举（struct-like variants）：`Text { text: String }`、`Thinking { thinking: String }`、`ToolUseId { id: String }`、`ToolUseName { name: String }`、`ToolUseInputChunk { input: String }`

### 数据类型（Protocol 内部层）

- **`ProtocolId`** — 协议标识符（newtype wrapper `String`），实现 `Display`、`From<&str>`、`From<String>`、`Hash`、`Clone`
- **`InternalMessage`** — `InternalRequest` 中的单条消息（role + content）
- **`InternalRequest`** — Protocol 内部请求结构（model、messages、temperature、max_tokens、stream、extra_body）
- **`InternalResponse`** — Protocol 内部组装的原始响应（content_blocks + usage + finish_reason）
- **`RawContentBlock`** — Protocol 内部原始内容块（结构同 ContentBlock，但不对外暴露）
- **`RawUsage`** — Protocol 内部原始用量（prompt_tokens、completion_tokens、total_tokens）
- **`RawSseChunk`** — Protocol 内部 SSE 原始 chunk（event_type + data）
- **`SseStateMachine`** — SSE 流解析状态机（跟踪 current_block_index、block_type、pending_thinking、pending_signature）

### Provider trait

- **`Provider`**（trait）— LLM Provider 抽象，唯一职责是持有配置（URL、credentials、HTTP client）并执行实际的 HTTP 请求/响应周期。使用 `async_trait`，`Send + Sync`。配置访问器（`id`、`base_url`、`api_key`、`supported_protocols`、`http_client`、`default_headers`）均为同步；`send` 和 `send_streaming` 为异步
- **`ProviderError`** — Provider 层错误：`Reqwest(reqwest::Error)`，HTTP 请求失败（网络错误、TLS 错误、超时、重定向限制、非成功状态码等）
- **`SseStream`** — 流式响应通道类型，`tokio::sync::mpsc::Receiver<RawSseChunk>`，调用方通过 `recv().await` 逐块消费 SSE 原始 chunk

### ChatProtocol trait

- **`ChatProtocol`**（trait）— 请求/响应协议转换 trait，负责在 internal unified types 和具体 LLM API 协议（OpenAI、Anthropic、GLM 等）的 wire format 之间互转。使用 `async_trait`，`Send + Sync`。标识方法（`protocol_id`、`path`）同步；`build_request`、`parse_response`、`decorate_headers` 同步（纯序列化/header 操作）；`create_sse_machine` 是工厂方法；`parse_sse_stream` 异步 streaming
- **`ProtocolError`** — Protocol 层错误：`RequestBuild`、`ResponseParse`、`HeaderDecorate`、`SseParse`
- **`IncomingSseStream`** — 入站 SSE 流类型，`Pin<Box<dyn Stream<Item = RawSseChunk> + Send>>`
- **`OutgoingEventStream`** — 出站事件流类型，`Pin<Box<dyn Stream<Item = Result<StreamEvent, ProtocolError>> + Send>>`

### ModelInterpreter trait

- **`ModelInterpreter`**（trait）— Provider-specific response normalisation。方法：`name()`、`interpret_response(InternalResponse) -> UnifiedResponse`、`interpret_stream_event(StreamEvent) -> Option<StreamEvent>`、`inject_extra_body(&mut InternalRequest)`（默认空实现）。所有实现必须 `Send + Sync`
- **`DefaultInterpreter`** — identity 转换 fallback：`RawContentBlock` → `ContentBlock`、`RawUsage` → `UnifiedUsage`
- **`InterpreterRegistry`** — 按 glob 模式（`provider/*`、`provider/model`）匹配 provider/model → `ModelInterpreter`；未匹配时返回 `DefaultInterpreter`
- **`MinimaxInterpreter`** — 处理 `reasoning_content` → `Thinking` block 映射（content 为空时使用 reasoning_content）
- **`GlmInterpreter`** — 同 Minimax 逻辑 + reasoning_content 阈值判断（len > 10 bytes 才生成 Thinking block，否则降级为 Text）
- **`DeepSeekInterpreter`** — 直接使用 DefaultInterpreter 逻辑（OpenAI 兼容）

### ModelPlugin trait

- **`ModelPlugin`**（trait）— 请求/响应拦截 hook surface。方法：`name()`、`before_request(&mut InternalRequest)`（默认空实现）、`after_response(&mut UnifiedResponse)`（默认空实现）、`on_stream_event(&StreamEvent) -> Option<StreamEvent>`（默认转发所有事件）
- **`PluginPipeline`** — 顺序执行 zero 或 more plugins。`before_request`/`after_response` 总是执行所有 plugin；`on_stream_event` 支持短路（返回 `None` 时后续 plugin 不再收到该事件）

### UnifiedChatClient

- **`UnifiedChatClient`** — 统一入口，组装完整调用链：Provider + ChatProtocol + InterpreterRegistry + PluginPipeline。方法：`chat(InternalRequest) -> Result<UnifiedResponse>`、`chat_streaming(InternalRequest) -> Result<OutgoingEventStream>`
- **`ClientError`** — Client 层错误枚举：`Provider(provider::ProviderError)`、`Protocol(ProtocolError)`

### 具体 Protocol 实现

- **`OpenAiProtocol`** — OpenAI 兼容协议，用于 OpenAI、MiniMax、VolcEngine、DeepSeek。Bearer token 认证，SSE 解析 `choices[0].delta` 格式，文本流式输出 `delta.content`；流式 tool_calls 解析 `delta.tool_calls` 数组，产出完整工具调用事件序列（`BlockStart(ToolUse)` + `ToolUseId` + `ToolUseName` + `ToolUseInputChunk` + `BlockEnd(ToolUse)`）。Block 转换时（Text/Thinking → ToolUse）自动结束前一 block；`finish_reason: "tool_calls"` 触发 `BlockEnd(ToolUse)` + `MessageEnd { finish_reason: "tool_calls" }`。使用 `next_block_index` 计数器保证多 block 场景下 index 唯一递增。
- **`GlmProtocol`** — 智谱 GLM 系列协议。Bearer token 认证，`parse_response` 优先 `content` 兜底 `reasoning_content`，SSE 解析 `reasoning_content` 优先 `content`；流式 tool_calls 解析 `delta.tool_calls` 数组（路径同 OpenAI，`choices[0].delta.tool_calls`），Block 转换时自动结束前一 block；`finish_reason: "tool_calls"` 触发 `BlockEnd(ToolUse)` + `MessageEnd { finish_reason: "tool_calls" }`。注：当前 GLM 实现使用固定 block index（所有 block 均用 index 0），与 OpenAiProtocol 的递增 index 策略不同，混用时需注意。
- **`AnthropicProtocol`** — Anthropic `/v1/messages` stub。`x-api-key` + `anthropic-version` header，`parse_response` 解析 `content[].text` 数组，SSE 流式暂未实现

### 数据类型（模型元数据）

- **`InputType`** — 模型支持的输入模态：Text（纯文本）、Image（多模态图文）
- **`ModelInfo`** — 模型元数据（id、name、context_window、max_tokens、default_temperature、reasoning、input_types），可从 `"provider/model_id"` 字符串解析（实现 `FromStr`）
- **`ParseModelInfoError`** — `ModelInfo` 解析错误（格式非法时返回）
- **`ReasoningLevels`** — 模型支持的思考强度级别：None（不支持）、Toggle { on }（GLM 开关式）、Levels { off, base, reasoner }（DeepSeek 多档）
- **`ModelRecommendParams`** — 知识库中单模型的推荐参数（context_window、max_tokens、default_temperature、reasoning、reasoning_levels、input_types、recommended_protocol）；`recommended_protocol` 为该模型推荐的协议 ID，未知时返回默认 `"openai"`
- **`ProviderModelKnowledge`** — 内嵌知识库，按 provider 存储多模型推荐参数；支持 `find(provider, model_id)` 查询、`all_models(provider)` 列表、`recommended_protocol(provider_id, model_id) -> ProtocolId` 查询（未知时 fallback `"openai"`）

### 数据类型（Provider 注册与管理）

- **`LLMRegistry`** — Provider 注册中心，按名字查找和分发
- **`LLMProvider`**（trait）— Provider 接口，抽象所有 LLM 调用
- **`FallbackClient`** — 包装 `LLMRegistry`，在多个 model 间做 fallback + 重试
- **`ModelEntry`** — fallback chain 中的单个模型条目（provider + model）
- **`FakeProvider`** — 测试用可控响应 Provider（通过 Builder 编排场景，FIFO 依次消耗）
- **`CapturedRequest`** — `FakeProvider` 捕获的请求记录（model、messages）
- **`Builder`** — `FakeProvider` 的 Builder，支持 `.then_ok()`、`.then_err()`、`.then_delay()`、`.or_else()`、`.stub()` 等 API

### 构造

- **`LegacyProviderAdapter::new`** — 构造桥接器，将旧 `LLMProvider` 适配到新 `Provider` trait；参数包括 inner provider、base_url、api_key、supported_protocols、http_client、default_headers
- **`LegacySessionAdapter::from_legacy_messages`** — 从旧 `Vec<Message>` 构造新 `ChatSession` 适配器（模型名 + 消息历史）
- **`LLMRegistry::new`** — 创建空注册中心

### 配置

- **`LLMRegistry::register** — 注册一个 Provider 实例
- **`FallbackClient::with_timeout`** — 设置单次 LLM 调用超时秒数
- **`LLMProvider::new`** — 构造 Provider 实例（MimMax、OpenAI、Anthropic、Stub 各有）
- **`OpenAIProvider::new_with_base_url(api_key: String, base_url: &str)`** — 以自定义 base URL 构造 OpenAI Provider 实例（用于测试环境注入 mock server）
- **`GlmProvider::new`** — 构造 GlmProvider 实例（使用默认 GLM API URL）
- **`GlmProvider::with_base_url`** — 以自定义 base URL 构造 GlmProvider 实例（用于测试环境注入 mock server）
- **`FakeProvider::new`** — 构造无场景的 `FakeProvider`（首次调用必然 panic，用于严格测试）
- **`FakeProvider::builder`** — 构造 `Builder`，开始编排 `FakeProvider` 场景
- **`FallbackClient::new`** — 同步构造（加载持久化 cooldown）
- **`FallbackClient::new_async`** — 异步构造
- **`FallbackClient::from_strings`** — 从 `"provider/model"` 字符串列表构造

- **`CacheEntry`** — 缓存条目（fetched_at + ttl_secs + models），含 `is_expired()` 判断过期
- **`CacheKey`** — 缓存 key 辅助工具，`token_prefix()` 截取 token 前 4 位，`compute()` 计算 SHA256 hex key
- **`ModelCache`** — 模型列表缓存管理器，按 (provider, token) 查缓存，TTL 3600 秒；文件不存在或损坏时静默返回 None

### 配置

- **`LLMRegistry::register** — 注册一个 Provider 实例
- **`FallbackClient::with_timeout`** — 设置单次 LLM 调用超时秒数

### 主操作

- **`LLMProvider::chat** — 发起一次聊天补全请求（各 Provider 实现）
- **`LLMProvider::chat_streaming`** — 发起流式聊天补全请求，返回 `StreamingResponse`，调用方逐块消费 `ChatStreamChunk`；默认实现将 `chat()` 结果包装为单块流。MiniMax 和 GLM Provider override 此方法，使用独立 SSE 流式实现
- **`FallbackClient::chat** — 带重试 + fallback 的聊天请求，自动处理冷却和模型切换

### 查询

- **`LLMRegistry::get** — 按名字查找已注册的 Provider
- **`LLMRegistry::list** — 列出所有已注册 Provider 名字
- **`LLMProvider::models** — 返回该 Provider 支持的模型列表（各 Provider 内部 hardcode 返回）
- **`LLMProvider::name** — 返回 Provider 名称
- **`LLMProvider::is_stub** — 返回该 Provider 是否为 stub（默认 false）
- **`LLMProvider::provider_display_name`** — 返回人类可读的 Provider 显示名称（如 "VolcEngine"、"DeepSeek"），默认返回 `name()`
- **`LLMProvider::fetch_model_list`** — 从 Provider API 获取可用模型列表，返回 `Vec<ModelInfo>`；默认返回 `ModelNotFound` 表示不支持动态发现；各 Provider 可 override：MiniMax/GLM 通过知识库补充 reasoning 标记，VolcEngine/DeepSeek 从 `/models` API 解析
- **`LLMError::kind** — 将错误分类为 ErrorKind
- **`GlmProvider::fetch_usage`** — 查询 GLM Usage/Quota API，返回 `GlmQuotaResponse`（limits、usage、remaining 等配额信息）；`fetch_usage` 接收的 `base_url` 应为 GLM API 根 URL（如 `https://open.bigmodel.cn/api`），方法内部追加 `/paas/quota`
- **`GlmProvider::models** — 返回该 Provider 支持的模型列表
- **`GlmProvider::name** — 返回 Provider 名称
- **`FakeProvider::captured_requests`** — 返回所有已捕获请求（不消费）
- **`FakeProvider::drain_requests`** — 移除并返回所有已捕获请求
- **`FakeProvider::clear_requests`** — 清空已捕获请求

### 重试与冷却

- **`CooldownManager::is_in_cooldown** — 检查指定 (provider, model) 是否处于冷却中
- **`CooldownManager::record_failure** — 记录一次失败，触发冷却计时
- **`CooldownManager::record_success** — 清除指定 (provider, model) 的冷却记录
- **`CooldownManager::load** — 异步加载持久化冷却状态
- **`CooldownManager::load_sync** — 同步加载持久化冷却状态（用于启动阶段）
- **`backoff_delay`** — 计算带确定性 jitter 的指数退避延迟

### 常量

- **`MAX_TRANSIENT_RETRIES`** — Transient 错误最大重试次数（3）
- **`MAX_UNKNOWN_RETRIES`** — Unknown 错误最大重试次数（1）
- **`TRANSIENT_BASE_DELAY`** — 60 秒
- **`TRANSIENT_MAX_DELAY`** — 1 小时
- **`BILLING_MAX_DELAY`** — 24 小时

---

## 架构 / 结构

### 子模块划分

| 文件 | 职责 |
|------|------|
| `mod.rs` | 类型定义（Message、ChatRequest、ErrorKind 等）、LLMRegistry、LLMProvider trait、re-export 所有 Provider |
| `adapter/mod.rs` | Adapter 模块入口：re-export `LegacyProviderAdapter` 和 `LegacySessionAdapter` |
| `adapter/legacy_provider.rs` | `LegacyProviderAdapter<P>`：包装实现旧 `LLMProvider` 的具体 Provider，桥接到新 `Provider` trait；`send()` 将 `InternalRequest` → `ChatRequest` → 调用 `inner.chat()` → `ChatResponse` → `InternalResponse`；`send_streaming()` 同理通过 `inner.chat_streaming()` 收集 SSE chunk |
| `adapter/legacy_session.rs` | `LegacySessionAdapter`：包装旧 `Vec<Message>` 为新 `ChatSession` trait；`from_legacy_messages` 将 `Vec<Message>` 转为 `Vec<SessionMessage>`；提供 `append_response`、`append_tool_result`、`build_api_request` 方法 |
| `model_cache.rs` | 本地模型列表缓存：按 (provider, token) 查询，`CacheKey` 计算 key，`ModelCache` 读写 `~/.closeclaw/model_cache.json`（`MODEL_CACHE_FILE` 环境变量可覆盖），TTL 3600 秒，过期/损坏时静默返回 None |
| `model_info.rs` | 模型元数据类型：`InputType`（Text/Image 模态枚举）、`ModelInfo`（模型元数据 struct，含 `FromStr` 从 `"provider/model_id"` 解析）、`ParseModelInfoError` |
| `knowledge.rs` | 内嵌知识库：`ReasoningLevels`（思考强度枚举）、`ModelRecommendParams`（推荐参数）、`ProviderModelKnowledge`（知识库，含 `find` 和 `all_models` 查询接口）；覆盖 MiniMax、GLM、VolcEngine、DeepSeek 四个 Provider |
| `minimax.rs` | MiniMax Chat Completions API adapter。推理模型（M2.5/M2.7）用户可见回复在 `reasoning_content` 字段，`content` 为空时做兜底提取；业务错误码通过 `base_resp.status_code` 返回（非零即失败），区别于 HTTP 状态码；`completion_tokens_details.reasoning_tokens` 在内部解析（unit test 覆盖），暂未通过 `Usage` 暴露给调用方。 |
| `glm/mod.rs` | 智谱 GLM 系列模型（glm-5.1、glm-4.7、glm-4.5-air 等）adapter。错误格式为 top-level `error`（code 为字符串），code "1211" → ModelNotFound、"1214" → InvalidRequest；推理模型 `content` 为空时提取 `reasoning_content`；`usage` 中含 `prompt_tokens_details.cached_tokens` 和 `completion_tokens_details.reasoning_tokens`。非流式 chat、流式 streaming、Usage API 均通过 mockito Mock Server 覆盖完整 HTTP 全链路（`glm/tests/mock_integration.rs` 13 个非流式用例 + `glm/tests/mock_extra.rs` 3 个非流式用例含错误场景 + `glm_stream/tests/mock_integration.rs` 3 个流式用例 + `glm/tests/mock_usage.rs` 3 个 Usage 用例）。 |
| `minimax_stream.rs` | MiniMax 流式接口：SSE 解析、delta 提取、流式错误处理 |
| `glm_stream.rs` | GLM 流式接口：SSE 解析、delta 提取（`reasoning_content` 优先、`content` 兜底）、流式错误处理；`GlmProvider::chat_streaming()` override 实现 |
| `openai.rs` | OpenAI Chat Completions API adapter |
| `anthropic.rs` | Anthropic API adapter（当前为 stub） |
| `provider.rs` | Provider trait：持有配置（URL、credentials、HTTP client），执行 HTTP 请求/响应周期；配置访问器同步，`send`/`send_streaming` 异步；`ProviderError`（Reqwest）；`SseStream`（mpsc channel） |
| `protocol.rs` | ChatProtocol trait：请求/响应协议转换，负责在 internal unified types 和具体 LLM API wire format 之间互转；标识方法同步，`build_request`/`parse_response`/`decorate_headers` 同步，`parse_sse_stream` 异步 streaming；`ProtocolError`、`IncomingSseStream`、`OutgoingEventStream` |
| `stub.rs` | 测试用固定响应 Provider |
| `volcengine.rs` | VolcEngine（火山方舟）Chat Completions API adapter。`provider_display_name` 返回 "VolcEngine"；`fetch_model_list` GET `/models`（火山方舟格式），按 `domain=="LLM"` 且 `status` 非 Shutdown/Retiring 过滤，`reasoning` 保守设为 false。 |
| `deepseek.rs` | DeepSeek Chat Completions API adapter。`provider_display_name` 返回 "DeepSeek"；`fetch_model_list` GET `/models`（OpenAI 兼容格式），按 `status` 非 deprecated/shutdown 过滤，`reasoning` 保守设为 false。 |
| `fake.rs` | `FakeProvider`：场景编排 Provider，支持 Ok/Err/Delay 场景、FIFO 消耗、请求捕获（feature `fake-llm`） |
| `fallback.rs` | FallbackClient：两层重试（内层同模型指数退避、外层模型切换） |
| `retry.rs` | CooldownManager：按 (provider, model) 分组的冷却持久化；backoff_delay 计算 |
| `protocol/mod.rs` | Protocol 模块入口：re-export trait 和三个具体协议实现 |
| `protocol/openai.rs` | `OpenAiProtocol`：OpenAI 兼容协议（OpenAI、MiniMax、VolcEngine、DeepSeek 共用），`build_request` 生成 OpenAI Chat Completions JSON，`parse_response` 解析 choices[0].message.content，`decorate_headers` 用 `Authorization: Bearer`，SSE 解析 `choices[0].delta.content` 和 `delta.tool_calls`（流式工具调用） |
| `protocol/glm.rs` | `GlmProtocol`：GLM 系列协议，请求格式同 OpenAI，`parse_response` 优先 `content` 兜底 `reasoning_content`，SSE 解析 `reasoning_content` 优先 `content`，同步处理 `delta.tool_calls`（流式工具调用） |
| `protocol/anthropic.rs` | `AnthropicProtocol`：Anthropic `/v1/messages` stub，`build_request` 生成 Anthropic 格式，`parse_response` 解析 `content[].text` 数组，`decorate_headers` 用 `x-api-key` + `anthropic-version`，SSE 暂未实现（stub） |
| `interpreter.rs` | `ModelInterpreter` trait + `DefaultInterpreter`/`MinimaxInterpreter`/`GlmInterpreter`/`DeepSeekInterpreter` 四种实现 + `InterpreterRegistry`（glob 模式匹配 provider/model → Interpreter） |
| `plugin.rs` | `ModelPlugin` trait + `PluginPipeline`（顺序执行 before_request/after_response/on_stream_event hooks，支持 on_stream_event 短路） |
| `client.rs` | `UnifiedChatClient`：组装 Provider + ChatProtocol + InterpreterRegistry + PluginPipeline，提供 `chat` 和 `chat_streaming` 两个统一入口 |

### Provider 错误映射

HTTP 状态码 → `LLMError` 变体：

| HTTP 状态码 | LLMError |
|------------|----------|
| 401 / 403 | `AuthFailed` |
| 404 | `ModelNotFound` |
| 429 | `RateLimitExceeded` |
| 400 / 422 | `InvalidRequest` |
| 500–504 | `ApiError`（kind = Transient） |
| 网络错误 | `NetworkError` |

### MiniMax 业务错误码

MiniMax API 通过响应体中的 `base_resp.status_code`（非 HTTP 状态码）返回业务错误，非零即失败。

| status_code | LLMError | 触发场景 |
|-------------|----------|---------|
| 1004 | `AuthFailed` | 认证失败（API Key 无效） |
| 2013 + 含 "unknown model" | `ModelNotFound` | 模型不存在 |
| 2013 + 其它 | `InvalidRequest` | 参数错误（如 messages 为空、缺少必填参数） |
| 其它非零 | `ApiError` | 其它业务错误 |

### GLM 业务错误码

GLM API 通过响应体中的 top-level `error` 字段（code 为字符串）返回业务错误。

| code | LLMError | 触发场景 |
|------|----------|---------|
| "1211" | `ModelNotFound` | 模型不存在 |
| "1214" | `InvalidRequest` | 参数错误（如 messages 为空） |
| 其它 | `ApiError` | 其它业务错误 |

### FallbackClient 两层错误处理

- **内层**（`chat_with_retry`）：仅对 `Transient` / `Unknown` 错误重试，耗尽后切模型
- **外层**（`chat`）：`InvalidRequest` / `Auth` / `Billing` 立即切模型；`Transient` / `Unknown` 重试耗尽后切模型；成功则清除 cooldown

### E2E 测试覆盖

| 测试 | 场景 | 验证点 |
|------|------|--------|
| `test_fallback_on_rate_limit` | primary Auth 失败 → fallback 成功 | fallback 链正确切换 |
| `test_success_then_fallback` | FakeProvider FIFO 场景依次消耗 | 顺序调用正确消费场景 |
| `test_delay_triggers_timeout` | Delay 场景触发超时 | FakeProvider delay 行为 |
| `test_registry_roundtrip` | LLMRegistry get → call | 注册中心查找和调用 |
| `test_cooldown_skip_after_auth_failure` | primary Auth 失败 → cooldown → 再次调用跳过冷却 provider | Auth 错误触发 1h cooldown，第二次调用跳过冷却 provider |
| `test_all_providers_exhausted` | 所有 provider 都失败 | 返回 `ApiError("all models in fallback chain exhausted")` |

### 冷却持久化

冷却状态以 JSON 写入 `~/.closeclaw/llm_cooldowns.json`（路径可由 `LLM_COOLDOWN_FILE` 环境变量覆盖）。每次 `record_failure` 更新文件，`load` 时过滤已过期的 entry。

### 流式响应架构

MiniMax 流式响应采用 SSE（Server-Sent Events）格式：
- 请求添加 `stream: true` 参数
- 响应体为 `data: {...}` 行序列，以 `data: [DONE]` 终止
- Delta chunk 中可见文本在 `delta.reasoning_content`（M2.5/M2.7 模型）或 `delta.content`；最终 chunk 含完整 `message`、`usage`、`base_resp`
- 实现使用 `tokio::sync::mpsc::channel` + 后台 task 读取 SSE，避免引入 `futures` 依赖
