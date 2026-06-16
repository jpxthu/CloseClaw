# LLM Provider Fixtures v2

> Phase 0-5 采集的真实 API 响应 fixture，用于 CloseClaw 功能开发和测试基准。

## 目录结构

```
tests/fixtures/llm/v2/
├── README.md                    # 本文件：整体索引 + 采集流程
├── capture_fixtures.py          # 采集脚本（chat + 非 chat 场景）
├── run_capture.sh               # 采集入口
├── providers.py                 # Provider 配置（端点 URL + 模型列表）
├── minimax/                    # MiniMax fixtures
│   ├── README.md               # MiniMax 专项说明
│   ├── {model}/                 # 按模型分组
│   │   ├── openai/              #   OpenAI 协议响应
│   │   └── anthropic/           #   Anthropic 协议响应
│   └── provider/              # Provider 级别 fixture（不绑定具体模型）
├── glm/                        # GLM fixtures（同上结构）
├── deepseek/                   # DeepSeek fixtures（同上结构）
├── mimo/                       # MiMo fixtures（Xiaomi MiMo 开放平台）
│   ├── README.md               # MiMo 专项说明
│   ├── {model}/                 # 按模型分组
│   │   ├── openai/
│   │   └── anthropic/
│   └── provider/
├── docs/                       # Phase 0 深挖文档
│   ├── minimax-api-summary.md
│   ├── glm-api-summary.md
│   ├── mimo-api-summary.md
│   └── deepseek-api-summary.md
```

## 顶层字段说明

每个 fixture JSON 的顶层字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `protocol` | string | `"openai"` 或 `"anthropic"` |
| `streaming` | bool | 是否为流式响应（流式场景为 `true`，原始 SSE 文本存于同目录 `.txt` 文件） |
| `scenario` | string | 场景名，标识这个 fixture 测试的是什么 |
| `model` | string | 模型名（provider 级别 fixture 用 `provider` 字段替代） |
| `expect` | string | 期望响应类型：`text` / `streaming` / `reasoning` / `tool_calls` 等 |
| `request` | object | 发送的请求体（实际发送的完整 payload） |
| `response` | object | 响应数据对象 |
| `tools_sent` | object[] | （工具调用场景）发送的 tools 定义 |
| `max_tokens_sent` | int | （Anthropic 协议）发送的 `max_tokens` 参数值 |
| `turns` | object[] | （多轮 cache 场景）每轮对话的 `messages` + `response`，替代 `request`/`response` |
| `rounds` | object[] | （工具调用多轮场景）每轮的 `messages` + `response`，与 `turns` 属不同场景 |
| `extra_body_sent` | object | （部分场景）发送的 `extra_body` 参数（如 `reasoning_split`、`thinking`） |
| `system_sent` | string/object | （多轮 cache 场景）发送的 system prompt |

**错误响应**（`expect: "error"`）：`response` 结构为：

| 字段 | 类型 | 说明 |
|------|------|------|
| `response.error` | bool | 固定 `true` |
| `response.http_code` | int | HTTP 状态码 |
| `response.reason` | string | 错误原因描述 |
| `response.body` | object | 响应体（含 `error.message` / `error.type` / `error.code`） |

**流式响应**：`.txt` 文件保存原始 SSE 文本，同名的 `-meta.json` 文件保存请求元数据（无 `response` 字段）。

## 四家 Provider 推荐 API

> 核心结论：**四家最优协议不一致**——OpenAI 阵营（GLM、MiMo）选 OpenAI；Anthropic 阵营（MiniMax、DeepSeek）选 Anthropic。

| Provider | 推荐协议 | 关键原因 |
|----------|---------|---------|
| **MiniMax** | **Anthropic** | OpenAI 下 `content` 含 `` 标签，thinking 混在回复中；Anthropic 下 `content[].type=thinking` 独立拆出。且支持 `cache_control:ephemeral` 主动缓存 |
| **GLM** | **OpenAI** | Anthropic simple 场景**丢失 thinking block**；OpenAI 下 `reasoning_content` 独立字段，回复干净 |
| **DeepSeek** | **Anthropic**（略优） | Anthropic 的 `content[].type=thinking` 有 `signature` 可追溯；OpenAI 的 `reasoning_content` 混在 message 里 |
| **MiMo** | **OpenAI**（略优） | 与 GLM 一致：OpenAI 下 `reasoning_content` 独立字段、回复干净；Anthropic 下 thinking block 独立但 `signature` 始终为空，优势不明显。OpenAI 路径实现更简单 |

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

**MiMo**
- OpenAI：`choices[].message.reasoning_content` 独立字段（**始终存在**，即使未启用 thinking 也有短内容）
- Anthropic：`content: [{type:"text", text:"..."}, {type:"thinking", thinking:"...", signature:""}]` — thinking 独立，但 `signature` 始终为空字符串
- 支持 cache（OpenAI `cached_tokens` / Anthropic `cache_read_input_tokens` 命中递增），无需 `cache_control` 标记
- **结论**：OpenAI 略优（顶层独立字段、实现简单、与 GLM 共享代码路径）；Anthropic 完全可用

## 场景索引

### Chat 场景

