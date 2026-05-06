# MiniMax Fixtures

## 推荐 API：Anthropic

MiniMax 在 OpenAI 和 Anthropic 两种协议下表现差异显著：

| 协议 | Thinking 格式 | 推荐场景 |
|------|-------------|---------|
| **Anthropic** ✅ | `content: [{type:"thinking"}, {type:"text"}]` 独立拆开 | 优先使用 |
| OpenAI | `content: "‖think...‖\n\n实际回复"` 含标签混在一起 | 仅在必须用 OpenAI 时使用 |

## 目录结构

```
minimax/
├── openai/
│   ├── MiniMax-M2.7-simple.json              # 基础 single-turn
│   ├── MiniMax-M2.7-streaming.txt             # SSE 流式原始文本
│   ├── MiniMax-M2.7-streaming-meta.json       # 流式 metadata
│   ├── MiniMax-M2.7-minimax-reasoning-split.json  # reasoning_split=true
│   ├── MiniMax-M2.7-tool-use.json             # 工具调用
│   ├── MiniMax-M2.7-tool-result.json          # 工具调用多轮
│   ├── MiniMax-M2.7-tool-use-streaming.txt    # 流式工具调用原始 SSE
│   ├── MiniMax-M2.7-multi-turn.json
│   ├── MiniMax-M2.7-cache.json                 # prompt_tokens_details.cached_tokens
│   └── MiniMax-M2.7-error-*.json               # 错误场景
└── anthropic/
    ├── MiniMax-M2.7-anthropic-simple.json
    ├── MiniMax-M2.7-anthropic-thinking.json    # thinking block 独立
    ├── MiniMax-M2.7-anthropic-streaming.txt
    ├── MiniMax-M2.7-anthropic-tool-use.json
    ├── MiniMax-M2.7-anthropic-tool-result.json
    ├── MiniMax-M2.7-anthropic-tool-use-streaming.txt
    ├── MiniMax-M2.7-anthropic-cache.json       # cache_control:ephemeral
    └── MiniMax-M2.7-anthropic-error-*.json
```

## 关键场景说明

### Thinking 格式（OpenAI vs Anthropic 对比）

**OpenAI** — 看 `MiniMax-M2.7-simple.json`：
```json
"content": "\n\n\n\nHello there, friend"
```
Thinking 混在 `content` 开头，用 `` 标签包裹，解析困难。

**Anthropic** — 看 `MiniMax-M2.7-anthropic-simple.json`：
```json
"content": [
  {"type": "thinking", "thinking": "The user asks: \"Say hello in 3 words.\" ..."},
  {"type": "text", "text": "Hello there, friend"}
]
```
清晰拆开，直接取 `content[1].text` 即为最终回复。

**结论**：对接 MiniMax 选 Anthropic 协议。

---

### reasoning_split 模式（OpenAI）

看 `MiniMax-M2.7-minimax-reasoning-split.json`：
```json
"extra_body_sent": {"reasoning_split": true}
```
响应中：
- `choices[].message.content` 仍含 `` 标签
- `choices[].message.reasoning_details` 数组独立出现（每步推理的详情）

---

### 工具调用多轮（Anthropic）

看 `MiniMax-M2.7-anthropic-tool-result.json`：
- Round 1：`content: [{type:"thinking"}, {type:"tool_use", name:"get_weather", input:{...}}]`
- Round 2：`content: [{type:"thinking"}, {type:"text", text:"回复内容"}]`

工具调用后 thinking 仍保留（`reasoning_split` 相关）。

---

### 流式 SSE 格式

**OpenAI 流式** — 看 `MiniMax-M2.7-streaming.txt`：
- 首个 chunk 有 `delta.role: "assistant"`
- `delta.content` 含 `` 标签（thinking 内容）
- `[DONE]` 为终止信号

**Anthropic 流式** — 看 `MiniMax-M2.7-anthropic-streaming.txt`：
- 事件序列：`message_start` → `content_block_start` → `content_block_delta` → `content_block_stop` → `message_stop`
- thinking 通过 `content_block_delta` / `type: "thinking_delta"` 增量传输

---

### Cache 机制

**OpenAI** — `MiniMax-M2.7-cache.json`：
```json
"usage": {"prompt_tokens_details": {"cached_tokens": 0}}
```
被动缓存，无法主动控制。

**Anthropic** — `MiniMax-M2.7-anthropic-cache.json`：
```json
"content": [...],
"usage": {"cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}
```
支持 `cache_control:ephemeral` 主动标记缓存断点。

---

## 模型列表

| 模型 | 说明 |
|------|------|
| `MiniMax-M2.7` | 主模型 |
| `MiniMax-M2.7-highspeed` | 高速版 |
| `MiniMax-M2.5` / `MiniMax-M2.5-highspeed` | 中等规模 |
| `MiniMax-M2.1` / `MiniMax-M2.1-highspeed` | 小规模 |
| `MiniMax-M2` | 基础版 |

## 错误响应格式

```json
// OpenAI / Anthropic 统一走 HTTP 4xx
{
  "error": {
    "message": "...",
    "type": "...",
    "code": "..."
  }
}
```

看 `error-auth` / `error-model` / `error-empty` / `error-tool-format` 了解各场景具体错误格式。