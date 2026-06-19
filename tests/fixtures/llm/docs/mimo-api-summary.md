# MiMo API Summary

> 来源：[Xiaomi MiMo API 开放平台](https://mimo.mi.com/docs/) 官方文档  
> 整理日期：2026-06-16  
> 文档版本：Phase 0.1（官方文档核实 + 实时 API 调用验证版）

## 修订记录

| 版本 | 日期 | 修正内容 |
|------|------|---------|
| Phase 0.1 | 2026-06-16 | 初版；从 mimo.mi.com/docs 抓取全部相关文档 + 真实 API 验证 |

## 概述

Xiaomi MiMo 开放平台提供 OpenAI 兼容和 Anthropic 兼容两种 API 协议。  
模型分两类：
- **通用 API（Pay-as-you-go，按 Token 计费）**：Base URL `https://api.xiaomimimo.com/v1`，Key 格式 `sk-xxxxx`
- **Token Plan（订阅制，按 Credit 配额计费）**：Base URL `https://token-plan-cn.xiaomimimo.com/v1`，Key 格式 `tp-xxxxx`

**注意**：两类 Key 和 Base URL **不互通**，混用会返回 401。

## 目录

1. [认证与请求](#1-认证与请求)
2. [OpenAI 兼容端点](#2-openai-兼容端点)
3. [Anthropic 兼容端点](#3-anthropic-兼容端点)
4. [模型列表](#4-模型列表)
5. [Reasoning/Thinking 参数](#5-reasoningthinking-参数)
6. [Streaming 响应（SSE）](#6-streaming-响应sse)
7. [工具调用（Function Calling / Web Search）](#7-工具调用function-calling--web-search)
8. [多模态理解（图像/音频/视频）](#8-多模态理解图像音频视频)
9. [响应格式](#9-响应格式)
10. [错误响应](#10-错误响应)
11. [用量 / 配额查询](#11-用量--配额查询)
12. [Token Plan 余量查询](#12-token-plan-余量查询)
13. [速率限制](#13-速率限制)
14. [Temperature / Top P 超参数](#14-temperature--top-p-超参数)
15. [重要注意事项](#15-重要注意事项)
16. [文档 URL 索引](#16-文档-url-索引)

---

## 1. 认证与请求

### 1.1 认证方式（双协议一致）

API 同时支持两种 Header，任选其一：

| 方法 | Header 格式 |
|------|-------------|
| Method 1（推荐） | `api-key: $MIMO_API_KEY` |
| Method 2 | `Authorization: Bearer $MIMO_API_KEY` |

两种 Header 在 OpenAI 和 Anthropic 协议下都可用。

### 1.2 API Key 格式

| 用量类型 | Key 格式 | 前缀示例 | Base URL |
|---------|---------|---------|---------|
| Pay-as-you-go（按量付费） | `sk-xxxxx` | `sk-` | `https://api.xiaomimimo.com/v1`（OpenAI）<br>`https://api.xiaomimimo.com/anthropic`（Anthropic） |
| Token Plan（订阅配额） | `tp-xxxxx` | `tp-` | `https://token-plan-cn.xiaomimimo.com/v1`（OpenAI）<br>`https://token-plan-cn.xiaomimimo.com/anthropic`（Anthropic） |

**注意**：
- 国内/海外账户返回不同的 Base URL + Key，**不可互通**
- Key 在 [Console → API Keys](https://platform.xiaomimimo.com/#/console/api-keys) 创建
- Token Plan 的 Key 在 [Console → Subscription Management](https://platform.xiaomimimo.com/#/console/plan-manage) 获取
- 401 错误的常见原因：Key 错误、Header 格式错误、Token Plan 和 Pay-as-you-go Key 混用

### 1.3 请求头示例（OpenAI 协议）

```bash
curl -X POST 'https://api.xiaomimimo.com/v1/chat/completions' \
  -H 'api-key: sk-xxxxx' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "mimo-v2.5-pro",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_completion_tokens": 1024
  }'
```

来源：[First API Call](https://mimo.mi.com/static/docs/quick-start/summary/first-api-call.md)

---

## 2. OpenAI 兼容端点

### 2.1 Base URL 与端点

| 项目 | 值 |
|------|-----|
| Base URL | `https://api.xiaomimimo.com/v1`（PayG） / `https://token-plan-cn.xiaomimimo.com/v1`（Token Plan） |
| Chat Completions 端点 | `/v1/chat/completions`（完整 URL：`{base_url}/chat/completions`） |
| Models 列表端点 | `/v1/models`（完整 URL：`{base_url}/models`） |
| Method | `POST`（chat completions），`GET`（models） |

### 2.2 Models 列表响应（实测验证）

```bash
curl -H 'Authorization: Bearer <API_KEY>' {base_url}/models
```

```json
{
  "object": "list",
  "data": [
    {"id": "mimo-v2-omni", "object": "model", "owned_by": "xiaomi"},
    {"id": "mimo-v2-pro", "object": "model", "owned_by": "xiaomi"},
    {"id": "mimo-v2-tts", "object": "model", "owned_by": "xiaomi"},
    {"id": "mimo-v2.5", "object": "model", "owned_by": "xiaomi"},
    {"id": "mimo-v2.5-asr", "object": "model", "owned_by": "xiaomi"},
    {"id": "mimo-v2.5-pro", "object": "model", "owned_by": "xiaomi"},
    {"id": "mimo-v2.5-tts", "object": "model", "owned_by": "xiaomi"},
    {"id": "mimo-v2.5-tts-voiceclone", "object": "model", "owned_by": "xiaomi"},
    {"id": "mimo-v2.5-tts-voicedesign", "object": "model", "owned_by": "xiaomi"}
  ]
}
```

> 注意：`/v1/models` 是标准 OpenAI 端点，但官方文档没有显式声明（实测存在并返回模型列表）。  
> Anthropic 兼容端点 `/anthropic/v1/models` 实测 **404**。

### 2.3 Chat Completions 请求字段

> 完全兼容 OpenAI Chat Completions API 规范。

#### 必填字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `model` | string | 模型 ID（如 `mimo-v2.5-pro`） |
| `messages` | array | 消息列表，最少 1 条 |

#### 可选字段

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `max_completion_tokens` | integer | 见模型表 | 最大输出 token 数（含 reasoning tokens）。TTS 模型范围 `[1, 8192]`，其他 `[1, 131072]` |
| `temperature` | number | 见模型表 | 范围 `[0, 1.5]`，thinking 模式下 mimo-v2.5-pro/v2.5/v2-pro/v2-omni 强制 1.0 |
| `top_p` | number | `0.95` | 范围 `[0.01, 1.0]`，thinking 模式下 mimo-v2.5-pro/v2.5/v2-pro/v2-omni 强制 0.95 |
| `stream` | boolean | `false` | 是否启用 SSE 流式输出 |
| `thinking` | object | 见模型表 | 深度思考控制：`{"type": "enabled"}` 或 `"disabled"` |
| `stop` | string\|array\|null | `null` | 最多 4 个停止序列（TTS 模型不支持） |
| `tools` | array | - | 工具列表（TTS 模型不支持） |
| `tool_choice` | string | `"auto"` | **仅支持 `auto`**，其他值会被后端丢弃（按 auto 处理） |
| `frequency_penalty` | number | `0` | 范围 `[-2.0, 2.0]` |
| `presence_penalty` | number | `0` | 范围 `[-2.0, 2.0]` |
| `response_format` | object | - | `{"type": "text"}` 或 `{"type": "json_object"}`（TTS 模型不支持） |
| `audio` | object | - | 音频输出参数，仅 TTS 模型支持（见音频章节） |
| `n` | - | - | **不支持**（实测返回 400） |

#### 消息角色（messages）

支持的角色：`system` / `developer` / `user` / `assistant` / `tool`

`tool` 消息必填 `tool_call_id` 字段。

#### 工具字段（tools）

支持两种类型：
- `{"type": "function", "function": {...}}` — 标准 OpenAI function calling
- `{"type": "web_search", ...}` — 联网搜索（需先在 Console 激活插件）

function 字段定义：
- `name`（必填）：函数名，匹配 `a-zA-Z0-9_-`，最长 64 字符
- `description`（可选）
- `parameters`（可选）：JSON Schema 对象；省略则为空参数列表
- `strict`（可选，默认 `false`）：是否严格遵循 schema

来源：[OpenAI API Compatibility](https://mimo.mi.com/static/docs/api/chat/openai-api.md)

### 2.4 OpenAI 协议 cURL 示例

```bash
curl -X POST 'https://api.xiaomimimo.com/v1/chat/completions' \
  --header "api-key: $MIMO_API_KEY" \
  --header "Content-Type: application/json" \
  --data '{
    "model": "mimo-v2.5-pro",
    "messages": [
      {"role": "system", "content": "You are MiMo, an AI assistant developed by Xiaomi."},
      {"role": "user", "content": "please introduce yourself"}
    ],
    "max_completion_tokens": 1024,
    "temperature": 1.0,
    "top_p": 0.95,
    "stream": false,
    "stop": null,
    "frequency_penalty": 0,
    "presence_penalty": 0
  }'
```

---

## 3. Anthropic 兼容端点

### 3.1 Base URL 与端点

| 项目 | 值 |
|------|-----|
| Base URL | `https://api.xiaomimimo.com/anthropic`（PayG） / `https://token-plan-cn.xiaomimimo.com/anthropic`（Token Plan） |
| Messages 端点 | `/anthropic/v1/messages`（完整 URL：`{base_url}/v1/messages`） |
| Models 端点 | `/anthropic/v1/models`（实测 404，**不可用**） |
| Method | `POST` |

### 3.2 Messages 请求字段

> 完全兼容 Anthropic Messages API 规范。

#### 必填字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `model` | string | 模型 ID（仅支持 `mimo-v2.5-pro`、`mimo-v2.5`、`mimo-v2-pro`、`mimo-v2-omni`、`mimo-v2-flash`，**不支持 TTS/ASR 模型**） |
| `messages` | array | 消息列表，每个含 `role` + `content` |
| `max_tokens` | integer | 最大输出 token 数（**必填**，OpenAI 协议下是 optional） |

#### 可选字段

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `system` | string\|array | - | 系统提示（OpenAI 协议下放在 messages 里） |
| `stream` | boolean | `false` | 是否 SSE 流式输出 |
| `temperature` | number | 见模型表 | 范围 `[0, 1.5]`，thinking 模式下 `mimo-v2.5-pro/v2.5/v2-pro/v2-omni` 强制 1.0 |
| `top_p` | number | `0.95` | 范围 `[0.01, 1.0]`，thinking 模式下强制 0.95 |
| `thinking` | object | 见模型表 | `{"type": "enabled"}` 或 `"disabled"` |
| `stop_sequences` | array | - | 停止序列 |
| `tools` | array | - | 工具定义（input_schema 格式，与 Anthropic 一致） |
| `tool_choice` | object | - | `{"type": "auto", "disable_parallel_tool_use": false}` — **仅支持 `auto`**，其他值会被后端丢弃 |

#### 消息 content blocks

支持：
- `{"type": "text", "text": "..."}`
- `{"type": "image", "source": {...}}`（仅 `mimo-v2.5` / `mimo-v2-omni`）
- `{"type": "tool_use", "id": "...", "name": "...", "input": {...}}`
- `{"type": "tool_result", "tool_use_id": "...", "content": "...", "is_error": false}`
- `{"type": "thinking", "thinking": "...", "signature": "..."}`

#### Tools 字段

```json
{
  "name": "get_weather",
  "description": "...",
  "type": "custom",
  "input_schema": {
    "type": "object",
    "properties": {...},
    "required": [...]
  }
}
```

来源：[Anthropic API Compatibility](https://mimo.mi.com/static/docs/api/chat/anthropic-api.md)

### 3.3 Anthropic 协议 cURL 示例

```bash
curl -X POST 'https://api.xiaomimimo.com/anthropic/v1/messages' \
  --header "api-key: $MIMO_API_KEY" \
  --header "Content-Type: application/json" \
  --data '{
    "model": "mimo-v2.5-pro",
    "max_tokens": 1024,
    "system": "You are MiMo, an AI assistant developed by Xiaomi.",
    "messages": [
      {"role": "user", "content": [{"type": "text", "text": "please introduce yourself"}]}
    ],
    "top_p": 0.95,
    "stream": false,
    "temperature": 1.0,
    "stop_sequences": null
  }'
```

---

## 4. 模型列表

### 4.1 文本生成模型

| 系列 | Model ID | 能力 | 上下文窗口 | 最大输出 | RPM | TPM |
|------|----------|------|-----------|---------|-----|-----|
| Pro 系列 | `mimo-v2.5-pro` | 文本生成、深度思考、流式输出、Function Call、Structured Output、Web Search | 1M | 128K | 100 | 10M |
| Pro 系列 | `mimo-v2-pro` | 同上 | 1M | 128K | 100 | 10M |
| Omni 系列 | `mimo-v2.5` | 文本生成、**全模态理解**、深度思考、流式输出、Function Call、Structured Output、Web Search | 1M | 128K | 100 | 10M |
| Omni 系列 | `mimo-v2-omni` | 同上 | 256K | 128K | 100 | 10M |
| Flash 系列 | `mimo-v2-flash` | 文本生成、深度思考、流式输出、Function Call、Structured Output、Web Search | 256K | 64K | 100 | 10M |

### 4.2 语音模型

| 类型 | Model ID | 能力 | 上下文窗口 | 最大输出 | RPM | TPM |
|------|----------|------|-----------|---------|-----|-----|
| ASR | `mimo-v2.5-asr` | 语音识别（中英文 + 多种方言：吴、粤、闽南、四川） | 8K | 2K | 100 | 10K |
| TTS | `mimo-v2.5-tts` | 语音合成（多音色） | 8K | 8K | 100 | 10M |
| TTS | `mimo-v2.5-tts-voiceclone` | 语音合成 + **音色克隆**（上传 mp3/wav 样本） | 8K | 8K | 100 | 10M |
| TTS | `mimo-v2.5-tts-voicedesign` | 语音合成 + **音色设计**（一句话定义音色） | 8K | 8K | 100 | 10M |
| TTS | `mimo-v2-tts` | 语音合成（V2 系列，免费限期） | 8K | 8K | 100 | 10M |

> **重要弃用公告**：V2 系列将在 **2026-06-30 00:00 (GMT+8)** 正式下线：
> - `mimo-v2-pro` / `mimo-v2-omni`：2026-06-01 起自动转发到 V2.5，按 V2.5 价格计费
> - `mimo-v2-flash` / `mimo-v2-tts`：2026-06-18 00:00 起自动转发到 V2.5，按 V2.5 价格计费
> - TTS 系列在 Token Plan 下 **限期免费**，不消耗 Credits

### 4.3 选型建议

| 场景 | 推荐模型 |
|------|---------|
| 复杂推理、深度分析、长文档处理 | `mimo-v2.5-pro` |
| 图像、音频、视频理解 | `mimo-v2.5` 或 `mimo-v2-omni` |
| 低成本、快速响应 | `mimo-v2-flash` |
| 语音转文字（支持中英文） | `mimo-v2.5-asr` |
| 文字转语音（标准预设音色） | `mimo-v2.5-tts` |
| 声音克隆（上传样本音频） | `mimo-v2.5-tts-voiceclone` |
| 自定义音色设计 | `mimo-v2.5-tts-voicedesign` |

来源：[Models](https://mimo.mi.com/static/docs/quick-start/summary/model.md)

---

## 5. Reasoning/Thinking 参数

### 5.1 Thinking 参数结构

OpenAI 协议（通过 `extra_body` 传入，因为 `thinking` 不是标准 OpenAI 参数）：
```json
{
  "thinking": {
    "type": "enabled"
  }
}
```

Anthropic 协议（顶层字段）：
```json
{
  "thinking": {
    "type": "enabled"
  }
}
```

可选值：`"enabled"` / `"disabled"`

### 5.2 默认值（按模型）

| 模型 | 默认 thinking |
|------|---------------|
| `mimo-v2.5-pro`、`mimo-v2.5`、`mimo-v2-pro`、`mimo-v2-omni` | `enabled` |
| `mimo-v2-flash` | `disabled` |
| TTS/ASR 模型 | **不支持** |

### 5.3 Thinking 模式下的限制

**强制覆盖超参数**：在 thinking 模式下，`mimo-v2.5-pro`、`mimo-v2.5`、`mimo-v2-pro`、`mimo-v2-omni` **不支持自定义** `temperature` 和 `top_p`。即使传入，也会被强制设为推荐默认值 `1.0` 和 `0.95`。

**`max_completion_tokens` 限制的是 thinking + answer 的总长度**。如果思考过程很长，最终答案的 token 空间会相应减少。

### 5.4 响应字段（reasoning_content / thinking）

**OpenAI 协议 — 非流式**：
```json
{
  "choices": [{
    "message": {
      "role": "assistant",
      "content": "最终回答内容",
      "reasoning_content": "思考过程内容..."
    }
  }]
}
```

**OpenAI 协议 — 流式**：先输出多个 `delta.reasoning_content` 增量块，再输出 `delta.content` 增量块。

**Anthropic 协议 — 非流式**：
```json
{
  "content": [
    {"type": "thinking", "thinking": "思考过程...", "signature": "..."},
    {"type": "text", "text": "最终回答..."}
  ]
}
```

**Anthropic 协议 — 流式**：通过 `content_block_start` + `content_block_delta` (type: `thinking_delta` / `text_delta`) 事件顺序输出。

**Usage 字段**：`completion_tokens_details.reasoning_tokens` 报告推理 token 数。

### 5.5 多轮 Tool Calls 在 Thinking 模式下（关键）

> ⚠️ **强制要求**：Agent 产品在 thinking 模式下的多轮 tool calls 中，所有历史 assistant 消息**必须完整传回 `reasoning_content` 字段**，否则返回 **400 错误**。
>
> 同时传回所有历史 `reasoning_content` 可以保留推理上下文，避免指令跟随下降和幻觉增加。

**OpenAI 协议**：把上轮 `assistant_message`（含 `reasoning_content` + `tool_calls`）整条追加到 `messages` 数组。

**Anthropic 协议**：把 `{"type": "thinking", "thinking": "...", "signature": "..."}` content block 完整追加到对应 assistant 消息的 content 数组。

**受影响的 Agent 产品**：

| 协议 | Agent 产品 |
|------|-----------|
| OpenAI 兼容 | TRAE、Cursor、Roo Code、Codex、GitHub Copilot CLI、Zed、AutoGen、Goose |
| Anthropic 兼容 | TRAE、GitHub Copilot CLI、AutoGen、Goose、OpenClaw、OpenCode、Kilo Code |

来源：[Deep Thinking](https://mimo.mi.com/static/docs/quick-start/usage-guide/text-generation/deep-thinking.md)

### 5.6 是否支持 `reasoning_effort`

**未找到**。MiMo 使用 `thinking.type`（enabled/disabled 二元开关），**没有 Anthropic 那样的 `thinking.budget_tokens` 或 OpenAI 风格的 `reasoning_effort` 三档分级**。

---

## 6. Streaming 响应（SSE）

### 6.1 OpenAI 协议 SSE 格式

OpenAI 标准 SSE：每个 chunk 以 `data: ` 前缀，最后一个 chunk 是 `data: [DONE]`。

**非 thinking 模式 chunk**：
```json
data: {"id":"...","choices":[{"delta":{"content":"Hello","role":"assistant"},"finish_reason":null,"index":0}],"created":...,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}
```

**thinking 模式 chunk 序列**（实测）：
1. 首个 chunk：`delta.content=""`, `delta.reasoning_content=null`
2. 多个 chunk：`delta.reasoning_content="..."` 增量（思考过程）
3. 切换点：`delta.content="# Tips..."` 开头
4. 多个 chunk：`delta.content="..."` 增量（最终回答）
5. 结束 chunk：`finish_reason="stop"`, `delta.content=null`
6. 最终 chunk：`choices=[]`, `usage={...}`
7. `data: [DONE]`

**Stream chunk 关键字段**：
- `choices[].delta.content` — 回答增量
- `choices[].delta.reasoning_content` — 推理增量（thinking 模式）
- `choices[].delta.role` — 仅首块出现
- `choices[].delta.tool_calls` — 工具调用增量
- `choices[].delta.audio` — 音频输出增量（TTS 模型）
- `choices[].finish_reason` — 终止原因
- `usage` — 仅最后一或两个 chunk 出现，含完整 token 统计

### 6.2 Anthropic 协议 SSE 格式

Anthropic 标准 SSE 事件：

| 事件类型 | 说明 |
|---------|------|
| `message_start` | 消息开始，含初始 message metadata |
| `content_block_start` | content block 开始（含 type: text/thinking/tool_use） |
| `content_block_delta` | content block 增量，含 `text_delta` / `thinking_delta` / `input_json_delta` |
| `content_block_stop` | content block 结束 |
| `message_delta` | message 级元数据更新（stop_reason 等） |
| `message_stop` | 消息结束 |
| `ping` | 心跳保活 |
| `error` | 错误事件 |

每个事件以 `event: <type>\ndata: <json>\n\n` 形式分隔。

来源：[Anthropic API Compatibility](https://mimo.mi.com/static/docs/api/chat/anthropic-api.md)

### 6.3 finish_reason / stop_reason

| 值 | 含义 |
|----|------|
| `stop` (OpenAI) / `end_turn` (Anthropic) | 自然停止点 |
| `length` (OpenAI) / `max_tokens` (Anthropic) | 达到 max_tokens |
| `tool_calls` (OpenAI) / `tool_use` (Anthropic) | 模型调用工具 |
| `content_filter` | 内容审查过滤 |
| `repetition_truncation` | 模型检测到重复 |

---

## 7. 工具调用（Function Calling / Web Search）

### 7.1 Function Calling

**OpenAI 协议**：标准格式，`tools: [{"type": "function", "function": {...}}]`，模型返回 `tool_calls`，客户端执行后用 `role: "tool", tool_call_id: "..."` 传回结果。

**Anthropic 协议**：标准格式，`tools: [{"name": "...", "description": "...", "input_schema": {...}}]`，模型返回 `tool_use` content block，客户端用 `tool_result` 传回结果。

### 7.2 Parallel Tool Calls

- OpenAI 协议：默认启用 parallel tool calls（`tool_choice` 仅支持 auto）
- Anthropic 协议：通过 `tool_choice.disable_parallel_tool_use` 控制（默认 `false` = 启用并行）

### 7.3 Web Search 工具

**仅 OpenAI 协议支持**（Anthropic 协议下 web_search tool 不可用）。  
需先在 [Console → Plugin Management](https://platform.xiaomimimo.com/#/console/plugin) 激活 Web Search 插件（独立计费，详见定价章节）。

**工具定义**：
```json
{
  "type": "web_search",
  "force_search": false,
  "max_keyword": 5,
  "limit": 5,
  "user_location": {
    "type": "approximate",
    "country": "China",
    "region": "Hubei",
    "city": "Wuhan"
  }
}
```

| 字段 | 默认值 | 说明 |
|------|-------|------|
| `type` | 必填 | `"web_search"` |
| `force_search` | `false` | `true` = 强制搜索；`false` = 模型自主判断 |
| `max_keyword` | `5` | 单次搜索最大关键词数，范围 `[1, 50]` |
| `limit` | `5` | 单次搜索最大返回结果数，范围 `[1, 50]` |
| `user_location.type` | 必填 | `"approximate"` |
| `user_location.country` | 可选 | ISO 国家代码或名称 |
| `user_location.region` | 可选 | 地区/省 |
| `user_location.city` | 可选 | 城市 |
| `user_location.district` | 可选 | 区/县 |
| `user_location.longitude/latitude` | 可选 | 经纬度 |

**响应字段**：`message.annotations` 数组返回所有引用 URL 的 metadata（`logo_url`, `publish_time`, `site_name`, `summary`, `title`, `type`, `url`），`message.error_message` 返回搜索错误信息。

**Usage 统计**：`usage.web_search_usage = {tool_usage, page_usage}`。

**流式特性**：搜索源在第一个 packet 中全部返回。

来源：[Web Search](https://mimo.mi.com/static/docs/quick-start/usage-guide/text-generation/tool-calling/web-search.md)

### 7.4 工具调用 + Thinking 联合示例

OpenAI 协议下，client 端多轮循环伪代码：

```python
response = client.chat.completions.create(
    model="mimo-v2.5-pro",
    messages=messages,  # 包含所有历史的 assistant_message（带 reasoning_content + tool_calls）
    tools=tools,
    extra_body={"thinking": {"type": "enabled"}}
)

assistant_message = response.choices[0].message
messages.append(assistant_message)  # 完整保存，包括 reasoning_content

if assistant_message.tool_calls:
    for tc in assistant_message.tool_calls:
        result = execute_tool(tc)
        messages.append({"role": "tool", "tool_call_id": tc.id, "content": result})
    # 再次请求，让模型基于工具结果生成最终答案
```

---

## 8. 多模态理解（图像/音频/视频）

### 8.1 支持的模型

| 模型 | 图像 | 音频 | 视频 |
|------|------|------|------|
| `mimo-v2.5` | ✅ | ✅ | ✅ |
| `mimo-v2-omni` | ✅ | ✅ | ✅ |
| 其他文本模型 | ❌ | ❌ | ❌ |

### 8.2 OpenAI 协议 — 图像输入

支持两种方式：
1. **URL**：`{"type": "image_url", "image_url": {"url": "https://..."}}`，URL 必须可公开访问，单文件 ≤ 50MB
2. **Base64**：`{"type": "image_url", "image_url": {"url": "data:image/png;base64,..."}}`

Anthropic 协议用 `{"type": "image", "source": {"type": "base64"|"url", ...}}` 格式。

### 8.3 视频输入

OpenAI 协议：
```json
{
  "type": "video_url",
  "video_url": {"url": "https://..."},
  "fps": 2,
  "media_resolution": "default"
}
```

- `fps`：每秒采样帧数，范围 `[0.1, 10.0]`，默认 `2`
- `media_resolution`：`"default"` 或 `"max"`，默认 `"default"`

### 8.4 音频输入

OpenAI 协议：
```json
{
  "type": "input_audio",
  "input_audio": {"data": "base64..."}
}
```

### 8.5 Usage 中的多媒体 token 统计

```json
"usage": {
  "prompt_tokens_details": {
    "cached_tokens": 1081,
    "image_tokens": 1024,
    "audio_tokens": 0,
    "video_tokens": 0
  }
}
```

### 8.6 ASR 模型调用

ASR（`mimo-v2.5-asr`）通过标准 chat completions endpoint 调用，messages 中传入音频 input（URL 或 base64）。  
按输入音频时长计费（精确到秒，转换为小时）。

来源：[Image Understanding](https://mimo.mi.com/static/docs/quick-start/usage-guide/multimodal-understanding/image-understanding.md)

---

## 9. 响应格式

### 9.1 OpenAI 协议 — 非流式响应

```json
{
  "id": "2b92b0964c9b4335bffad7c2f75cfe9e",
  "choices": [
    {
      "finish_reason": "stop",
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Machine learning is...",
        "reasoning_content": "The user wants...",  // thinking 模式才有
        "tool_calls": null,                        // 调用工具时有值
        "annotations": [...],                      // web_search 时有值
        "audio": {                                 // TTS 时有值
          "id": "...",
          "data": "base64...",
          "expires_at": null,
          "transcript": null
        },
        "final_text_preview": "..."                // TTS optimize_text_preview=true 时
      }
    }
  ],
  "created": 1781233054,
  "model": "mimo-v2.5-pro",
  "object": "chat.completion",
  "usage": {
    "completion_tokens": 171,
    "prompt_tokens": 60,
    "total_tokens": 231,
    "completion_tokens_details": {
      "reasoning_tokens": 110
    },
    "prompt_tokens_details": {
      "cached_tokens": 0,
      "audio_tokens": 0,
      "image_tokens": 0,
      "video_tokens": 0
    },
    "web_search_usage": {                         // web_search 时有值
      "tool_usage": 1,
      "page_usage": 5
    }
  }
}
```

### 9.2 Anthropic 协议 — 非流式响应

```json
{
  "id": "msg_...",
  "type": "message",
  "role": "assistant",
  "content": [
    {"type": "thinking", "thinking": "...", "signature": "..."},
    {"type": "text", "text": "..."},
    {"type": "tool_use", "id": "...", "name": "get_weather", "input": {...}}
  ],
  "model": "mimo-v2.5-pro",
  "stop_reason": "end_turn",
  "usage": {
    "input_tokens": 100,
    "output_tokens": 50,
    "cache_read_input_tokens": 80
  }
}
```

---

## 10. 错误响应

### 10.1 实测错误格式（OpenAI 协议）

```json
{
  "error": {
    "message": "Invalid API Key",
    "param": "Please provide valid API Key",
    "code": "401",
    "type": "invalid_key"
  }
}
```

**格式特征**：标准的 OpenAI `{ "error": { "message", "type", "code", "param" } }` 格式。

### 10.2 错误码清单

| HTTP Code | 含义 | 触发场景 | 解决方法 |
|-----------|------|---------|---------|
| **400** | Invalid Format | JSON 格式错、缺参数、参数越界、模型不存在、多模态格式不合规、多轮 thinking 模式下漏传 `reasoning_content` | 检查请求参数 |
| **401** | Authentication Fails | Key 缺失/无效、Header 格式错误、Token Plan 和 PayG Key 混用 | 检查 Key + Header + Base URL 对应 |
| **402** | Insufficient Balance | 账户余额不足 | 充值 |
| **403** | Forbidden Access | 地区不可用 / API Key 被风控 | 创建新 Key，避免敏感输入 |
| **404** | Not Found | 模型/端点不支持图像输入 | 改用多模态模型 |
| **421** | Content Filter | 内容被审查阻断 | 修改输入 |
| **429** | Too Many Requests | RPM/TPM 超限、Token Plan 配额耗尽 | 指数退避重试；升级 Token Plan 或切换 PayG |
| **500** | Server Error | 服务端故障 | 重试或联系客服 |
| **503** | Server Overloaded | 服务器过载 | 稍后重试 |

来源：[Error Codes](https://mimo.mi.com/static/docs/api/guidance/error-codes.md)

---

## 11. 用量 / 配额查询

### 11.1 用量查询

**官方方式**：在 [Console → Usage Information](https://platform.xiaomimimo.com/#/console/usage) 查看和导出按日期的 Token 用量和请求数。

> **未找到 API 端点**：MiMo 平台**没有公开的 balance/quota/usage REST API**。所有用量查询都必须通过 Console Web 界面进行。
>
> 对比：
> - OpenAI 提供 `/v1/usage` 端点 ❌ MiMo 没有
> - Anthropic 提供 `/v1/organizations/usage` ❌ MiMo 没有
> - Anthropic 提供 `/v1/organizations/balance` ❌ MiMo 没有
> - DeepSeek 提供 `/user/balance` ❌ MiMo 没有

### 11.2 余额提醒（仅 Web）

- **余额提醒**：[Account Balance 页面](https://platform.xiaomimimo.com/#/console/balance) 可开启余额提醒，余额低于阈值时通过短信/邮件通知
- **Token Plan 配额提醒**：当用量达到 50%、90%、100% 时，会收到短信和邮件提醒

### 11.3 每次响应的 Usage 字段

OpenAI 协议：响应中含 `usage` 字段，包含 `prompt_tokens`、`completion_tokens`、`total_tokens`、cache/audio/image/video 详情、reasoning_tokens、web_search_usage。

Anthropic 协议：响应中含 `usage` 字段，包含 `input_tokens`、`output_tokens`、`cache_read_input_tokens`。

---

## 12. Token Plan 余量查询

### 12.1 套餐结构

**月套餐**：

| 套餐 | 价格 | 月度 Credit 配额 |
|------|------|----------------|
| Lite | $6 / ¥39 | 4.1B |
| Standard | $16 / ¥99 | 11B |
| Pro | $50 / ¥329 | 38B |
| Max | $100 / ¥659 | 82B |

**年套餐**：年付 88% 月付总价（即月套餐 × 10.56 倍年付）。  
**首购折扣**：12% off（仅一次）；**年付连续订阅**：额外 12% off；**夜间折扣**：北京时间 0:00-8:00（即 UTC 16:00-24:00）系数 0.8x。

### 12.2 Credit 消耗规则

**语言模型**（按 Token 计）：

| 模型 | Input (Cache Hit) | Input (Cache Miss) | Output |
|------|-------------------|--------------------|--------|
| `mimo-v2.5-pro` | 2.5 Credits | 300 Credits | 600 Credits |
| `mimo-v2.5` | 2 Credits | 100 Credits | 200 Credits |
| `mimo-v2-pro` | 2.5 Credits | 300 Credits | 600 Credits |
| `mimo-v2-omni` | 2 Credits | 100 Credits | 200 Credits |

**ASR 模型**：`mimo-v2.5-asr` = 30M Credits / 小时音频。

**TTS 模型**：限期免费，不消耗 Credits。

> **配额共享**：可用模型按不同比例**并行消耗**同一个 Credit 池，**不是独立计数**。

### 12.3 余量查询方式

> ⚠️ **没有公开 API 端点查询 Token Plan 配额余额**。

仅可通过 [Console → Subscription Management](https://platform.xiaomimimo.com/#/console/plan-manage) 查看：
- 当前套餐剩余 Credit
- 用量进度条
- 包年包月到期时间
- 升级/续费/自动续费开关

**配额耗尽行为**：Credit 耗尽 → **服务停止**，不会消耗账户余额或其他奖励。需升级套餐或切换到 PayG。

来源：[Token Plan](https://mimo.mi.com/static/docs/price/token-plan.md), [FAQ - Token Plan](https://mimo.mi.com/static/docs/quick-start/faq/token-plan.md)

---

## 13. 速率限制

| 模型 | RPM | TPM |
|------|-----|-----|
| `mimo-v2.5-pro` | 100 | 10M |
| `mimo-v2-pro` | 100 | 10M |
| `mimo-v2.5` | 100 | 10M |
| `mimo-v2-omni` | 100 | 10M |
| `mimo-v2-flash` | 100 | 10M |
| `mimo-v2.5-asr` | 100 | 10K |
| `mimo-v2.5-tts` | 100 | 10M |
| `mimo-v2.5-tts-voiceclone` | 100 | 10M |
| `mimo-v2.5-tts-voicedesign` | 100 | 10M |
| `mimo-v2-tts` | 100 | 10M |

**计费范围**：
- RPM：单个账号下所有 API Key 调用**同一模型**的总请求数 / 分钟
- TPM：单个账号下所有 API Key 调用**同一模型**的总 Token 数 / 分钟

来源：[Rate Limit](https://mimo.mi.com/static/docs/api/guidance/rate-limit.md)

---

## 14. Temperature / Top P 超参数

| 模型 | temperature 默认 | temperature 范围 | top_p 默认 | top_p 范围 |
|------|-----------------|----------------|-----------|----------|
| `mimo-v2.5-pro` / `mimo-v2-pro` | 1.0 | [0, 1.5] | 0.95 | [0.01, 1.0] |
| `mimo-v2.5` / `mimo-v2-omni` | 1.0 | [0, 1.5] | 0.95 | [0.01, 1.0] |
| TTS 模型（`mimo-v2.5-tts` 等） | 0.6 | [0, 1.5] | 0.95 | [0.01, 1.0] |
| `mimo-v2-flash` | 0.3 | [0, 1.5] | 0.95 | [0.01, 1.0] |

**`mimo-v2-flash` 任务推荐值**：

| 任务类型 | temperature | top_p |
|---------|------------|-------|
| Vibe Coding | 0.3 | 0.95 |
| Function Call | 0.3 | 0.95 |
| General Conversation | 0.8 | 0.95 |
| Creative Writing | 0.8 | 0.95 |
| WebDev | 0.8 | 0.95 |
| Mathematical Reasoning | 1.0 | 0.95 |

**`mimo-v2.5-pro` / `mimo-v2.5` / `mimo-v2-pro` / `mimo-v2-omni`**：以上所有任务都用 `temperature=1.0`, `top_p=0.95`。

来源：[Model Hyperparameters](https://mimo.mi.com/static/docs/api/guidance/model-hyperparameters.md)

---

## 15. 重要注意事项

### 15.1 认证相关

1. **Token Plan 和 PayG 的 Key、Base URL 必须配套使用**，混用返回 401
2. 国内和海外账号返回不同的 Base URL + Key，**不可互通**
3. Key 在 [Console](https://platform.xiaomimimo.com/) 创建/管理；删除 Key 后不能再调用，历史消费记录仍可查

### 15.2 请求相关

1. **Thinking 模式下**：mimo-v2.5-pro/v2.5/v2-pro/v2-omni 的 `temperature` 和 `top_p` 被强制设为 1.0 和 0.95
2. **多轮 Tool Calls + Thinking**：必须**完整传回**所有历史的 `reasoning_content`（OpenAI）或 thinking block（Anthropic），否则 400
3. **`max_completion_tokens`** 限制 thinking + answer 的总 token 数
4. **`tool_choice`** 仅支持 `auto`，传其他值会被后端丢弃（按 auto 处理）
5. **`stop` 参数**：OpenAI 最多 4 个停止序列，TTS 模型不支持

### 15.3 TTS 音频输出参数（OpenAI 协议）

仅 TTS 模型支持：

```json
{
  "audio": {
    "voice": "mimo_default",          // 或内置音色名（见下）
    "format": "wav",                  // wav | mp3 | pcm | pcm16（pcm/pcm16 等价）
    "optimize_text_preview": false    // 是否让模型自动优化播报文本（仅 voicedesign 支持）
  }
}
```

**内置 voice 选项**：

| 模型 | 可用 voices |
|------|------------|
| `mimo-v2-tts` | `mimo_default`, `default_en`, `default_zh` |
| `mimo-v2.5-tts` | `mimo_default`, `冰糖`, `茉莉`, `苏打`, `白桦`, `Mia`, `Chloe`, `Milo`, `Dean` |
| `mimo-v2.5-tts-voiceclone` | 仅支持传入 mp3/wav 音频样本的 base64 编码 |
| `mimo-v2.5-tts-voicedesign` | **不支持 voice 字段** |

**TTS 调用规范**：
- messages 中必须包含 role 为 `assistant` 的消息，其 content 指定要合成的文本
- 使用 `mimo-v2.5-tts-voicedesign` 时，**额外要求** messages 中有 role 为 `user` 的消息（除非 `optimize_text_preview=true`）
- 非流式时 `format` 默认 `wav`；流式时默认 `pcm`

### 15.4 TTS 流式输出（OpenAI 协议）

流式时 `delta.audio` 包含 base64 编码的音频分片，`object="chat.completion.chunk"`。  
非流式时 `message.audio.data` 包含完整 base64 音频。

### 15.5 国内/海外差异

| 项目 | 国内 | 海外 |
|------|------|------|
| 支付 | 微信支付、支付宝、小米支付 | Waffo 网关（美元结算） |
| Base URL + Key | 中国区独立 | 海外独立 |
| 发票 | 支持电子发票（个人/企业） | 不支持开票 |
| 实名认证 | 需要 | 不需要 |

### 15.6 弃用计划

| 模型 | 自动转发到 V2.5 时间 | 完全下线时间 |
|------|---------------------|------------|
| `mimo-v2-pro` | 2026-06-01 00:00 GMT+8 | 2026-06-30 00:00 GMT+8 |
| `mimo-v2-omni` | 2026-06-01 00:00 GMT+8 | 2026-06-30 00:00 GMT+8 |
| `mimo-v2-flash` | 2026-06-18 00:00 GMT+8 | 2026-06-30 00:00 GMT+8 |
| `mimo-v2-tts` | 2026-06-18 00:00 GMT+8 | 2026-06-30 00:00 GMT+8 |

### 15.7 Token Plan 使用范围限制

Token Plan 包内 Credit **只能在编程工具中使用**（如 OpenClaw、OpenCode、Claude Code 等），**禁止**用于：
- 自动化脚本
- 自定义应用后端的非编程场景 API 调用

违反会被暂停服务、封禁 Key。

---

## 16. 文档 URL 索引

所有 markdown 文档原始 URL（用于核对和重抓）：

### 快速开始
- https://mimo.mi.com/static/docs/quick-start/summary/welcome.md
- https://mimo.mi.com/static/docs/quick-start/summary/first-api-call.md
- https://mimo.mi.com/static/docs/quick-start/summary/model.md

### 使用指南
- https://mimo.mi.com/static/docs/quick-start/usage-guide/text-generation/deep-thinking.md
- https://mimo.mi.com/static/docs/quick-start/usage-guide/text-generation/tool-calling/web-search.md
- https://mimo.mi.com/static/docs/quick-start/usage-guide/multimodal-understanding/image-understanding.md
- https://mimo.mi.com/static/docs/quick-start/usage-guide/multimodal-understanding/audio-understanding.md
- https://mimo.mi.com/static/docs/quick-start/usage-guide/multimodal-understanding/video-understanding.md
- https://mimo.mi.com/static/docs/quick-start/usage-guide/audio/Speech-Recognition.md
- https://mimo.mi.com/static/docs/quick-start/usage-guide/audio/speech-synthesis-v2.5.md
- https://mimo.mi.com/static/docs/quick-start/usage-guide/audio/speech-synthesis.md

### API 参考
- https://mimo.mi.com/static/docs/api/guidance/rate-limit.md
- https://mimo.mi.com/static/docs/api/guidance/model-hyperparameters.md
- https://mimo.mi.com/static/docs/api/guidance/error-codes.md
- https://mimo.mi.com/static/docs/api/chat/openai-api.md
- https://mimo.mi.com/static/docs/api/chat/anthropic-api.md
- https://mimo.mi.com/static/docs/api/audio/Speech-Recognition.md

### 定价
- https://mimo.mi.com/static/docs/price/pay-as-you-go.md
- https://mimo.mi.com/static/docs/price/token-plan.md

### FAQ
- https://mimo.mi.com/static/docs/quick-start/faq/account.md
- https://mimo.mi.com/static/docs/quick-start/faq/payment.md
- https://mimo.mi.com/static/docs/quick-start/faq/api-integration.md
- https://mimo.mi.com/static/docs/quick-start/faq/token-plan.md
- https://mimo.mi.com/static/docs/quick-start/faq/others.md

### 更新日志
- https://mimo.mi.com/static/docs/updates/model.md
- https://mimo.mi.com/static/docs/updates/feature.md

### 入口
- 文档主页：https://mimo.mi.com/docs/
- llms.txt 索引：https://mimo.mi.com/llms.txt
- 完整文档：https://mimo.mi.com/llms-full.txt
- Console：https://platform.xiaomimimo.com/

---

## 附录：研究方法说明

1. **数据来源**：所有内容均抓取自 https://mimo.mi.com/docs 官方文档（路径通过 `/llms.txt` 索引发现）
2. **API 验证**：使用 Token Plan key 对 `https://token-plan-cn.xiaomimimo.com` 进行了实测：
   - OpenAI 协议 chat completions：✅ 成功（验证了 reasoning_content、usage 字段）
   - Anthropic 协议 messages：✅ 成功（验证了 thinking content block、usage 字段）
   - `/v1/models` 端点：✅ 成功（返回 9 个模型）
   - `/anthropic/v1/models` 端点：❌ 404（不存在）
3. **未找到的内容**（已明确标注）：
   - 用量查询 API 端点（只有 Console Web）
   - 余额/配额查询 API 端点（只有 Console Web）
   - Token Plan 余额查询 API 端点
   - `reasoning_effort` 三档分级参数（仅有 thinking.enabled/disabled 二元开关）