| 场景 | 验证内容 | 适用 provider |
|------|---------|--------------|
| `simple` | 基础 single-turn 对话，协议响应格式 | 全部 |
| `streaming` | SSE chunk 格式（`delta.role` / `finish_reason` / `[DONE]`） | 全部 |
| `multi-turn` | 多轮对话上下文处理 | 全部 |
| `cache` | KV Cache 增量命中：多轮对话中前缀缓存递增 | 全部 |
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
| `context-pressure` | 多轮递增长对话，prompt_tokens 随轮次增长 | 全部 |
| `anthropic-tool-use-streaming` | Anthropic SSE 流式工具调用事件序列 | MiniMax |

> **MiMo 特有说明**：MiMo 在所有场景下 thinking 是**默认行为**（不需要 `glm-thinking` / `deepseek-thinking-high` 这类独立场景）。`reasoning_content` 始终存在于 OpenAI 响应，`thinking` block 始终存在于 Anthropic 响应。

### 非 Chat 场景（Provider 级别）

输出到 `{provider}/provider/` 目录。

| 场景 | 验证内容 | 适用 provider |
|------|---------|--------------|
| `model-list` | GET /models 返回的模型列表结构 | 全部 |
| `usage-quota` | 用量/配额/余额 API 响应 | DeepSeek / GLM / MiniMax（MiMo 无此 API） |

## 快速查找

需要验证某功能时，按场景名查找对应 fixture：

```
# 看 OpenAI simple 响应格式
→ minimax/openai/MiniMax-M3-simple.json
→ glm/openai/glm-5.2-simple.json
→ deepseek/openai/deepseek-v4-flash-simple.json
→ mimo/openai/mimo-v2.5-pro/simple.json

# 看 Anthropic simple 响应格式
→ minimax/anthropic/MiniMax-M3-anthropic-simple.json
→ glm/anthropic/glm-5.2-anthropic-simple.json
→ deepseek/anthropic/deepseek-v4-flash-anthropic-simple.json
→ mimo/anthropic/mimo-v2.5-pro/anthropic-simple.json

# 看流式 SSE 原始格式
→ minimax/openai/MiniMax-M3-streaming.txt
→ deepseek/anthropic/deepseek-v4-flash-anthropic-streaming.txt
→ mimo/openai/mimo-v2.5-pro/streaming.txt
→ mimo/anthropic/mimo-v2.5-pro/anthropic-streaming.txt

# 看工具调用响应
→ minimax/openai/MiniMax-M3-tool-use.json
→ deepseek/openai/deepseek-v4-flash-tool-use.json
→ mimo/openai/mimo-v2.5-pro/tool-use.json

# 看推理模式下 thinking 格式
→ minimax/openai/MiniMax-M3-minimax-reasoning-split.json
→ glm/openai/glm-5.2-glm-thinking.json
→ deepseek/openai/deepseek-v4-flash-deepseek-thinking-high.json
→ mimo/anthropic/mimo-v2.5-pro/anthropic-thinking.json   # thinking 默认行为

# 看 Anthropic 流式工具调用事件序列
→ minimax/anthropic/MiniMax-M3-anthropic-tool-use-streaming.txt
```

## Phase 0 深挖文档

各 provider 的完整 API 格式说明见 `docs/` 目录：

- `minimax-api-summary.md` — MiniMax 认证、请求参数、响应格式、工具调用、Cache 机制
- `glm-api-summary.md` — GLM 认证、请求参数、响应格式、工具调用、thinking 参数
- `mimo-api-summary.md` — MiMo 双协议认证、Pay-as-you-go vs Token Plan、thinking 默认行为、模型列表
- `deepseek-api-summary.md` — DeepSeek 认证、请求参数、响应格式、thinking 模式、错误码

每个结论均有具体文档 URL 来源，便于追溯。

### 非 Chat 场景

非 Chat 场景采集 Provider 级别的 API（不绑定具体模型），输出到 `{provider}/provider/` 目录。

**model-list**：采集 `GET /models` 端点的真实返回。供应商的模型列表 API 只返回模型 ID 和所有者信息，不包含能力参数（context window、max output 等）——这些参数由知识库（`src/llm/assets/`）持有。

**usage-quota**：采集各供应商的用量/配额 API 响应。

| 供应商 | 端点 | 返回内容 |
|--------|------|--------|
| GLM | `GET /api/monitor/usage/quota/limit` | Coding Plan 套餐等级、多维限额 |
| DeepSeek | `GET /user/balance` | 账户余额（赠金 + 充值） |
| MiniMax | `GET /v1/token_plan/remains` | Token Plan 订阅剩余额度 |
| MiMo | 无 | — |

GLM 的用量数据与模型无关，按 provider 级别采集一次即可。MiMo 无 usage-quota API，fixture 记录为 `{"skipped": true, "reason": "..."}`。

## 新模型适配流程

供应商发布新模型时的标准适配流程：

1. **Fixture 采集**：运行 `run_capture.sh <provider> <model> <protocol> <api_key> all`，采集全套 chat + 非 chat 场景
2. **参数查取**：从供应商官方文档查取模型能力参数（context window、max output、是否推理模型）
3. **知识库更新**：将参数写入 `src/llm/assets/<provider>.json`
4. **适配验证**：确认 fixture 数据完整（cache 命中 cached_tokens > 0、模型探测包含新模型 ID）