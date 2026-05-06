# DeepSeek Fixtures

## 推荐 API：Anthropic（略优）

DeepSeek 两种协议表现接近，Anthropic 因有 `signature` 字段可用于追溯，略优。

| 协议 | Thinking 格式 | 推荐场景 |
|------|-------------|---------|
| **Anthropic** ✅ | `content: [{type:"thinking", signature:"..."}, {type:"text"}]` | 优先使用 |
| OpenAI | `reasoning_content` 独立字段，content 干净 | 也可接受 |

## 目录结构

```
deepseek/
├── openai/                    # 2 模型 × 12 场景 = 24 个文件
│   ├── deepseek-v4-flash-simple.json
│   ├── deepseek-v4-flash-streaming.json
│   ├── deepseek-v4-flash-multi-turn.json
│   ├── deepseek-v4-flash-cache.json
│   ├── deepseek-v4-flash-deepseek-thinking-high.json   # reasoning_effort: high
│   ├── deepseek-v4-flash-deepseek-thinking-disabled.json # reasoning_effort: low
│   ├── deepseek-v4-flash-tool-use.json
│   ├── deepseek-v4-flash-tool-result.json              # 2轮对话
│   ├── deepseek-v4-flash-tool-use-streaming.json
│   ├── deepseek-v4-flash-error-model.json
│   ├── deepseek-v4-flash-error-empty.json
│   ├── deepseek-v4-flash-error-tool-format.json
│   └── deepseek-v4-pro-*.json                          # Pro 模型同上
└── anthropic/                  # 2 模型 × 8 场景 = 16 个文件
    ├── deepseek-v4-flash-anthropic-simple.json
    ├── deepseek-v4-flash-anthropic-thinking.json
    ├── deepseek-v4-flash-anthropic-streaming.txt
    ├── deepseek-v4-flash-anthropic-tool-use.json
    ├── deepseek-v4-flash-anthropic-tool-result.json   # 2轮，preserve thinking
    ├── deepseek-v4-flash-anthropic-tool-use-streaming.txt
    ├── deepseek-v4-flash-anthropic-error-model.json
    ├── deepseek-v4-flash-anthropic-error-empty.json
    └── deepseek-v4-pro-*.json                          # Pro 模型同上
```

## 关键场景说明

### Thinking 格式

**OpenAI** — 看 `deepseek-v4-flash-simple.json`：
```json
"choices": [{
  "message": {
    "content": "Hello my friend",
    "reasoning_content": "We need to say hello in 3 words..."
  }
}]
```
`reasoning_content` 和 `content` 独立，格式清晰。

**Anthropic** — 看 `deepseek-v4-flash-anthropic-simple.json`：
```json
"content": [
  {"type": "thinking", "thinking": "We need to respond...", "signature": "cb11..."},
  {"type": "text", "text": "Hello, my friend."}
]
```
thinking 有 `signature` 字段，可用于多轮对话追溯。

---

### reasoning_effort 参数

`deepseek-thinking-high`：发送 `reasoning_effort: "high"`，reasoning_tokens 约 200+
`deepseek-thinking-disabled`：发送 `reasoning_effort: "low"`，仍返回 `reasoning_content`（约 68-92 tokens）

**重要**：DeepSeek 无法真正关闭 thinking。`reasoning_effort: "low"` 只是减少 reasoning token，不是关闭。

---

### 工具调用多轮

**OpenAI** — 看 `deepseek-v4-flash-tool-result.json`：
- Round 1：`tool_calls` + `reasoning_content`
- Round 2：`reasoning_content` 必须回传

**Anthropic** — 看 `deepseek-v4-flash-anthropic-tool-result.json`：
- Round 1：`content: [{type:"thinking", signature:"..."}, {type:"tool_use", ...}]`
- Round 2：`{'role':'assistant', 'content': [thinking, tool_use]}` 整体回传，再接 tool_result
- 关键：必须回传**完整 content 数组**（含 thinking），否则 400：`thinking content must be passed back`

---

### 流式 SSE 格式

**OpenAI** — 看 `deepseek-v4-flash-streaming.txt`：
- 首个 chunk：`delta: {role:"assistant", content:null, reasoning_content:""}`
- reasoning 和 content 通过**互斥的 delta 字段**交替输出
- reasoning 结束时 `reasoning_content: null`，content 开始
- 终止：`[DONE]`

**Anthropic** — 看 `deepseek-v4-flash-anthropic-streaming.txt`：
- 事件序列：`message_start` → `content_block_start(type=thinking)` → `content_block_delta(type=thinking_delta)` → ... → `message_stop`
- `ping` 事件穿插
- 事件总数：Flash 约 233 条，Pro 约 434 条（Pro 输出更长）

---

### Cache 机制（被动）

DeepSeek 支持被动缓存，无主动控制。

**OpenAI** — `deepseek-v4-flash-cache.json`：
```json
"usage": {
  "prompt_cache_hit_tokens": 0,
  "prompt_cache_miss_tokens": 14,
  "prompt_tokens": 14
}
```

**Anthropic**：
```json
"usage": {
  "cache_creation_input_tokens": 0,
  "cache_read_input_tokens": 0,
  "input_tokens": 11
}
```

注意：采集的 fixture 中 cache 全为 0，因为 prompt 太短未触发缓存阈值。

---

## 模型列表

| 模型 | 场景覆盖 |
|------|---------|
| `deepseek-v4-flash` | 全场景 12 OpenAI + 8 Anthropic |
| `deepseek-v4-pro` | 全场景 12 OpenAI + 8 Anthropic |

**Flash vs Pro**：接口行为完全一致，Pro 输出更详细（reasoning_tokens 更多，streaming chunks 更多），是模型能力差异而非接口差异。

---

## 错误响应格式

```json
{"error": {"message": "...", "type": "invalid_request_error", "code": "invalid_request_error"}}
```

| 场景 | HTTP 状态码 |
|------|------------|
| `error-model` | 400 |
| `error-empty` | 400 |
| `error-tool-format` | 400 |

DeepSeek 不返回 `error-auth`（因安全考虑），但 401 会返回：
```json
{"error": {"message": "Authentication Fails...", "type": "authentication_error", "code": "invalid_request_error"}}
```