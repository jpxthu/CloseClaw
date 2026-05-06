# LLM Provider Fixtures v2

> Phase 0-5 采集的真实 API 响应 fixture，用于 CloseClaw 功能开发和测试基准。

## 目录结构

```
tests/fixtures/llm/v2/
├── README.md                    # 本文件：整体索引
├── minimax/                    # MiniMax fixtures
│   ├── README.md               # MiniMax 专项说明
│   ├── openai/                 # OpenAI 兼容协议响应
│   │   ├── {model}-{scenario}.json
│   │   └── {model}-{scenario}.txt          # 流式原始 SSE
│   └── anthropic/              # Anthropic 兼容协议响应
│       ├── {model}-{scenario}.json
│       └── {model}-{scenario}.txt          # 流式原始 SSE
├── glm/                        # GLM fixtures
│   ├── README.md
│   ├── openai/
│   └── anthropic/
├── deepseek/                   # DeepSeek fixtures
│   ├── README.md
│   ├── openai/
│   └── anthropic/
├── docs/                       # Phase 0 深挖文档
│   ├── minimax-api-summary.md
│   ├── glm-api-summary.md
│   └── deepseek-api-summary.md
└── providers.py                 # Provider 配置（供 capture 脚本使用）
```

## 顶层字段说明

每个 fixture JSON 的顶层字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `protocol` | string | `"openai"` 或 `"anthropic"` |
| `streaming` | bool | 是否为流式响应（流式场景为 `true`，输出为原始 SSE 文本） |
| `scenario` | string | 场景名，标识这个 fixture 测试的是什么 |
| `model` | string | 模型名 |
| `expect` | string | 期望响应类型：`text` / `streaming` / `reasoning` / `tool_calls` 等 |
| `request` | object | 发送的请求体（实际发送的完整 payload） |
| `response_raw` | string | 响应原始文本（JSON 字符串或 SSE 原始文本） |
| `http_status` | int | HTTP 状态码 |
| `error_detail` | object | （错误场景）错误详情 |
| `extra_body_sent` | object | （部分 provider）额外发送的参数 |

## 三家 Provider 推荐 API

> 核心结论：**三方各自最优协议不同，不能统一协议**。

| Provider | 推荐协议 | 关键原因 |
|----------|---------|---------|
| **MiniMax** | **Anthropic** | OpenAI 下 `content` 含 `` 标签，thinking 混在回复中；Anthropic 下 `content[].type=thinking` 独立拆出。且支持 `cache_control:ephemeral` 主动缓存 |
| **GLM** | **OpenAI** | Anthropic simple 场景**丢失 thinking block**；OpenAI 下 `reasoning_content` 独立字段，回复干净 |
| **DeepSeek** | **Anthropic**（略优） | Anthropic 的 `content[].type=thinking` 有 `signature` 可追溯；OpenAI 的 `reasoning_content` 混在 message 里 |

### 详细说明

**MiniMax**
- OpenAI：`choices[].message.content` = `"‖think...‖\n\n\n实际回复"`（标签包裹 thinking）
- Anthropic：`content: [{type:"thinking", thinking:"..."}, {type:"text", text:"..."}]` — thinking 独立
- 工具调用多轮：`reasoning_split: true` 后 `reasoning_details` 数组独立出现
- **结论**：系统对接选 Anthropic 路径

**GLM**
- OpenAI：`choices[].message.reasoning_content` 独立字段，`choices[].message.content` 是干净回复
- Anthropic simple：`content: [{type:"text", text:"..."}]` — **thinking block 完全丢失**
- Anthropic tool-use：返回 `content[].type=tool_use`，需主动配置 `thinking` 参数
- **结论**：系统对接选 OpenAI 路径；Anthropic 模式下需设置 `thinking` 参数保留 thinking

**DeepSeek**
- OpenAI：`choices[].message.reasoning_content` 字段，reasoning 和 content 互斥
- Anthropic：`content: [{type:"thinking", thinking:"...", signature:"..."}, {type:"text", text:"..."}]`
- `reasoning_effort: low` 仍返回 `reasoning_content`（无法真正关闭）
- **结论**：Anthropic 略优（signature 可追溯），OpenAI 也可接受

