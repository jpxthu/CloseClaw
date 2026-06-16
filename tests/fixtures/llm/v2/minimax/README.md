# MiniMax Fixtures

> 模型：`MiniMax-M3`（最新）、`MiniMax-M2.7`（历史）

## 推荐协议：Anthropic

| 协议 | Thinking 处理 | 工具调用 | Cache 字段 | 推荐场景 |
|------|-------------|---------|-----------|---------|
| **Anthropic** ✅ | M2.7: `[{type:"thinking"},{type:"text"}]`；M3: `[{type:"text"}]`（无 thinking） | M2.7: `[{type:"thinking"},{type:"tool_use"}]`；M3: `[{type:"text"},{type:"tool_use"}]` | `usage.cache_read_input_tokens`（存在，即使为 0） | 优先使用 |
| OpenAI | `content` 内含 `<think>...</think>` 标签，混在一起 | `finish_reason: "tool_calls"` + `tool_calls` 数组 | `prompt_tokens_details.cached_tokens`（存在，即使为 0） | 仅在必须用 OpenAI 时使用 |

---

## 各场景响应字段

### simple（基础对话）

**OpenAI** — `openai/simple.json`
- `choices[].message.content`: 含 `<think>...</think>` 标签（thinking 混在回复开头），需自行解析
- `choices[].message.tool_calls`: 无
- `usage.completion_tokens_details.reasoning_tokens`: 有值（推理 token 数）
- `usage.prompt_tokens_details.cached_tokens`: 有字段（值可能为 0）

**Anthropic** — `anthropic/anthropic-simple.json`
- M2.7: `content[0].type = "thinking"`（独立 block）+ `content[1].type = "text"`（最终回复）
- M3: `content[0].type = "text"`（仅文本块，无 thinking block，见上方 M3 行为变更）
- `usage.cache_read_input_tokens`: 有字段（值可能为 0）
- `stop_reason = "end_turn"`

### thinking（推理过程）

**Anthropic** — `anthropic/anthropic-thinking.json`
- `content[0].type = "thinking"`: 含 signature 字段（**M2.7 行为**；M3 下不返回 thinking block，仅 `content: [{type: "text"}]`，见上方 M3 行为变更说明）
- `content[1].type = "text"`: 含 markdown 格式的详细推理步骤（M2.7）
- `stop_reason = "end_turn"`

> OpenAI 协议下无独立 thinking block，thinking 内容嵌入 `choices[].message.content` 的 `<think>` 标签中。

> **M3 行为变化**（重要）：MiniMax-M3 在 Anthropic 协议下**不再返回独立的 thinking block**。`content[]` 只含 `text` block（thinking 信息丢失）。OpenAI 协议仍返回 `<think>` 标签格式。`usage.completion_tokens_details.reasoning_tokens` 仍正常返回。适配 M3 时不应依赖 Anthropic thinking block 解析。


### streaming（SSE 流式）

**OpenAI** — `openai/streaming.txt`
- 事件：`data: {...}` 格式
- 首个 chunk 含 `delta.role: "assistant"`
- thinking 内容通过 `delta.content` 增量输出，含 `<think>` 标签
- 终止：`data: [DONE]`
- `usage` 在 chunk 中为 null，仅在最后一块有

**Anthropic** — `anthropic/anthropic-streaming.txt`
- 事件序列：`message_start` → `content_block_start(index=0, type=thinking)` → `content_block_delta(type=thinking_delta)` → `content_block_stop` → `content_block_start(index=1, type=text)` → `content_block_delta(type=text_delta)` → `message_stop`
- thinking 和 text 分块传输，清晰分离
- 终止：`[DONE]`

### tool-use（工具调用）

**OpenAI** — `openai/tool-use.json`
- `finish_reason = "tool_calls"`
- `choices[].message.tool_calls`: 数组，每个元素含 `id`, `type: "function"`, `function.name`, `function.arguments`
- `choices[].message.content`: 空或含 `<think>...</think>`（thinking 仍在）

