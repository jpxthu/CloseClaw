# GLM Fixtures

## 推荐协议结论

**优先使用 OpenAI 协议。**

GLM 在 Anthropic 协议下，simple 场景（无工具调用）会**完全丢失 thinking 内容**，无法通过任何字段恢复。

OpenAI 协议下 `reasoning_content` 是独立字段，`content` 干净，thinking 始终可用。

| 协议 | Thinking 处理 | 备注 |
|------|-------------|------|
| **OpenAI** ✅ | `choices[].message.reasoning_content` 独立字段，`content` 不混入 thinking | 优先使用 |
| Anthropic ⚠️ | simple 场景 `content` 只有 text block，thinking block 完全丢失 | 仅工具调用场景使用 |

---

## 目录结构

```
glm/
├── glm-5.1/
│   ├── openai/          # 16 场景 × 多模型
│   │   ├── simple.json          # 基础对话
│   │   ├── glm-thinking.json    # thinking enabled（extra_body.thinking.type="enabled"）
│   │   ├── glm-thinking-disabled.json  # thinking disabled
│   │   ├── cache.json          # cache 验证（结果 prompt_tokens_details.cached_tokens = 0）
│   │   ├── streaming.txt       # SSE 流式
│   │   ├── streaming-meta.txt  # 含 usage 元数据的 SSE
│   │   ├── tool-use.json       # tool_calls + reasoning_content
│   │   ├── tool-result.json    # 多轮回传 reasoning_content
│   │   ├── tool-use-streaming.txt      # 流式 tool_calls
│   │   ├── tool-use-streaming-meta.txt  # 流式 + usage
│   │   ├── multi-turn.json     # 多轮对话
│   │   ├── error-auth.json     # 401
│   │   ├── error-model.json    # 400 model not found
│   │   └── error-empty.json    # 空消息体
│   └── anthropic/       # 14 场景 × 多模型
│       ├── anthropic-simple.json      # ⚠️ 无 thinking block
│       ├── anthropic-thinking.json     # ⚠️ 仍无 thinking block
│       ├── anthropic-cache.json        # cache_read_input_tokens = 0（不支持）
│       ├── anthropic-streaming.txt    # SSE content_block_delta
│       ├── anthropic-streaming-meta.txt
│       ├── anthropic-tool-use.json    # content[].type="tool_use"
│       ├── anthropic-tool-result.json
│       ├── anthropic-tool-use-streaming.txt
│       ├── anthropic-tool-use-streaming-meta.txt
│       └── anthropic-error-*.json
├── glm-4.7/             # 同结构，支持 tool_stream=True
├── glm-4.6/             # 同结构，支持 tool_stream=True
├── glm-5/               # 同结构，支持 tool_stream=True
├── glm-5-turbo/         # OpenAI only
├── glm-4.5-air/         # OpenAI only
└── glm-4.5-airx/        # OpenAI only
```

---

## 各场景响应字段说明

### OpenAI — simple（基础对话）

文件：`glm-5.1/openai/simple.json`

```json
{
  "choices": [{
    "message": {
      "content": "Hello to you.",
      "reasoning_content": "1. **Analyze the Request:** ...",   // 始终存在，内容可能很短
      "role": "assistant"
    },
    "finish_reason": "stop"
  }],
  "usage": {
    "prompt_tokens_details": {"cached_tokens": 0},
    "completion_tokens_details": {"reasoning_tokens": 290},
    "completion_tokens": 296
  }
}
```

**注意**：即使未启用 thinking，`reasoning_content` 字段也会出现，只是内容很短（模型仍做内部推理）。

---

### OpenAI — glm-thinking（thinking enabled）

文件：`glm-5.1/openai/glm-thinking.json`

请求需发送：
```json
"extra_body": {"thinking": {"type": "enabled"}}
```

响应：
```json
{
  "choices": [{
    "message": {
      "reasoning_content": "1. Understand the Goal...\n2. Choose a Method...\n3. Drafting...\n4. Structuring...\n...",
      "content": "17 * 23 = **391**\n\nHere is the step-by-step..."
    }
  }],
  "usage": {
    "completion_tokens_details": {"reasoning_tokens": 846},
    "completion_tokens": 1270
  }
}
```

`reasoning_content` 长且完整，`content` 干净不含 thinking。

---

### OpenAI — cache

文件：`glm-5.1/openai/cache.json`

**GLM 不支持 cache**，`prompt_tokens_details.cached_tokens` 始终为 `0`。
连续三次相同 system + user 调用，prompt_tokens 均为 26，无缓存命中。

---

### OpenAI — streaming（SSE）

文件：`glm-5.1/openai/streaming.txt`

