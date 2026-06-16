# MiMo Fixtures

> Xiaomi MiMo 开放平台（OpenAI / Anthropic 双协议）  
> 模型：`mimo-v2.5-pro`（主力）、`mimo-v2.5`（基础版）

## 推荐协议：OpenAI（略优）

| 协议 | Thinking 处理 | Thinking 字段 | 工具调用 | Cache 字段 |
|------|-------------|--------------|---------|-----------|
| **OpenAI** ✅ | `choices[].message.reasoning_content` **独立字段**，`content` 干净 | 始终存在（即使未启用 thinking 也有短内容） | `finish_reason="tool_calls"` + `tool_calls[]` | `usage.prompt_tokens_details.cached_tokens` |
| Anthropic | `content[]` 含独立 `type:"thinking"` block，有 `signature` 字段 | thinking 块独立 | `content[]` 含 `type:"tool_use"` | `usage.cache_read_input_tokens` |

**OpenAI 略优**：thinking 在 OpenAI 下结构最简单（顶层独立字段，无需遍历 `content[]` 数组），与 GLM 的处理方式一致，平台实现可以共享代码路径。Anthropic 也完全可用（thinking 独立 block、signature 字段），不是缺陷。

> **关键差异**：MiMo 与 GLM 一样，**OpenAI 协议下 `reasoning_content` 始终存在**（即使未启用 thinking 也有短内容），不依赖 `extra_body.thinking` 参数——thinking 是模型默认行为。

---

## 目录结构

```
mimo/
├── mimo-v2.5-pro/             # 主力模型（推荐使用）
│   ├── openai/                # 14 场景
│   │   ├── simple.json
│   │   ├── multi-turn.json
│   │   ├── cache.json         # cache 命中递增（cached_tokens 首轮缺失，后续命中 = 256）
│   │   ├── context-pressure.json  # 多轮递增长对话
│   │   ├── streaming.txt      # SSE：reasoning_content → content 分两阶段
│   │   ├── streaming-meta.json
│   │   ├── tool-use.json
│   │   ├── tool-result.json
│   │   ├── tool-use-streaming.txt
│   │   ├── tool-use-streaming-meta.json
│   │   └── error-*.json       # auth / empty / model / tool-format
│   └── anthropic/             # 13 场景（anthropic- 前缀）
│       ├── anthropic-simple.json
│       ├── anthropic-thinking.json       # thinking 独立 block
│       ├── anthropic-cache.json
│       ├── anthropic-context-pressure.json
│       ├── anthropic-streaming.txt       # thinking_delta → text_delta
│       ├── anthropic-streaming-meta.json
│       ├── anthropic-tool-use.json
│       ├── anthropic-tool-result.json
│       ├── anthropic-tool-use-streaming.txt
│       ├── anthropic-tool-use-streaming-meta.json
│       └── anthropic-error-*.json        # auth / empty / model（无 tool-format）
├── mimo-v2.5/                 # 基础版，相同结构
└── provider/                  # Provider 级别 fixture
    ├── model-list.json        # GET /v1/models（9 个模型）
    └── usage-quota.json       # skipped（MiMo 无 usage-quota API）
```

---

## 各场景响应字段

### OpenAI — simple

文件：`mimo-v2.5-pro/openai/simple.json`

```json
{
  "choices": [{
    "message": {
      "content": "Hello to you! 👋",
      "reasoning_content": "The user is asking me to say hello in 3 words.",   // 始终存在
      "role": "assistant",
      "tool_calls": null
    },
    "finish_reason": "stop"
  }],
  "usage": {
    "completion_tokens": 22,
    "prompt_tokens": 258,
    "completion_tokens_details": {"reasoning_tokens": 13},
    "prompt_tokens_details": {"cached_tokens": 192}        // 命中缓存
  }
}
```

**注意**：`reasoning_content` 始终有值（即使未启用 thinking 也会有短推理内容）。

---

### OpenAI — cache

文件：`mimo-v2.5-pro/openai/cache.json`

**MiMo 支持 cache**：`cached_tokens` 随轮次递增。
- Turn 1：长 system prompt 完整计费
- Turn 2/3/4：长 system prompt 命中 cache，`cached_tokens` > 0

---

### OpenAI — streaming（SSE）

文件：`mimo-v2.5-pro/openai/streaming.txt`

流式分两个阶段（与 GLM 一致）：
1. **`reasoning_content` 阶段**：`delta.reasoning_content` 增量
2. **`content` 阶段**：`delta.content` 增量
3. **结束 chunk**：`finish_reason="stop"`（usage=null），后续独立 chunk 携带 `usage` 统计

```
data: {...,"delta":{"reasoning_content":"The user is"}}
data: {...,"delta":{"reasoning_content":" asking me to"}}
...
data: {...,"delta":{"content":"1"}}
data: {...,"delta":{"content":", 2"}}
data: {...,"delta":{"content":", 3!"}}
data: {...,"finish_reason":"stop","delta":{"content":""}}
data: {...,"usage":{"prompt_tokens":259,...}}
data: [DONE]
```

---

### OpenAI — tool-use

文件：`mimo-v2.5-pro/openai/tool-use.json`