## 场景索引

| 场景 | 验证内容 | 适用 provider |
|------|---------|--------------|
| `simple` | 基础 single-turn 对话，协议响应格式 | 全部 |
| `streaming` | SSE chunk 格式（`delta.role` / `finish_reason` / `[DONE]`） | 全部 |
| `multi-turn` | 多轮对话上下文处理 | 全部 |
| `cache` | `prompt_tokens_details.cached_tokens` / `prompt_cache_hit_tokens` | MiniMax / DeepSeek |
| `minimax-reasoning-split` | `extra_body.reasoning_split: true` → `reasoning_details` 字段 | MiniMax |
| `glm-thinking` / `glm-thinking-disabled` | `extra_body.thinking.type` → `reasoning_content` 字段 | GLM |
| `deepseek-thinking-high` / `deepseek-thinking-disabled` | `reasoning_effort` → `reasoning_content` 字段 | DeepSeek |
| `tool-use` | `finish_reason=tool_calls`，`tool_calls` 结构 | 全部 |
| `tool-result` | 工具调用多轮：`tool_call` → `tool_result` → final | 全部 |
| `tool-use-streaming` | 流式下 `delta.tool_calls` 增量格式 | 全部 |
| `error-auth` / `error-model` / `error-empty` | 错误响应格式（status code + JSON body） | 全部 |
| `error-tool-format` | `tools` 格式错误（缺少 `function.name`）的报错格式 | 全部 |
| `anthropic-simple` / `anthropic-thinking` | Anthropic `content[].type` 格式 | 全部 |
| `anthropic-streaming` | Anthropic SSE 事件序列 | 全部 |
| `anthropic-tool-use` / `anthropic-tool-result` | Anthropic `content[].type=tool_use` 格式 | 全部 |
| `anthropic-cache` | `cache_control:ephemeral` 主动缓存标记 | MiniMax |
| `anthropic-tool-use-streaming` | Anthropic SSE 流式工具调用事件序列 | MiniMax |

## 快速查找

需要验证某功能时，按场景名查找对应 fixture：

```
# 看 OpenAI simple 响应格式
→ minimax/openai/MiniMax-M2.7-simple.json
→ glm/openai/glm-5.1-simple.json
→ deepseek/openai/deepseek-v4-flash-simple.json

# 看 Anthropic simple 响应格式
→ minimax/anthropic/MiniMax-M2.7-anthropic-simple.json
→ glm/anthropic/glm-5.1-anthropic-simple.json
→ deepseek/anthropic/deepseek-v4-flash-anthropic-simple.json

# 看流式 SSE 原始格式
→ minimax/openai/MiniMax-M2.7-streaming.txt
→ deepseek/anthropic/deepseek-v4-flash-anthropic-streaming.txt

# 看工具调用响应
→ minimax/openai/MiniMax-M2.7-tool-use.json
→ deepseek/openai/deepseek-v4-flash-tool-use.json

# 看推理模式下 thinking 格式
→ minimax/openai/MiniMax-M2.7-minimax-reasoning-split.json
→ glm/openai/glm-5.1-glm-thinking.json
→ deepseek/openai/deepseek-v4-flash-deepseek-thinking-high.json

# 看 Anthropic 流式工具调用事件序列
→ minimax/anthropic/MiniMax-M2.7-anthropic-tool-use-streaming.txt
```

## Phase 0 深挖文档

各 provider 的完整 API 格式说明见 `docs/` 目录：

- `minimax-api-summary.md` — MiniMax 认证、请求参数、响应格式、工具调用、Cache 机制
- `glm-api-summary.md` — GLM 认证、请求参数、响应格式、工具调用、thinking 参数
- `deepseek-api-summary.md` — DeepSeek 认证、请求参数、响应格式、thinking 模式、错误码

每个结论均有具体文档 URL 来源，便于追溯。