# GLM Fixtures

## 推荐 API：OpenAI

GLM 在 Anthropic 协议 simple 场景下会**丢失 thinking block**，只能看到 `content: [{type:"text"}]`。

| 协议 | Thinking 处理 | 推荐场景 |
|------|-------------|---------|
| **OpenAI** ✅ | `choices[].message.reasoning_content` 独立字段，`content` 干净 | 优先使用 |
| Anthropic | simple 场景下 thinking block 完全丢失 | 仅工具调用时使用 |

## 目录结构

```
glm/
├── openai/                    # 约 147 个文件，9 模型 × 16 场景
│   ├── glm-5.1-simple.json
│   ├── glm-5.1-streaming.txt
│   ├── glm-5.1-streaming-meta.json
│   ├── glm-5.1-glm-thinking.json              # thinking enabled
│   ├── glm-5.1-glm-thinking-disabled.json     # thinking disabled
│   ├── glm-5.1-glm-tool-result.json           # 工具多轮（必须回传 reasoning_content）
│   ├── glm-5.1-glm-tool-use-streaming.txt     # tool_stream=True 流式工具调用
│   ├── glm-5.1-glm-tool-use-streaming-meta.json
│   ├── glm-5.1-multi-turn.json
│   ├── glm-5.1-cache.json
│   ├── glm-5.1-tool-use.json
│   ├── glm-5.1-tool-use-streaming.txt
│   ├── glm-5.1-tool-use-streaming-meta.json
│   ├── glm-5.1-error-*.json
│   └── glm-4.7/ ...                           # 其他模型
└── anthropic/                  # 约 69 个文件，5 模型 × 14 场景
    ├── glm-5.1-anthropic-simple.json          # ⚠️ 无 thinking block
    ├── glm-5.1-anthropic-thinking.json
    ├── glm-5.1-anthropic-streaming.txt
    ├── glm-5.1-anthropic-streaming-meta.json
    ├── glm-5.1-glm-anthropic-tool-use.json    # 工具调用
    ├── glm-5.1-glm-anthropic-tool-result.json
    └── glm-5.1-anthropic-error-*.json
```

## 关键场景说明

### Thinking 格式

**OpenAI** — 看 `glm-5.1-glm-thinking.json`：
```json
"choices": [{
  "message": {
    "reasoning_content": "1. Analyze...\n2. Plan...\n3. Final Output...",
    "content": "Hello to you!"
  }
}]
```
`reasoning_content` 独立，清晰可用。

`glm-thinking-disabled` 场景：发送 `extra_body.thinking.type = "disabled"`，`reasoning_content` 字段不出现。

**Anthropic** — 看 `glm-5.1-anthropic-simple.json`：
```json
"content": [{"type": "text", "text": "Hello to you!"}]
```
**无 thinking block**，thinking 完全丢失。仅在 `glm-anthropic-tool-use` 场景下（工具调用）会返回 thinking。

---

### 工具调用多轮（OpenAI）

看 `glm-5.1-glm-tool-result.json`：
- Round 1：`tool_calls` + `reasoning_content`（必须回传）
- Round 2：回传 `reasoning_content`，才能继续多轮

**关键**：`extra_body.thinking.clear_thinking: false` 控制是否保留上一轮 reasoning_content。

---

### 流式工具调用（OpenAI）

看 `glm-5.1-glm-tool-use-streaming.txt`：
- `delta.tool_calls` 增量格式：`{"index": 0, "id": "call_xxx", "function": {"name": "get_weather", "arguments": ""}}`
- `delta.reasoning_content` 增量

注意：`glm-5.1` 不支持 `tool_stream: True`，仅 GLM-5 / GLM-4.7 / GLM-4.6 支持。

---

### 工具调用（Anthropic）

看 `glm-5.1-glm-anthropic-tool-use.json`：
```json
"content": [{"type": "tool_use", "name": "get_weather", "input": {...}}]
```
注意：`tools` 参数格式为 `[{name, description, input_schema}]`（无 `type` 和 `function` 层）。

---

## 模型列表

| 模型 | OpenAI | Anthropic | tool_stream |
|------|--------|-----------|-------------|
| `glm-5.1` | ✅ | ✅ | ❌ 不支持 |
| `glm-5` | ✅ | ✅ | ✅ |
| `glm-5-turbo` | ✅ | ❌ | ❌ |
| `glm-4.7` | ✅ | ✅ | ✅ |
| `glm-4.7-flash` | ✅ | ❌ | ❌ |
| `glm-4.7-flashx` | ✅ | ❌ | ❌ |
| `glm-4.6` | ✅ | ✅ | ✅ |
| `glm-4.5-air` | ✅ | ❌ | ❌ |
| `glm-4.5-airx` | ✅ | ❌ | ❌ |

---

## Cache 支持

**GLM 不支持任何 Cache 机制**（Phase 0 文档未覆盖，fixture 中无 cache 相关字段）。
如需 cache 功能，请使用 MiniMax 或 DeepSeek。

---

## 错误响应格式

```json
// OpenAI 走 HTTP 4xx
{"error": {"message": "...", "type": "...", "code": "..."}}

// Anthropic 走 HTTP 4xx
{"error": {"message": "...", "type": "...", "code": "..."}}
```

两者格式一致，HTTP 状态码对应错误类型。