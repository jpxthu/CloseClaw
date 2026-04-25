# LLM 模块

## 模块概述

LLM 模块为 CloseClaw 提供统一的多 Provider LLM 调用抽象。通过 `LLMProvider` trait + `LLMRegistry` 实现插件式架构，支持同时注册多个 Provider（MiniMax、OpenAI、Anthropic、Stub）。核心设计：Provider 只负责 HTTP 调用和响应解析，错误分类由 `ErrorKind` 统一处理；重试和 fallback 逻辑由 `FallbackClient` 包装，对调用方透明。

边界：模块不负责 prompt 工程、不做缓存、不做 token 估算（Usage 数字由 Provider 透传）。冷却状态通过 `CooldownManager` 持久化到 `~/.closeclaw/llm_cooldowns.json`。

---

## 公开接口

### 数据类型（核心类型）

- **`Message`** — LLM 对话消息（role + content）
- **`ChatRequest`** — 聊天补全请求（model、messages、temperature、max_tokens）
- **`ChatResponse`** — 聊天补全响应（content、model、usage，不含 cooldown_retry_after）
- **`Usage`** — token 用量统计（prompt_tokens、completion_tokens、total_tokens）
- **`LLMError`** — 错误枚举：AuthFailed、RateLimitExceeded、ModelNotFound、InvalidRequest、ApiError、NetworkError
- **`ErrorKind`** — 错误可恢复性分类：Transient、Auth、Billing、InvalidRequest、Unknown
- **`Scenario`** — `FakeProvider` 场景枚举：Ok（成功响应）、Err（错误响应）、Delay（sleep 后执行内部场景，内部场景可为 Ok/Err/另一个 Delay，嵌套递归解析）

### 数据类型（Provider 注册与管理）

- **`LLMRegistry`** — Provider 注册中心，按名字查找和分发
- **`LLMProvider`**（trait）— Provider 接口，抽象所有 LLM 调用
- **`FallbackClient`** — 包装 `LLMRegistry`，在多个 model 间做 fallback + 重试
- **`ModelEntry`** — fallback chain 中的单个模型条目（provider + model）
- **`FakeProvider`** — 测试用可控响应 Provider（通过 Builder 编排场景，FIFO 依次消耗）
- **`CapturedRequest`** — `FakeProvider` 捕获的请求记录（model、messages）
- **`Builder`** — `FakeProvider` 的 Builder，支持 `.then_ok()`、`.then_err()`、`.then_delay()`、`.or_else()`、`.stub()` 等 API

### 构造

- **`LLMRegistry::new`** — 创建空注册中心
- **`LLMProvider::new`** — 构造 Provider 实例（MimMax、OpenAI、Anthropic、Stub 各有）
- **`FakeProvider::new`** — 构造无场景的 `FakeProvider`（首次调用必然 panic，用于严格测试）
- **`FakeProvider::builder`** — 构造 `Builder`，开始编排 `FakeProvider` 场景
- **`FallbackClient::new`** — 同步构造（加载持久化 cooldown）
- **`FallbackClient::new_async`** — 异步构造
- **`FallbackClient::from_strings`** — 从 `"provider/model"` 字符串列表构造

### 配置

- **`LLMRegistry::register** — 注册一个 Provider 实例
- **`FallbackClient::with_timeout`** — 设置单次 LLM 调用超时秒数

### 主操作

- **`LLMProvider::chat** — 发起一次聊天补全请求（各 Provider 实现）
- **`FallbackClient::chat** — 带重试 + fallback 的聊天请求，自动处理冷却和模型切换

### 查询

- **`LLMRegistry::get** — 按名字查找已注册的 Provider
- **`LLMRegistry::list** — 列出所有已注册 Provider 名字
- **`LLMProvider::models** — 返回该 Provider 支持的模型列表
- **`LLMProvider::name** — 返回 Provider 名称
- **`LLMProvider::is_stub** — 返回该 Provider 是否为 stub（默认 false）
- **`LLMError::kind** — 将错误分类为 ErrorKind
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
| `minimax.rs` | MiniMax Chat Completions API adapter |
| `openai.rs` | OpenAI Chat Completions API adapter |
| `anthropic.rs` | Anthropic API adapter（当前为 stub） |
| `stub.rs` | 测试用固定响应 Provider |
| `fake.rs` | `FakeProvider`：场景编排 Provider，支持 Ok/Err/Delay 场景、FIFO 消耗、请求捕获（feature `fake-llm`） |
| `fallback.rs` | FallbackClient：两层重试（内层同模型指数退避、外层模型切换） |
| `retry.rs` | CooldownManager：按 (provider, model) 分组的冷却持久化；backoff_delay 计算 |

### Provider 错误映射

HTTP 状态码 → `LLMError` 变体：

| HTTP 状态码 | LLMError |
|------------|----------|
| 401 / 403 | `AuthFailed` |
| 429 | `RateLimitExceeded` |
| 400 / 422 | `InvalidRequest` |
| 500–504 | `ApiError`（kind = Transient） |
| 网络错误 | `NetworkError` |

### FallbackClient 两层错误处理

- **内层**（`chat_with_retry`）：仅对 `Transient` / `Unknown` 错误重试，耗尽后切模型
- **外层**（`chat`）：`InvalidRequest` / `Auth` / `Billing` 立即切模型；`Transient` / `Unknown` 重试耗尽后切模型；成功则清除 cooldown

### MiniMax Thinking Tag 剥离

MiniMax thinking 模型在响应 `content` 中嵌入 `<think>` ... `</think>` XML 标签包裹的思考内容。模块在返回响应前自动剥离这两个标签并 `trim()`，调用方拿到的 `content` 不含 thinking tag。

### 冷却持久化

冷却状态以 JSON 写入 `~/.closeclaw/llm_cooldowns.json`（路径可由 `LLM_COOLDOWN_FILE` 环境变量覆盖）。每次 `record_failure` 更新文件，`load` 时过滤已过期的 entry。