```json
{
  "choices": [{
    "finish_reason": "tool_calls",
    "message": {
      "content": "",
      "reasoning_content": "The user wants to know the current weather in San Francisco...",
      "tool_calls": [{
        "id": "call_5850d8fee82b4ca3940e9a2f",
        "type": "function",
        "function": {"name": "get_weather", "arguments": "{\"location\":\"San Francisco\"}"}
      }]
    }
  }],
  "usage": {
    "prompt_tokens_details": {"cached_tokens": 448},
    "completion_tokens_details": {"reasoning_tokens": 29}
  }
}
```

**多轮工具调用**：回传时需携带上一轮的 `reasoning_content`（与 GLM 处理一致），否则多轮可能断裂。

---

### Anthropic — simple / thinking

文件：`mimo-v2.5-pro/anthropic/anthropic-simple.json` / `anthropic-thinking.json`

**Anthropic 协议下 thinking 始终为独立 block**：

```json
{
  "content": [
    {"type": "text", "text": "Hello to you! 👋"},
    {"type": "thinking", "thinking": "The user wants me to say hello in exactly 3 words.", "signature": ""}
  ],
  "stop_reason": "end_turn",
  "usage": {"input_tokens": 66, "output_tokens": 22, "cache_read_input_tokens": 192}
}
```

**注意 `signature` 字段**：实测中 MiMo Anthropic 协议下 `signature` 为**空字符串**（`""`）——这是一个有效但为空的签名值，回传时不能丢弃。

> 与 GLM 行为对比：GLM 在 Anthropic 协议下**完全丢失 thinking block**（仅 text）；MiMo 在 Anthropic 协议下 thinking 保留完整。

---

### Anthropic — streaming

文件：`mimo-v2.5-pro/anthropic/anthropic-streaming.txt`

```
event: message_start
event: content_block_start       // index=0, type="thinking"
event: content_block_delta        // type=thinking_delta
event: content_block_delta        // type=signature_delta（signature=""）
event: content_block_stop
event: content_block_start       // index=1, type="text"
event: content_block_delta        // type=text_delta
...
event: message_delta
event: message_stop
data: [DONE]
```

thinking 和 text 分块传输，结构清晰。

---

### Anthropic — tool-use

文件：`mimo-v2.5-pro/anthropic/anthropic-tool-use.json`

```json
{
  "content": [
    {"type": "thinking", "thinking": "...", "signature": ""},
    {"type": "tool_use", "id": "call_...", "name": "get_weather", "input": {"location": "San Francisco"}}
  ],
  "stop_reason": "tool_use"
}
```

工具参数格式：`[{name, description, input_schema}]`（Anthropic 风格，无 `type`/`function` 层）。

---

## 模型能力对比

| 模型 | OpenAI | Anthropic | 备注 |
|------|--------|-----------|------|
| `mimo-v2.5-pro` | ✅ | ✅ | 主力模型，能力更强 |
| `mimo-v2.5` | ✅ | ✅ | 基础版，能力略弱 |

`mimo-v2.5-pro` 和 `mimo-v2.5` 的 fixture 集**完全平行**（14 + 13 = 27 个 chat 场景 × 2 个模型 + provider 级别 2 个）。

---

## Cache 支持结论

**MiMo 支持 cache**：
- OpenAI 协议：`prompt_tokens_details.cached_tokens` 命中递增
- Anthropic 协议：`usage.cache_read_input_tokens` 命中递增

无 `cache_control` 标记时也会自动 cache 长前缀（实测 system prompt 长度触发缓存）。

---

## 错误响应格式

| 错误类型 | HTTP 状态 | 典型 body |
|---------|----------|----------|
| 401 auth | 401 | `{"error":{"message":"Invalid API Key","code":"401","type":"invalid_key"}}` |
| 400 empty messages | 400 | `{"error":{"code":"400","message":"Param Incorrect","param":"messages must not be empty"}}` |
| 400 model not found | 400 | `{"error":{"code":"400","message":"Param Incorrect","param":"Not supported model <id>"}}` |
| 400 tool format | 400 | `{"error":{"code":"400","message":"Param Incorrect","param":"`name` is not set"}}` |

`response.error` 字段为 `true` 时表示错误响应（fixture 包装层），实际错误体在 `response.body.error` 下。

---

## 特殊说明

### 1. Key 与 Base URL 分两套（不互通）

| 用量类型 | Key 格式 | Base URL |
|---------|---------|---------|
| Pay-as-you-go（按量） | `sk-xxx` | `https://api.xiaomimimo.com/v1` |
| Token Plan（订阅） | `tp-xxx` | `https://token-plan-cn.xiaomimimo.com/v1` |

**两类不可混用**，混用会返回 401。

### 2. 双认证 Header

```bash
api-key: $MIMO_API_KEY         # 推荐
# 或
Authorization: Bearer $MIMO_API_KEY
```

### 3. 无 usage-quota API

`mimo/provider/usage-quota.json` 是 `{"skipped": true, "reason": "mimo 无 usage-quota API"}` 占位符。

### 4. OpenAI reasoning_content 始终存在

与 GLM 一致：`reasoning_content` 不依赖 `extra_body.thinking` 参数，thinking 是默认行为。这简化了客户端实现——不需要根据 thinking 开关判断字段是否出现。

---

## 完整 API 文档

详见 `../docs/mimo-api-summary.md`（认证、参数、模型列表、错误码、URL 索引等）。