**Anthropic** — `anthropic/anthropic-tool-use.json`
- `stop_reason = "tool_use"`
- `content[0]`: M2.7 为 `type="thinking"`；**M3 为 `type="text"`**（thinking 块被替换为文本块，与上方 M3 行为变更一致）
- `content[N].type = "tool_use"`: 含 `id`, `name`, `input` 字段

### tool-result（多轮工具调用）

**OpenAI** — `openai/tool-result.json`
- 两轮：`round1.finish_reason = "tool_calls"`，`round2.finish_reason = "stop"`
- `extra_body_sent: {reasoning_split: true}`

**Anthropic** — `anthropic/anthropic-tool-result.json`
- Round 1: `stop_reason = "tool_use"`，`content`: M2.7 为 `[thinking, tool_use]`；**M3 为 `[text, tool_use]`**
- Round 2: `stop_reason = "end_turn"`，`content`: M2.7 为 `[thinking, text]`；**M3 为 `[text]`**
- 工具结果以 `role: user, content: [{type: "tool_result", tool_use_id: "...", content: "..."}]` 传入 Round 2

### cache（Prompt Cache）

**OpenAI** — `openai/cache.json`
- `usage.prompt_tokens_details.cached_tokens`: 有字段（测试中值均为 0）
- 三个响应示例，无主动 cache control 标记

**Anthropic** — `anthropic/anthropic-cache.json`
- `usage.cache_creation_input_tokens` / `cache_read_input_tokens`: 有字段（测试中值均为 0）
- `cache_control_note`: 标注 `cache_control:ephemeral` 在 system 末尾
- 系统 prompt 支持通过 `cache_control:ephemeral` 主动标记缓存断点

### 错误响应

| 场景 | 文件 | HTTP 状态 |
|------|------|----------|
| Auth 失败 | `openai/error-auth.json` / `anthropic/anthropic-error-auth.json` | 401 |
| 模型不可用 | `openai/error-model.json` / `anthropic/anthropic-error-model.json` | 400/404 |
| 空消息 | `openai/error-empty.json` / `anthropic/anthropic-error-empty.json` | 400 |
| 工具格式错误 | `openai/error-tool-format.json` | 400 |

错误 body 结构：`{"type": "error", "error": {"type": "...", "message": "..."}, "request_id": "..."}`（`error` 内不含 `code` 字段，错误类型在 `error.type`；OpenAI 协议错误额外含 `error.http_code` 字符串）

---

## 目录结构

```
provider/                   # 提供商级别
├── model-list.json         # Anthropic /v1/models 返回
└── usage-quota.json        # Token Plan 余额查询 GET /v1/token_plan/remains

MiniMax-M3/                  # 最新模型（行为有变化，详见上方 M3 变更说明）
├── openai/                 # 15 场景
│   ├── simple.json
│   ├── cache.json
│   ├── context-pressure.json          # M3 新增
│   ├── streaming.txt
│   ├── streaming-meta.json
│   ├── minimax-reasoning-split.json
│   ├── tool-use.json
│   ├── tool-result.json
│   ├── tool-use-streaming.txt
│   ├── tool-use-streaming-meta.json
│   ├── multi-turn.json
│   └── error-auth.json / error-empty.json / error-model.json / error-tool-format.json
└── anthropic/              # 13 场景
    ├── anthropic-simple.json
    ├── anthropic-thinking.json          # ⚠️ M3 下不再含 thinking block
    ├── anthropic-context-pressure.json  # M3 新增
    ├── anthropic-streaming.txt
    ├── anthropic-streaming-meta.json
    ├── anthropic-tool-use.json          # ⚠️ M3 下 thinking 被替换为 text
    ├── anthropic-tool-result.json       # ⚠️ M3 下 thinking 被替换为 text
    ├── anthropic-tool-use-streaming.txt
    ├── anthropic-tool-use-streaming-meta.json
    ├── anthropic-cache.json
    └── anthropic-error-auth.json / anthropic-error-empty.json / anthropic-error-model.json

MiniMax-M2.7/               # 历史模型，结构同上（含完整 thinking block）
├── openai/                 # 14 场景
└── anthropic/              # 12 场景
```

两个模型的 fixture 集**完全平行**（场景名一致），可以并排比较验证 M3 行为变更。
