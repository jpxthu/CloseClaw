# DeepSeek Fixtures

## 推荐 API：Anthropic（略优）

两种协议表现接近，Anthropic 有 `signature` 字段可用于多轮对话追溯，略优。

| 协议 | Thinking 格式 | 推荐场景 |
|------|-------------|---------|
| **Anthropic** ✅ | `content: [{type:"thinking", thinking:"...", signature:"..."}, {type:"text", text:"..."}]` | 优先使用 |
| OpenAI | `choices[].message.reasoning_content` 独立字段，`content` 干净 | 也可接受 |

---

## 目录结构

```
deepseek/
├── deepseek-v4-flash/
│   ├── openai/
│   │   ├── simple.json            # 普通回复
│   │   ├── cache.json             # 3轮 cache 对话
│   │   ├── deepseek-thinking-high.json  # reasoning_effort: high
│   │   ├── streaming.txt          # SSE 流式，reasoning_content 分块
│   │   ├── tool-use.json          # tool_calls
│   │   ├── tool-result.json       # 2轮对话，Round2 带 reasoning_content
│   │   ├── tool-use-streaming.json
│   │   ├── error-model.json
│   │   ├── error-empty.json
│   │   └── error-tool-format.json
│   └── anthropic/
│       ├── anthropic-simple.json      # 普通回复，含 thinking+signature
│       ├── anthropic-thinking.json     # reasoning 场景
│       ├── anthropic-streaming.txt     # SSE，thinking 和 text 分块交替
│       ├── anthropic-streaming-meta.json  # streaming 元数据
│       ├── anthropic-tool-use.json    # tool_use block
│       ├── anthropic-tool-result.json # 2轮，preserve thinking
│       ├── anthropic-tool-use-streaming.txt
│       ├── anthropic-tool-use-streaming-meta.json
│       ├── anthropic-cache.json        # 3轮 cache 对话
│       ├── anthropic-error-auth.json
│       ├── anthropic-error-empty.json
│       └── anthropic-error-model.json
└── deepseek-v4-pro/
    └── [同上结构]
```

**文件命名**：Anthropic 文件统一加 `anthropic-` 前缀以区分协议；OpenAI 文件直接为场景名。

---

## 场景响应字段说明

### Simple（普通回复）

**OpenAI** (`openai/simple.json`)：
```json
{
  "choices": [{
    "message": {
      "role": "assistant",
      "content": "Hello my friend",
      "reasoning_content": "We need to say hello in 3 words..."
    }
  }],
  "usage": {
    "completion_tokens_details": { "reasoning_tokens": 165 }
  }
}
```
- `reasoning_content`：thinking 内容
- `content`：最终回复
- `completion_tokens_details.reasoning_tokens`：reasoning token 计数

**Anthropic** (`anthropic/anthropic-simple.json`)：
```json
{
  "content": [
    { "type": "thinking", "thinking": "We need to say hello...", "signature": "ddb4..." },
    { "type": "text", "text": "Hello, my friend." }
  ],
  "usage": { "output_tokens": 124 }
}
```
- thinking block 含 `signature`（值为 message id），可用于多轮追溯
- text block 含最终回复

---

### Thinking / DeepSeek Thinking High

两个模型一致：

**OpenAI** (`openai/deepseek-thinking-high.json`)：
```json
{
  "choices": [{
    "message": {
      "content": "To compute 17×23...",
      "reasoning_content": "We need to compute 17*23. Step by step..."
    }
  }],
  "usage": {
    "completion_tokens_details": { "reasoning_tokens": 128 }
  }
}
```

**Anthropic** (`anthropic/anthropic-thinking.json`)：
```json
{
  "content": [
    { "type": "thinking", "thinking": "We need to compute...", "signature": "fef4..." },
    { "type": "text", "text": "17 × 23 = 391..." }
  ]
}
```

---

### Streaming SSE

**OpenAI** (`openai/streaming.txt`)：
- 首个 chunk：`delta: {role:"assistant", content:null, reasoning_content:""}`
- reasoning 阶段：`delta: {reasoning_content:"片段", content:null}`
- content 阶段：`delta: {content:"片段", reasoning_content:null}`
- 终止 chunk：`finish_reason: "stop"`，含 usage
- 格式：`data: {...}\n\ndata: [DONE]\n\ndata: [DONE]`

**Anthropic** (`anthropic/anthropic-streaming.txt`)：
- 事件流：`message_start` → `content_block_start(type=thinking)` → `content_block_delta(type=thinking_delta)` → `signature_delta` → `content_block_stop` → `content_block_start(type=text)` → `content_block_delta(type=text_delta)` → `message_delta` → `message_stop`
- `ping` 事件穿插
- reasoning 内容通过 `thinking_delta` 分块，`signature_delta` 最后发送
- Flash 约 233 条事件，Pro 约 434 条

---

### Tool Use（工具调用）

**OpenAI** (`openai/tool-use.json`)：
```json
{
  "choices": [{
    "message": {
      "content": "",
      "reasoning_content": "The user wants weather info...",
      "tool_calls": [{
        "index": 0,
        "id": "call_00_...",
        "type": "function",
        "function": { "name": "get_weather", "arguments": "{\"location\":\"San Francisco\"}" }
      }]
    },
    "finish_reason": "tool_calls"
  }],
  "usage": {
    "prompt_tokens_details": { "cached_tokens": 256 },
    "prompt_cache_hit_tokens": 256
  }
}
```

**Anthropic** (`anthropic/anthropic-tool-use.json`)：
```json
{
  "content": [
    { "type": "thinking", "thinking": "User wants weather...", "signature": "8d9..." },
    { "type": "tool_use", "id": "call_00_...", "name": "get_weather", "input": {"location": "San Francisco"} }
  ],
  "stop_reason": "tool_use",
  "usage": { "cache_read_input_tokens": 256 }
}
```
- thinking 后跟 `tool_use` block，无 text block

---

### Tool Result（多轮对话）

**OpenAI**：`openai/tool-result.json`
- Round 1：`tool_calls` + `reasoning_content`
- Round 2：继续传 `reasoning_content`，需完整回传

**Anthropic**：`anthropic/anthropic-tool-result.json`
- Round 1：`content: [thinking, tool_use]`
- Round 2：必须回传**完整 content 数组**（含 thinking），否则 400
- 错误：`thinking content must be passed back`

---

### Cache（被动缓存）

DeepSeek 无主动 cache control，被动触发。

**OpenAI** (`openai/cache.json`)：
```json
"usage": {
  "prompt_tokens_details": { "cached_tokens": 128 },
  "prompt_cache_hit_tokens": 128,
  "prompt_cache_miss_tokens": 123
}
```

**Anthropic** (`anthropic/anthropic-cache.json`)：
```json
"usage": {
  "input_tokens": 123,
  "cache_creation_input_tokens": 0,
  "cache_read_input_tokens": 128
}
```

Cache 被动触发，`cached_tokens` / `cache_read_input_tokens` 随轮次递增（第 2 轮起稳定在 256/512）。

---

## 错误响应

两协议共用格式：
```json
{ "error": { "message": "...", "type": "...", "code": "..." } }
```

| 场景 | 状态码 |
|------|--------|
| `error-model` | 400 |
| `error-empty` | 400 |
| `error-auth` | 401 |
| `error-tool-format` | 400 |

---

## 模型对比

| 模型 | reasoning_tokens（simple） | streaming 事件数 |
|------|--------------------------|------------------|
| `deepseek-v4-flash` | ~165 | ~233 条 |
| `deepseek-v4-pro` | ~492 | ~434 条 |

差异源于模型能力，非接口格式不同。