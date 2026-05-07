# MiniMax LLM API 总结

> 信息来源：https://platform.minimaxi.com/docs/ 下所有官方文档
> 整理日期：2026-05-06
> 注：所有引用均标注具体文档 URL

---

## 目录

1. [认证与请求基础](#1-认证与请求基础)
2. [Chat Completions（OpenAI 协议）](#2-chat-completionsopenai-协议)
3. [Messages（Anthropic 协议）](#3-messagesanthropic-协议)
4. [Reasoning / Thinking 参数](#4-reasoning--thinking-参数)
5. [响应格式](#5-响应格式)
6. [Streaming 响应](#6-streaming-响应)
7. [错误响应](#7-错误响应)
8. [Cache 机制](#8-cache-机制)
9. [多轮对话](#9-多轮对话)
10. [厂商特有字段](#10-厂商特有字段)
11. [工具调用（Tool Use）](#11-工具调用tool-use)

---

## 1. 认证与请求基础

### Base URL

| 协议 | Base URL | 文档来源 |
|------|----------|---------|
| OpenAI 兼容 | `https://api.minimaxi.com` | [text-chat-openai](https://platform.minimaxi.com/docs/api-reference/text-chat-openai.md) |
| Anthropic 兼容 | `https://api.minimaxi.com` | [text-chat-anthropic](https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md) |
| 国际用户（语音等） | `https://api.minimax.io` | [text-prompt-caching](https://platform.minimaxi.com/docs/api-reference/text-prompt-caching.md) |

> 原文：
> - OpenAI: `servers: - url: https://api.minimaxi.com`
> - Anthropic: `servers: - url: https://api.minimaxi.com`
> - 国内用户使用 `https://api.minimaxi.com/v1`，国际用户使用 `https://api.minimax.io/v1`

### 认证方式

- **类型**: HTTP Bearer Auth，JWT 格式
- **Header**: `Authorization: Bearer ${API_KEY}`
- **API Key 获取**: [账户管理 > 接口密钥](https://platform.minimaxi.com/user-center/basic-information/interface-key)

> 原文（[text-chat-openai](https://platform.minimaxi.com/docs/api-reference/text-chat-openai.md)）：
> ```
> bearerFormat: JWT
> description: HTTP: Bearer Auth - HTTP Authorization Scheme: Bearer API_key，用于验证账户信息
> ```

### 核心端点汇总

| 协议 | 端点 | Method |
|------|------|--------|
| OpenAI Chat Completions | `/v1/chat/completions` | POST |
| Anthropic Messages | `/anthropic/v1/messages` | POST |
| OpenAI List Models | `/v1/models` | GET |
| Anthropic List Models | `/anthropic/v1/models` | GET |

---

## 2. Chat Completions（OpenAI 协议）

> 来源：[text-chat-openai.md](https://platform.minimaxi.com/docs/api-reference/text-chat-openai.md)

### Endpoint

```
POST https://api.minimaxi.com/v1/chat/completions
```

### 必填字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `model` | string | 模型 ID |
| `messages` | array | 对话历史消息列表 |

> 原文：`required: - model - messages`

### 可选字段

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `stream` | boolean | `false` | 是否流式传输 |
| `max_completion_tokens` | integer | — | 生成内容上限（Token），上限 2048 |
| `temperature` | double | `1.0` | 温度系数，影响随机性，取值范围 (0, 1] |
| `top_p` | double | `0.95` | 采样策略，取值范围 (0, 1] |

> 原文：
> ```
> max_completion_tokens: description: 指定生成内容长度的上限（Token 数），上限为 2048
> temperature: default: 1.0, maximum: 1
> top_p: default: 0.95
> ```

### 支持的模型（OpenAI 协议）

| 模型 ID | 输入输出总 Token | 输出速度 | 文档来源 |
|---------|----------------|---------|---------|
| `MiniMax-M2.7` | 204800 | ~60 tps | [api-overview](https://platform.minimaxi.com/docs/api-reference/api-overview.md) |
| `MiniMax-M2.7-highspeed` | 204800 | ~100 tps | 同上 |
| `MiniMax-M2.5` | 204800 | ~60 tps | 同上 |
| `MiniMax-M2.5-highspeed` | 204800 | ~100 tps | 同上 |
| `MiniMax-M2.1` | 204800 | ~60 tps | 同上 |
| `MiniMax-M2.1-highspeed` | 204800 | ~100 tps | 同上 |
| `MiniMax-M2` | 204800 | — | 同上 |

> 注：text-chat-openai.md 中 `model` 字段 enum 仅列出 4 个值（MiniMax-M2.7/M2.7-highspeed/M2.5/M2.1），其余模型通过 api-overview 文档说明可用。

> 原文（[api-overview](https://platform.minimaxi.com/docs/api-reference/api-overview.md)）：
> ```
> MiniMax-M2.7: 输入输出总 token: 204800, 输出速度约60tps
> MiniMax-M2.7-highspeed: 输入输出总 token: 204800, 输出速度约100tps
> ```

### Messages 角色类型（OpenAI 协议）

| role 值 | 说明 |
|---------|------|
| `system` | 设定模型的角色和行为 |
| `user` | 用户的输入 |
| `assistant` | 模型的历史回复 |
| `user_system` | 设定用户的角色和人设 |
| `group` | 对话的名称 |
| `sample_message_user` | 示例的用户输入 |
| `sample_message_ai` | 示例的模型输出 |

> 原文：`enum: - system - user - assistant - user_system - group - sample_message_user - sample_message_ai`

---

## 3. Messages（Anthropic 协议）

> 来源：[text-chat-anthropic.md](https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md)

### Endpoint

```
POST https://api.minimaxi.com/anthropic/v1/messages
```

### 必填字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `model` | string | 模型 ID |
| `messages` | array | 对话历史消息列表 |

> 原文：`required: - model - messages`

### 可选字段

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `system` | string 或 array | — | 设定模型的角色和行为，可为纯文本或内容块数组 |
| `stream` | boolean | `false` | 是否流式传输 |
| `max_tokens` | integer | — | 生成内容上限（Token），上限 2048 |
| `temperature` | double | `1.0` | 温度系数，取值范围 (0, 1] |
| `top_p` | double | `0.95` | 采样策略，取值范围 (0, 1] |

> 原文：
> ```
> max_tokens: description: 指定生成内容长度的上限（Token 数），上限为 2048
> temperature: default: 1.0
> top_p: default: 0.95
> ```

### 支持的模型（Anthropic 协议）

| 模型 ID | 输入输出总 Token | 输出速度 | 文档来源 |
|---------|----------------|---------|---------|
| `MiniMax-M2.7` | 204800 | ~60 tps | [api-overview](https://platform.minimaxi.com/docs/api-reference/api-overview.md) |
| `MiniMax-M2.7-highspeed` | 204800 | ~100 tps | 同上 |
| `MiniMax-M2.5` | 204800 | ~60 tps | 同上 |
| `MiniMax-M2.5-highspeed` | 204800 | ~100 tps | 同上 |
| `MiniMax-M2.1` | 204800 | ~60 tps | 同上 |
| `MiniMax-M2.1-highspeed` | 204800 | ~100 tps | 同上 |
| `MiniMax-M2` | 204800 | — | 同上 |

> 注：text-chat-anthropic.md 中 `model` 字段 enum 仅列出 4 个值（MiniMax-M2.7/M2.7-highspeed/M2.5/M2.1），其余模型通过 api-overview 文档说明可用。

> 原文：`enum: - MiniMax-M2.7 - MiniMax-M2.7-highspeed - MiniMax-M2.5 - MiniMax-M2.1`

### Messages 角色类型（Anthropic 协议）

| role 值 | 说明 |
|---------|------|
| `user` | 用户的输入 |
| `assistant` | 模型的历史回复 |
| `user_system` | 设定用户的角色和人设 |
| `group` | 对话的名称 |
| `sample_message_user` | 示例的用户输入 |
| `sample_message_ai` | 示例的模型输出 |

> 注意：Anthropic 协议 **不支持** `system` 作为 messages 中的 role；system 应通过独立的 `system` 参数传递

### Anthropic Content Block 类型

| type | 说明 |
|------|------|
| `text` | 文本内容 |
| `thinking` | 模型思考过程（含 signature） |

> 原文：`enum: - text - thinking`

---

## 4. Reasoning / Thinking 参数

> 来源：[text-chat-anthropic.md](https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md)、[text-prompt-caching](https://platform.minimaxi.com/docs/api-reference/text-prompt-caching.md)

### Anthropic 协议 — Thinking 内容

Anthropic 协议的响应 content 数组中，模型思考过程以独立 content block 形式返回：

```json
{
  "type": "thinking",
  "thinking": "用户的思考内容...",
  "signature": "ce704495524bad054531fe187e18b4a8d874a52fbb3923ce18fceace5e768ec9"
}
```

> 原文（[text-chat-anthropic](https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md)）：
> ```
> content:
>   - thinking: 用户用中文说"你好"，这是一个简单的问候。我应该用中文友好地回应。
>     signature: ce704495524bad054531fe187e18b4a8d874a52fbb3923ce18fceace5e768ec9
>     type: thinking
>   - text: 你好！有什么我可以帮助你的吗？
>     type: text
> ```

### OpenAI 协议 — Thinking/Reasoning 分离

OpenAI 协议中思考内容的处理方式：

- **参数**: `extra_body.reasoning_split`（布尔值）

> 原文（[text-chat-openai.md](https://platform.minimaxi.com/docs/api-reference/text-chat-openai.md)）：
> `reasoning_split: description: 将思考内容分离到 reasoning_details 字段`

- `reasoning_split=True` 时，思考内容分离到 `reasoning_details` 字段（已确认存在）
- `reasoning_split` **默认值未在文档中明确说明**（[已搜索以下文档，均未找到](#已搜索的文档)）

**`reasoning_details` 响应结构已确认**：

```json
"reasoning_details": [
  {
    "type": "reasoning.text",
    "id": "reasoning-text-1",
    "format": "MiniMax-response-v1",
    "index": 0,
    "text": "思考内容全文..."
  }
]
```


> 原文（[text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)）：
> `"reasoning_details": [{"type": "reasoning.text", "id": "reasoning-text-1", "format": "MiniMax-response-v1", "index": 0, "text": "Let me think about this request..."}]`

### 思考相关字段汇总

| 字段 | 协议 | 位置 | 说明 |
|------|------|------|------|
| `thinking` | Anthropic | content[].thinking | 思考内容文本 |
| `signature` | Anthropic | content[].signature | 思考内容签名 |
| `reasoning_split` | OpenAI | extra.body | [待确认] 默认值；开启后思考分离到 reasoning_details |
| `reasoning_details` | OpenAI | response | 已确认：数组，内含 `type`/`id`/`format`/`index`/`text` 子字段 |

---

## 5. 响应格式

### 5.1 OpenAI 兼容响应

> 来源：[text-chat-openai.md](https://platform.minimaxi.com/docs/api-reference/text-chat-openai.md)

#### 非流式响应（chat.completion）

```json
{
  "id": "06379c22ee61299eeadfb33e3b3e9102",
  "object": "chat.completion",
  "created": 1776838788,
  "model": "MiniMax-M2.7",
  "choices": [{
    "index": 0,
    "message": {
      "role": "assistant",
      "content": "回复内容"
    },
    "finish_reason": "stop"
  }],
  "usage": {
    "total_tokens": 43,
    "prompt_tokens": 22,
    "completion_tokens": 21,
    "prompt_tokens_details": {
      "cached_tokens": 0
    }
  },
  "input_sensitive": false,
  "output_sensitive": false,
  "base_resp": {
    "status_code": 0,
    "status_msg": "success"
  }
}
```

> 注：文档示例中还包含 `total_characters: 0`、`input_sensitive_type: 0`、`output_sensitive_type: 0`、`output_sensitive_int: 0` 等字段，这些未在 schema 定义中列出，schema 与示例不完全一致。

#### usage.prompt_tokens_details.cached_tokens

> 原文（[text-prompt-caching](https://platform.minimaxi.com/docs/api-reference/text-prompt-caching.md)）：
> ```json
> "prompt_tokens_details": {
>   "cached_tokens": 800
> }
> ```

#### 敏感词字段（非标准 OpenAI 字段）

| 字段 | 类型 | 说明 |
|------|------|------|
| `input_sensitive` | boolean | 输入内容是否命中敏感词 |
| `input_sensitive_type` | integer | 命中类型：1严重违规 2色情 3广告 4违禁 5谩骂 6暴恐 7其他 |
| `output_sensitive` | boolean | 输出内容是否命中敏感词 |
| `output_sensitive_type` | integer | 同上 |

> 原文：`input_sensitive_type: description: 取值为以下其一：1 严重违规；2 色情；3 广告；4 违禁；5 谩骂；6 暴恐；7 其他`

### 5.2 Anthropic 兼容响应

> 来源：[text-chat-anthropic.md](https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md)

```json
{
  "id": "06379dbe27b33d7c58d8410a8efe6394",
  "type": "message",
  "role": "assistant",
  "model": "MiniMax-M2.7",
  "content": [
    {
      "type": "thinking",
      "thinking": "思考内容",
      "signature": "ce704495..."
    },
    {
      "type": "text",
      "text": "回复内容"
    }
  ],
  "stop_reason": "end_turn",
  "usage": {
    "input_tokens": 42,
    "output_tokens": 30
  }
}
```

#### stop_reason 值

| 值 | 说明 |
|----|------|
| `end_turn` | 模型自然结束 |
| `max_tokens` | 达到 max_tokens 上限 |
| `stop_sequence` | 命中停止序列 |

> 原文：`enum: - end_turn - max_tokens - stop_sequence`

#### Anthropic usage 字段（OpenAI 兼容字段对照）

| Anthropic 字段 | OpenAI 字段 | 说明 |
|---------------|------------|------|
| `input_tokens` | `prompt_tokens` | 输入消耗的 Token 数 |
| `output_tokens` | `completion_tokens` | 输出消耗的 Token 数 |
| `cache_read_input_tokens` | `prompt_tokens_details.cached_tokens` | 命中缓存的 Token 数 |
| `cache_creation_input_tokens` | — | 创建新缓存的 Token 数 |

> 原文（[text-prompt-caching](https://platform.minimaxi.com/docs/api-reference/text-prompt-caching.md)）：
> ```
> cache_creation_input_tokens: 创建新缓存条目时写入缓存的 token 数量
> cache_read_input_tokens: 此请求从缓存中检索的 token 数量
> input_tokens: 未从缓存读取或用于创建缓存的输入 token 数量
> ```

---

## 6. Streaming 响应

### 6.1 OpenAI 协议 Streaming

> 来源：[text-chat-openai.md](https://platform.minimaxi.com/docs/api-reference/text-chat-openai.md)

#### SSE event: `text/event-stream`

```json
{
  "id": "06379c22ee61299eeadfb33e3b3e9102",
  "choices": [{
    "index": 0,
    "delta": {
      "content": "增量文本",
      "role": "assistant"
    },
    "finish_reason": null
  }],
  "created": 1776838946,
  "model": "MiniMax-M2.7",
  "object": "chat.completion.chunk"
}
```

- **`finish_reason`**: 最后一个 chunk 时为 `"stop"` 或 `"length"`，过程中为 `null`
- **`role`**: 每个 chunk 的 delta 中**可能**包含 `"role": "assistant"`（文档示例中每个 chunk 都包含，但 schema 中 role 为 assistant 非必填，建议按实际行为处理）
- **`usage`**: 仅在最后一个 chunk 中返回
- **额外字段**: 文档示例中每个 chunk 还包含 `name: "MiniMax AI"`、`audio_content: ''`、`input_sensitive: false`、`output_sensitive: false` 等字段，未在 schema 定义中列出，schema 与示例不完全一致。

#### Streaming chunk 序列示例

```
chunk 1: delta.content = "你好"（role=assistant）
chunk 2: delta.content = "！"（finish_reason=null）
chunk N: finish_reason="stop"
```

### 6.2 Anthropic 协议 Streaming

> 来源：[text-chat-anthropic.md](https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md)

#### SSE event types

| type | 说明 | 包含内容 |
|------|------|---------|
| `message_start` | 消息开始 | 完整消息元数据（id, type, role, content=[], model, stop_reason=null） |
| `ping` | 心跳事件 | — |
| `content_block_start` | 内容块开始 | index + content_block（type=text/thinking） |
| `content_block_delta` | 内容块增量 | index + delta（type=text_delta/thinking_delta + 对应内容） |
| `content_block_stop` | 内容块结束 | index |
| `message_delta` | 消息级增量 | delta.stop_reason + usage |
| `message_stop` | 消息结束 | — |

#### Streaming 完整事件序列示例

```
1. message_start  → 包含完整的 message 对象（usage.input_tokens=0, output_tokens=0）
2. ping
3. content_block_start  → type=thinking
4. content_block_delta  → type=thinking_delta, thinking="用户"
5. content_block_delta → thinking="用中文说..."
6. content_block_delta → type=signature_delta, signature="..."
7. content_block_stop
8. content_block_start → type=text
9. content_block_delta → type=text_delta, text="你好！"
10. content_block_stop
11. message_delta → stop_reason=end_turn, usage={input_tokens, output_tokens}
12. message_stop
```

> 原文：
> ```
> type: message_start → usage.input_tokens: 0, output_tokens: 0
> type: content_block_delta → delta.type: thinking_delta
> type: content_block_delta → delta.type: signature_delta
> type: message_delta → usage.input_tokens: 39, output_tokens: 29
> ```

---

## 7. 错误响应

### 7.1 错误格式

> 来源：[text-chat-openai](https://platform.minimaxi.com/docs/api-reference/text-chat-openai.md)、[text-chat-anthropic](https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md)、[errorcode](https://platform.minimaxi.com/docs/api-reference/errorcode.md)

#### OpenAI 协议错误字段

`base_resp` 嵌套在响应根对象中：

```json
{
  "base_resp": {
    "status_code": 1004,
    "status_msg": "鉴权失败"
  }
}
```

#### HTTP Status Code

**确认结论**：MiniMax API **总是返回 HTTP 200**，错误信息在响应 body 的 `base_resp` 字段中。HTTP 层面不会返回 4xx/5xx，认证失败（1004）也返回 HTTP 200。

> 原文（[text-chat-openai.md](https://platform.minimaxi.com/docs/api-reference/text-chat-openai.md)）：
> `responses: '200': description: ''`
>
> 原文（[text-chat-anthropic.md](https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md)）：
> `responses: '200': description: ''`
>
> 原文（[errorcode.md](https://platform.minimaxi.com/docs/api-reference/errorcode.md)）：错误码列表中仅有错误码含义说明，无 HTTP status code 分组，印证所有错误均通过 body 传递。
>
> **结论**：所有 API 错误都在 `base_resp.status_code` 中编码，HTTP status 始终为 200。

### 7.2 错误码列表

| 错误码 | 含义 | 解决方法 | 文档来源 |
|--------|------|---------|---------|
| `1000` | 未知错误/系统默认错误 | 请稍后再试 | [errorcode](https://platform.minimaxi.com/docs/api-reference/errorcode.md) |
| `1001` | 请求超时 | 请稍后再试 | 同上 |
| `1002` | 请求频率超限 | 请稍后再试 | 同上 |
| `1004` | 未授权/Token 不匹配/Cookie 缺失 | 请检查 API Key | 同上 |
| `1008` | 余额不足 | 请检查您的账户余额 | 同上 |
| `1013` | 服务内部错误 | 请稍后再试 | 同上 |
| `1024` | 内部错误 | 请稍后再试 | 同上 |
| `1026` | 输入内容涉敏 | 请调整输入内容 | 同上 |
| `1027` | 输出内容涉敏 | 请调整输入内容 | 同上 |
| `1033` | 系统错误/下游服务错误 | 请稍后再试 | 同上 |
| `1039` | Token 限制 | 请调整 max_tokens | 同上 |
| `1041` | 连接数限制 | 请联系我们 | 同上 |
| `1042` | 不可见字符比例超限/非法字符超过 10% | 请检查输入内容 | 同上 |
| `1043` | ASR 相似度检查失败 | 请检查 file_id 与 text_validation 匹配度 | 同上 |
| `1044` | 克隆提示词相似度检查失败 | 请检查克隆提示音频和提示词 | 同上 |
| `2013` | 参数错误 | 请检查请求参数 | 同上 |
| `20132` | 语音克隆参数错误 | 请检查 voice_id 参数 | 同上 |
| `2037` | 语音时长不符合要求（太长或太短） | 请检查音频文件时长（≥10s，≤5min） | 同上 |
| `2038` | 用户语音克隆功能被禁用 | 请完成账户身份认证 | 同上 |
| `2039` | 语音克隆 voice_id 重复 | 请修改 voice_id | 同上 |
| `2042` | 无权访问该 voice_id | 请确认为 voice_id 创建者 | 同上 |
| `2045` | 请求频率增长超限 | 请避免请求骤增骤减 | 同上 |
| `2048` | 语音克隆提示音频太长 | 请调整 prompt_audio 时长（<8s） | 同上 |
| `2049` | 无效的 API Key | 请检查 API Key | 同上 |
| `2056` | 超出Token Plan资源限制 | 请等待资源释放后重试 | 同上 |

> 注：text-chat-openai.md 和 text-chat-anthropic.md 的 schema 中 base_resp.status_code 仅列出部分错误码（1000/1001/1002/1004/1008/1013/1027/1039/2013），errorcode.md 包含更完整的错误码列表（含语音相关错误码 20132/2037-2049 等）。

> 原文（[errorcode](https://platform.minimaxi.com/docs/api-reference/errorcode.md)）：
> ```
> 1000: 未知错误/系统默认错误 → 请稍后再试
> 1001: 请求超时 → 请稍后再试
> 1004: 未授权/Token 不匹配/Cookie 缺失 → 请检查 API Key
> 1008: 余额不足 → 请检查您的账户余额
> 1027: 输出内容涉敏 → 请调整输入内容
> 1039: Token 限制 → 请调整 max_tokens
> 2013: 参数错误 → 请检查请求参数
> 2049: 无效的 API Key → 请检查 API Key
> ```

---

## 8. Cache 机制

> 来源：[text-prompt-caching](https://platform.minimaxi.com/docs/api-reference/text-prompt-caching.md)、[anthropic-api-compatible-cache](https://platform.minimaxi.com/docs/api-reference/anthropic-api-compatible-cache.md)

### 两种缓存模式对比

| | Prompt 缓存（被动缓存） | Anthropic 主动缓存 |
|---|---|---|
| 使用方式 | 自动识别重复内容并缓存，无需改调用方式 | 在 API 中显式设置 `cache_control` |
| 计费 | 命中缓存 token 优惠价；写入缓存无额外计费 | 命中缓存优惠价；写入缓存额外计费（1.25x 输入价） |
| 缓存过期 | 根据系统负载自动调整 | 5 分钟，持续使用自动续期 |
| 支持模型 | M2.7/M2.5/M2.1 系列 | M2.7/M2.5/M2.1/M2 系列 |
| 支持的协议 | OpenAI + Anthropic | 仅 Anthropic |

> 原文：
> ```
> 被动缓存：自动识别重复的上下文内容，无需更改接口调用方式
> 主动缓存：在 anthropic API 中使用需要显式设置参数的缓存模式
> 缓存过期：被动缓存根据系统负载自动调整；主动缓存 5min 过期，持续使用自动续期
> ```

### 被动缓存（Prompt 缓存）

- **触发条件**：输入 token 数量 ≥ 512 时自动触发
- **匹配方式**：前缀匹配（`工具定义 → 系统提示词 → 历史对话内容`）
- **计费**：命中缓存 token 按优惠价；无额外写入费

> 原文：
> ```
> 缓存适用于包含 512 个及以上的输入 token 数量的 API 调用
> 缓存采用前缀匹配的方式，以「工具定义-系统提示词-历史对话内容」为顺序构建
> ```

### 主动缓存（Anthropic cache_control）

- **`cache_control` 参数位置**：应在内容块对象**内部**使用（`{"type": "text", "text": "...", "cache_control": {"type": "ephemeral"}}`），而非作为独立参数传入
- **缓存顺序**: `tools` → `system` → `messages`
- **回溯窗口**: 每个断点前最多检查 20 个块
- **最大断点数**: 每请求最多 4 个 `cache_control` 参数（超过时只取从后向前最近的 4 个）
- **缓存生命周期**: 5 分钟，每次命中自动续期
- **可缓存内容**: `tools` 数组中的工具定义、`system` 数组中的内容块、`messages.content` 中的文本内容块和 `tool_use`/`tool_result` 类型块

> 原文：
> ```
> 20 块回溯窗口：系统在每个显式 Cache 断点之前最多检查 20 个块
> 一次调用最多支持 4 个 cache_control 参数
> 缓存内容的生命周期为 5 分钟，每次命中缓存内容时，缓存生命周期都会自动刷新
> ```

#### 主动缓存计费

| 操作 | 价格倍数 |
|------|---------|
| 缓存写入（cache_creation） | 基础输入价的 **1.25 倍** |
| 缓存读取（cache_read） | 基础输入价的 **0.1 倍** |

> 原文：
> ```
> 缓存写入 token 是基础输入 token 价格的 1.25 倍
> 缓存读取 token 是基础输入 token 价格的 0.1 倍
> ```

#### 主动缓存 token 字段

| 字段 | 说明 |
|------|------|
| `cache_creation_input_tokens` | 写入新缓存的 token 数 |
| `cache_read_input_tokens` | 从缓存读取的 token 数 |
| `input_tokens` | 最后一个断点之后的新增 token 数 |

> 原文：`total_input_tokens = cache_read_input_tokens + cache_creation_input_tokens + input_tokens`

#### 可被主动缓存的内容

- `tools` 数组中的工具定义
- `system` 数组中的内容块
- `messages.content` 数组中的文本消息内容块
- `messages.content` 中的 `tool_use` 和 `tool_result` 类型块

---

## 9. 多轮对话

### OpenAI 协议

- **上下文记忆方式**: 将历史消息作为 `messages` 数组传入
- **Session 处理**: 无 session 概念，客户端自行维护 messages 历史
- **最大 history 长度**: 最多 204800 tokens（含输入+输出）
- **角色类型**: `system`、`user`、`assistant`、`user_system`、`group`、`sample_message_user`、`sample_message_ai`

### Anthropic 协议

- **上下文记忆方式**: 将历史消息作为 `messages` 数组传入
- **Session 处理**: 无 session 概念，客户端自行维护 messages 历史
- **最大 history 长度**: 最多 204800 tokens（含输入+输出）
- **角色类型**: `user`、`assistant`、`user_system`、`group`、`sample_message_user`、`sample_message_ai`
- **注意**: Anthropic 协议不支持 `system` 作为 messages 中的 role

> 原文（[api-overview](https://platform.minimaxi.com/docs/api-reference/api-overview.md)）：
> ```
> MiniMax-M2.7: 输入输出总 token: 204800
> ```

---

## 10. 厂商特有字段

### OpenAI 协议特有字段

| 字段 | 位置 | 类型 | 说明 |
|------|------|------|------|
| `extra_body.reasoning_split` | 请求体 | boolean | 将思考内容分离到 reasoning_details 字段 |
| `input_sensitive` | 响应根 | boolean | 输入是否命中敏感词 |
| `input_sensitive_type` | 响应根 | integer | 输入敏感类型（1-7） |
| `output_sensitive` | 响应根 | boolean | 输出是否命中敏感词 |
| `output_sensitive_type` | 响应根 | integer | 输出敏感类型（1-7） |
| `base_resp` | 响应根 | object | 错误状态码和详情 |
| `base_resp.status_code` | 响应根 | integer | MiniMax 内部错误码 |
| `base_resp.status_msg` | 响应根 | string | 错误详情 |
| `prompt_tokens_details.cached_tokens` | usage | integer | 命中缓存的 prompt token 数 |

### Anthropic 协议特有字段

| 字段 | 位置 | 类型 | 说明 |
|------|------|------|------|
| `cache_control` | content block | object | `{"type": "ephemeral"}` 标记缓存断点 |
| `usage.cache_creation_input_tokens` | usage | integer | 创建缓存的 token 数 |
| `usage.cache_read_input_tokens` | usage | integer | 命中缓存的 token 数 |
| `content[].thinking` | content[] | string | 模型思考内容（type=thinking 时） |
| `content[].signature` | content[] | string | 思考内容签名 |
| `service_tier` | message | string | 服务层级 |

### API Key 相关说明

- MiniMax API Key 可同时用于按量付费和 Token Plan
- Key 获取地址: https://platform.minimaxi.com/user-center/basic-information/interface-key

> 原文：
> ```
> 按量付费：通过接口密钥 > 创建新的 API Key，获取 API Key
> Token Plan：通过接口密钥 > 创建 Token Plan Key，获取 API Key
> ```

---

## 附：各文档信息索引

| 文档 | URL | 主要覆盖内容 |
|------|-----|------------|
| text-chat-openai.md | https://platform.minimaxi.com/docs/api-reference/text-chat-openai.md | OpenAI 协议请求/响应格式 |
| text-chat-anthropic.md | https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md | Anthropic 协议请求/响应格式 |
| text-prompt-caching.md | https://platform.minimaxi.com/docs/api-reference/text-prompt-caching.md | 被动缓存机制 |
| anthropic-api-compatible-cache.md | https://platform.minimaxi.com/docs/api-reference/anthropic-api-compatible-cache.md | 主动缓存机制 |
| api-overview.md | https://platform.minimaxi.com/docs/api-reference/api-overview.md | 所有模型一览 |
| errorcode.md | https://platform.minimaxi.com/docs/api-reference/errorcode.md | 错误码 |
| models/openai/list-models.md | https://platform.minimaxi.com/docs/api-reference/models/openai/list-models.md | OpenAI 模型列表 |
| models/anthropic/list-models.md | https://platform.minimaxi.com/docs/api-reference/models/anthropic/list-models.md | Anthropic 模型列表 |

---

## 11. 工具调用（Tool Use）

> 来源：
> - [text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)（工具使用主文档）
> - [text-chat-openai.md](https://platform.minimaxi.com/docs/api-reference/text-chat-openai.md)（扫 tools 相关参数）
> - [text-chat-anthropic.md](https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md)（扫 tools 相关参数）
> - [anthropic-api-compatible-cache.md](https://platform.minimaxi.com/docs/api-reference/anthropic-api-compatible-cache.md)（扫 tool_use/tool_result 缓存）

### 概述

MiniMax-M2.7 是 Agentic Model，原生支持 **工具使用（Tool Use）** 和 **Interleaved Thinking**。模型能在每轮 Tool Use 前，根据工具返回结果进行思考并决策下一步行动。

### OpenAI 协议 — tools 参数

```json
tools: [
  {
    "type": "function",           // 固定值
    "function": {
      "name": "get_weather",        // 函数名，字符串
      "description": "Get weather of a location, the user should supply a location first.",  // 描述
      "parameters": {               // JSON Schema 格式的参数定义
        "type": "object",
        "properties": {
          "location": {
            "type": "string",
            "description": "The city and state, e.g. San Francisco, US"
          }
        },
        "required": ["location"]
      }
    }
  }
]
```

**字段说明：**
- `type`: 固定为 `"function"`，不可省略
- `function.name`: 工具函数名，必填
- `function.description`: 工具描述，可选但建议填写
- `function.parameters`: [JSON Schema](https://json-schema.org/) 格式的参数定义，必填

> 原文（[text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)）：
> ```python
> tools = [
>     {
>         "type": "function",
>         "function": {
>             "name": "get_weather",
>             "description": "Get weather of a location, the user should supply a location first.",
>             "parameters": {
>                 "type": "object",
>                 "properties": {
>                     "location": {
>                         "type": "string",
>                         "description": "The city and state, e.g. San Francisco, US",
>                     }
>                 },
>                 "required": ["location"],
>             },
>         },
>     }
> ]
> ```

**tools 是否必填：** 在 `ChatCompletionReq` schema 中 `tools` 未标记为 `required`，为可选参数。

### OpenAI 协议 — tool_call 响应

当模型决定调用工具时，`message.tool_calls` 数组包含调用信息：

```json
"tool_calls": [
  {
    "id": "call_function_2831178524_1",   // 工具调用唯一 ID
    "type": "function",                    // 固定值
    "function": {
      "name": "get_weather",               // 被调用的工具名称
      "arguments": "{\"location\": \"San Francisco, US\"}"  // JSON 格式参数字符串
    },
    "index": 0
  }
]
```

**finish_reason**: 当模型触发工具调用时，值为 `"tool_calls"`（非 `"stop"`）


> 原文（[text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)）：
> ```json
> "choices": [{
    "finish_reason": "tool_calls",
    "message": {
      "tool_calls": [{
        "id": "call_function_2831178524_1",
        "type": "function",
        "function": {
          "name": "get_weather",
          "arguments": "{\"location\": \"San Francisco, US\"}"
        },
        "index": 0
      }]
    }
  }]
> ```

**多轮对话中的工具结果回传（OpenAI 格式）：**

```python
messages.append(response_message)  # 回传完整的 assistant 消息（含 tool_calls）
messages.append({
    "role": "tool",
    "tool_call_id": tool_call.id,
    "content": "24℃, sunny"
})
```

> 原文（[text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)）：
> ```python
> messages.append(response_message)
messages.append(
    {
        "role": "tool",
        "tool_call_id": tool_call.id,
        "content": "24℃, sunny",
    }
)
> ```

### Anthropic 协议 — tools 参数


```json
tools: [
  {
    "name": "get_weather",          // 函数名，与 OpenAI 的 function.name 对应
    "description": "Get weather of a location, the user should supply a location first.",  // 描述
    "input_schema": {                 // JSON Schema 格式（注意字段名是 input_schema 而非 parameters）
      "type": "object",
      "properties": {
        "location": {
          "type": "string",
          "description": "The city and state, e.g. San Francisco, US"
        }
      },
      "required": ["location"]
    }
  }
]
```

**与 OpenAI 协议的字段对照：**

| OpenAI 字段 | Anthropic 字段 | 说明 |
|------------|--------------|------|
| `function.name` | `name` | 函数名 |
| `function.description` | `description` | 描述 |
| `function.parameters` | `input_schema` | 参数定义（注意命名差异） |

> 原文（[text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)）：
> ```python
> tools = [
>     {
>         "name": "get_weather",
>         "description": "Get weather of a location, the user should supply a location first.",
>         "input_schema": {
>             "type": "object",
>             "properties": {
>                 "location": {
>                     "type": "string",
>                     "description": "The city and state, e.g. San Francisco, US",
>                 }
>             },
>             "required": ["location"]
>         }
>     }
> ]
> ```

### Anthropic 协议 — tool_use 响应

模型响应中的 `content` 数组包含 `tool_use` 类型块：

```json
"content": [
  {
    "type": "tool_use",
    "id": "toolu_01XXXXXXXX",       // 工具调用唯一 ID
    "name": "get_weather",               // 被调用的工具名称
    "input": {"location": "San Francisco, US"}  // 参数对象（注意：是对象而非 JSON 字符串）
  }
]
```

> 原文（[text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)）：
> ```python
> for block in response.content:
>     if block.type == "tool_use":
>         print(f"🔧 Tool>\t{block.name}({json.dumps(block.input, ensure_ascii=False)})")
>         # block.input 是 dict 对象（而非 JSON 字符串）
> ```

### tool_result 格式

用户/助手回传工具执行结果时使用 `tool_result` 块：

```json
{
  "type": "tool_result",
  "tool_use_id": "toolu_01XXXXXXXX",   // 对应 tool_use 块的 id
  "content": "24℃, sunny"             // 工具返回的内容
}
```

**可选字段：**

| 字段 | 类型 | 说明 |
|------|------|------|
| `tool_use_id` | string | 必填，对应 tool_use 块的 id |
| `content` | string | 必填，工具返回内容 |
| `is_error` | boolean | 可选，标识是否为错误 [待确认] |

> 原文（[text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)）：
> ```python
> messages.append({
>     "role": "user",
>     "content": [
>         {
>             "type": "tool_result",
>             "tool_use_id": tool_use_blocks[0].id,
>             "content": tool_result
>         }
>     ]
> })
> ```

### thinking 与工具调用

**MiniMax-M2.7 支持 Interleaved Thinking**：模型在每轮 Tool Use 前会先输出 thinking 内容块，再输出 tool_use 块。

**Anthropic 协议响应示例：**

```json
"content": [
  {
    "type": "thinking",
    "thinking": "用户问天气，我需要调用 get_weather 工具...",
    "signature": "ce704495524bad054531fe187e18b4a8..."
  },
  {
    "type": "tool_use",
    "id": "toolu_01XXXXXXXX",
    "name": "get_weather",
    "input": {"location": "San Francisco, US"}
  }
]
```

> 原文（[text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)）：
> ```python
> for block in response.content:
>     if block.type == "thinking":
>         print(f"💭 Thinking>\n{block.thinking}\n")
>     elif block.type == "tool_use":
>         print(f"🔧 Tool>\t{block.name}({json.dumps(block.input)})")
> ```

**OpenAI 协议下的 thinking：**

通过 `reasoning_split=True` 参数，thinking 内容会单独输出到 `reasoning_details` 字段：

```json
"message": {
  "content": "\n",
  "reasoning_details": [
    {
      "type": "reasoning.text",
      "id": "reasoning-text-1",
      "format": "MiniMax-response-v1",
      "index": 0,
      "text": "Let me think about this request..."
    }
  ],
  "tool_calls": [...]
}
```

> 原文（[text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)）：
> `reasoning_split=True` 可将思考内容分离到 `reasoning_details` 字段中。

### Streaming 下的工具调用

**Anthropic 协议 Streaming：** SSE chunk 中 tool_use 以增量形式输出：

| 事件类型 | 说明 |
|---------|------|
| `content_block_start` | 块开始，type 为 `tool_use` |
| `content_block_delta` | 增量更新，delta.type 为 `tool_use_delta` 时表示工具调用的增量 |
| `content_block_stop` | 块结束 |

**delta.type 的可能值：**

| delta.type | 说明 |
|------------|------|
| `thinking_delta` | thinking 内容的增量 |
| `text_delta` | text 内容的增量 |
| `tool_use_delta` | tool_use 块的增量（工具名/参数）[待确认] |
| `signature_delta` | thinking 签名的增量 [待确认] |

> 原文（[text-chat-anthropic.md](https://platform.minimaxi.com/docs/api-reference/text-chat-anthropic.md)）Streaming 示例中未出现 tool_use 块，工具调用的 Streaming 格式 **[待确认]**。

**OpenAI 协议 Streaming：** SSE chunk 中 tool_call 以 delta 形式输出：

```json
{
  "choices": [{
    "index": 0,
    "delta": {
      "role": "assistant",
      "tool_calls": [{
        "index": 0,
        "id": "call_function_xxx",
        "type": "function",
        "function": {
          "name": "get_weather",
          "arguments": "{\"location\": \"..."}"
        }
      }]
    }
  }]
}
```

[待确认] OpenAI 协议 Streaming 中 tool_call delta 的具体格式（如 arguments 是否拆分逐 token 输出）**[待确认]**。

### 工具调用错误

| 错误码 | 含义 | 说明 |
|--------|------|------|
| 2013 | 参数错误 | 可能由 tool 相关参数格式错误引发 [待确认是否单独针对 tool 错误码] |

> 原文（[errorcode.md](https://platform.minimaxi.com/docs/api-reference/errorcode.md)）：
> `2013: 参数错误 — 请检查请求参数`

**已知 2013 可能场景：**
- tools 数组格式错误
- function.arguments JSON 解析失败
- 必填参数缺失

[待确认] tool 相关参数错误的更细分错误码 **[待确认]**。

### 工具与缓存

**结论：tools 数组可以被主动缓存。**

> 原文（[anthropic-api-compatible-cache.md](https://platform.minimaxi.com/docs/api-reference/anthropic-api-compatible-cache.md)）**可被缓存的内容** 一节明确列出：
> - **工具**：`tools` 数组中的工具定义
> - **工具使用和工具结果**：`messages.content` 数组中的内容块中的 tool_use 和 tool_result 类型

**缓存示例：**

```python
response = client.messages.create(
    model="MiniMax-M2.7",
    tools=[
        {
            "name": "get_weather",
            "description": "Get the current weather in a given location",
            "input_schema": {...}
        },
        {
            "name": "get_time",
            "description": "Get the current time in a given time zone",
            "input_schema": {...},
            "cache_control": {"type": "ephemeral"}  # 标记缓存断点
        }
    ],
    ...
)
```

**注意**：缓存前缀按顺序 `tools → system → messages` 构建。对缓存内容的修改会使该级别及后续所有级别失效。

### 关键约束：完整回传

**多轮对话中必须将完整的模型返回添加到对话历史**：


- **OpenAI SDK**：将完整的 `response_message` 对象（含 `tool_calls` 和 `reasoning_details`）添加到消息历史
- **Anthropic SDK**：将完整的 `response.content`（含 thinking/text/tool_use 等所有块）添加到消息历史

> 原文（[text-m2-function-call.md](https://platform.minimaxi.com/docs/guides/text-m2-function-call.md)）：
> 在多轮 Function Call 对话中，必须将完整的模型返回（即 assistant 消息）添加到对话历史，以保持思维链的连续性。

如不完整回传，后续对话会丢失上下文信息，模型性能下降。