流式输出分为两个阶段：
1. **reasoning_content 阶段**：`delta` 中只有 `reasoning_content`，持续 chunks
2. **content 阶段**：`delta` 中只有 `content`，持续 chunks
3. **结束 chunk**：`choices[].finish_reason="stop"` + `usage` 字段

典型 chunk 序列：
```
data: {"choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"1"}}]}
data: {"choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":". "}}]}
... （reasoning_content 逐渐累积）
data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"1"}}]}
data: {"choices":[{"index":0,"delta":{"content":","}}]}
... （content 逐渐累积）
data: {"choices":[{"index":0,"finish_reason":"stop","delta":{"role":"assistant","content":""}}],
       "usage":{"prompt_tokens":13,"completion_tokens":115,...}}
data: [DONE]
```

---

### OpenAI — tool-use

文件：`glm-5.1/openai/tool-use.json`

```json
{
  "choices": [{
    "finish_reason": "tool_calls",
    "message": {
      "reasoning_content": "The user wants to know the current weather...",
      "content": "",
      "tool_calls": [{
        "id": "call_-7666540262859992883",
        "type": "function",
        "function": {"name": "get_weather", "arguments": "{\"location\":\"San Francisco\"}"}
      }]
    }
  }],
  "usage": {
    "prompt_tokens_details": {"cached_tokens": 128},  // 有缓存命中
    "completion_tokens_details": {"reasoning_tokens": 20},
    "completion_tokens": 33
  }
}
```

**多轮工具调用关键**：回传时需携带上一轮的 `reasoning_content`，否则多轮会断裂。
控制参数：`extra_body.thinking.clear_thinking: false`（保留上一轮）。

---

### Anthropic — simple / thinking

文件：`glm-5.1/anthropic/anthropic-simple.json` / `anthropic-thinking.json`

**Both have NO thinking block**，response is clean text only：

```json
{
  "content": [{"type": "text", "text": "Hello to you!"}],
  "stop_reason": "end_turn"
}
```

thinking 内容完全丢失，无法通过任何字段恢复。Anthropic 协议下 GLM 不返回 thinking block。

---

### Anthropic — cache

文件：`glm-5.1/anthropic/anthropic-cache.json`

```json
"usage": {
  "cache_read_input_tokens": 0
}
```

**GLM 不支持 cache**，即使 system 末尾标记了 `cache_control:ephemeral`，`cache_read_input_tokens` 仍为 0。

---

### Anthropic — streaming

文件：`glm-5.1/anthropic/anthropic-streaming.txt`

SSE 事件流：
```
event: message_start
event: ping
event: content_block_start      // index=0, type="text"
event: content_block_delta       // text_delta 逐段输出
event: content_block_delta
...
event: content_block_stop
event: message_delta             // 含 usage 和 stop_reason
event: message_stop
data: [DONE]
```

无 thinking block，全程只有 `content_block_delta` → `text`。

---

### Anthropic — tool-use

文件：`glm-5.1/anthropic/anthropic-tool-use.json`

```json
{
  "content": [{
    "type": "tool_use",
    "id": "call_ab8014a9d2e84db2807640bd",
    "name": "get_weather",
    "input": {"location": "San Francisco"}
  }],
  "stop_reason": "tool_use"
}
```

工具调用格式：`content[].type="tool_use"`，无 reasoning_content 字段。
`tools` 参数格式：`[{name, description, input_schema}]`（无 `type` 和 `function` 层）。

---

## 模型能力对比

| 模型 | OpenAI | Anthropic | tool_stream | 备注 |
|------|--------|-----------|-------------|------|
| `glm-5.1` | ✅ | ✅ | ❌ | |
| `glm-5` | ✅ | ✅ | ✅ | |
| `glm-5-turbo` | ✅ | ❌ | ❌ | |
| `glm-4.7` | ✅ | ✅ | ✅ | |
| `glm-4.7-flash` | ✅ | ❌ | ❌ | |
| `glm-4.7-flashx` | ✅ | ❌ | ❌ | |
| `glm-4.6` | ✅ | ✅ | ✅ | |
| `glm-4.5-air` | ✅ | ❌ | ❌ | |
| `glm-4.5-airx` | ✅ | ❌ | ❌ | |

**tool_stream**：仅 GLM-5 / GLM-4.7 / GLM-4.6 支持 `tool_stream: True`，其他模型不支持。

---

## Cache 支持结论

**GLM 不支持任何 Cache 机制**。
- OpenAI 协议：`prompt_tokens_details.cached_tokens` 始终为 0
- Anthropic 协议：`cache_read_input_tokens` 始终为 0

如需 cache 功能，请使用 MiniMax 或 DeepSeek。

---

## 错误响应格式

两者格式一致，均为：
```json
{"error": {"message": "...", "type": "...", "code": "..."}}
```
HTTP 状态码对应错误类型（401 认证失败、400 参数错误、422 格式错误等）。