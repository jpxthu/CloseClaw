# LLM Provider Fixtures

> Phase 0-5 采集的真实 API 响应 fixture，用于 CloseClaw 功能开发和测试基准。

**相关文档**：
- Provider 推荐协议和行为特征 → `docs/design/llm/providers/{provider}.md`
- 协议→统一块映射规则 → `docs/design/llm/protocol-mapping.md`

## 目录结构

```
tests/fixtures/llm/
├── README.md                    # 本文件
├── capture_fixtures.py          # 采集脚本（chat + 非 chat 场景）
├── run_capture.sh               # 采集入口
├── providers.py                 # Provider 配置（端点 URL + 模型列表）
├── {provider}/                  # 各供应商 fixture
│   ├── {model}/                 # 按模型分组
│   │   ├── openai/              #   OpenAI 协议响应
│   │   └── anthropic/           #   Anthropic 协议响应
│   └── provider/                # Provider 级别 fixture（不绑定具体模型）
├── docs/                        # Phase 0 深挖文档（调研记录）
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
| `request` | object | 发送的请求体 |
| `response` | object | 响应数据对象 |
| `tools_sent` | object[] | （工具调用场景）发送的 tools 定义 |
| `max_tokens_sent` | int | （Anthropic 协议）发送的 `max_tokens` 参数值 |
| `turns` | object[] | （多轮场景）每轮对话的 `messages` + `response`，替代 `request`/`response` |
| `rounds` | object[] | （工具调用多轮场景）每轮消息 + 响应 |
| `extra_body_sent` | object | 发送的 `extra_body` 参数 |
| `system_sent` | string/object | 发送的 system prompt |

**错误响应**（`expect: "error"`）：

| 字段 | 说明 |
|------|------|
| `response.error` | 固定 `true` |
| `response.http_code` | HTTP 状态码 |
| `response.reason` | 错误原因描述 |
| `response.body` | 响应体（含 `error.message` / `error.type` / `error.code`） |

**流式响应**：`.txt` 文件保存原始 SSE 文本，同名的 `-meta.json` 文件保存请求元数据（无 `response` 字段）。

## 场景索引

### Chat 场景

| 场景 | 验证内容 |
|------|---------|
| `simple` | 基础 single-turn 对话，协议响应格式 |
| `streaming` | SSE chunk 格式 |
| `multi-turn` | 多轮对话上下文处理 |
| `cache` | KV Cache 增量命中 |
| `tool-use` | `finish_reason=tool_calls`，`tool_calls` 结构 |
| `tool-result` | 工具调用多轮：`tool_call` → `tool_result` → final |
| `tool-use-streaming` | 流式下 `delta.tool_calls` 增量格式 |
| `error-auth` / `error-model` / `error-empty` | 错误响应格式 |
| `error-tool-format` | tools 格式错误的报错格式 |
| `anthropic-simple` / `anthropic-thinking` | Anthropic `content[].type` 格式 |
| `anthropic-streaming` | Anthropic SSE 事件序列 |
| `anthropic-tool-use` / `anthropic-tool-result` | Anthropic `content[].type=tool_use` 格式 |
| `anthropic-tool-use-streaming` | Anthropic SSE 流式工具调用事件序列 |
| `context-pressure` | 多轮递增长对话，prompt_tokens 随轮次增长 |

**Provider 特有场景**：各 provider 的 thinking 控制场景（`glm-thinking`、`deepseek-thinking-high` 等）在各 provider design doc 中说明。

### 非 Chat 场景（Provider 级别）

输出到 `{provider}/provider/` 目录：

| 场景 | 说明 |
|------|------|
| `model-list` | GET /models 返回的模型列表结构 |
| `usage-quota` | 用量/配额/余额 API 响应 |

## 新模型适配流程

1. **Fixture 采集**：运行 `run_capture.sh <provider> <model> <protocol> <api_key> all`
2. **参数查取**：从供应商官方文档查取模型能力参数
3. **知识库更新**：将参数写入 `src/llm/assets/<provider>.json`
4. **适配验证**：确认 fixture 数据完整

### ⚠️ 安全红线

- **API Key 禁止落盘**：Key 只能通过 CLI 参数 `--api-key` 或环境变量 `LLM_API_KEY` 传入采集脚本，绝不出现在任何脚本文件、配置文件、fixture 文件中
- **采集后审查**：每次采集完成后，必须对新增的 fixture 文件做安全审核（`grep` 检查不含 key 片段），确认无凭据泄露才能提交
- **临时文件清理**：采集过程中产生的任何含 key 的中间文件（如 wrapper 脚本）必须立即删除

## Phase 0 深挖文档

各 provider 的完整 API 格式说明见 `docs/` 目录（调研记录，非设计文档）：

- `minimax-api-summary.md` — MiniMax 认证、请求参数、响应格式、工具调用、Cache 机制
- `glm-api-summary.md` — GLM 认证、请求参数、响应格式、工具调用、thinking 参数
- `mimo-api-summary.md` — MiMo 双协议认证、Pay-as-you-go vs Token Plan、thinking 默认行为
- `deepseek-api-summary.md` — DeepSeek 认证、请求参数、响应格式、thinking 模式、错误码
