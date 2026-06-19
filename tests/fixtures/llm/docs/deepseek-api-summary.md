# DeepSeek API Summary

> 来源：https://api-docs.deepseek.com/zh-cn/  
> 文档读取时间：2026-05-06  
> 所有信息来自文档原文，禁止猜测

## 目录

- [1. 认证与请求](#1-认证与请求)
  - [1.1 认证方式](#11-认证方式)
  - [1.2 base_url](#12-base_url)
  - [1.3 端点](#13-端点)
- [2. Chat Completions 请求参数（OpenAI 格式）](#2-chat-completions-请求参数openai-格式)
  - [2.1 必填字段](#21-必填字段)
  - [2.2 可选字段](#22-可选字段)
  - [2.3 thinking 参数](#23-thinking-参数)
  - [2.4 thinking 模式下不支持的参数](#24-thinking-模式下不支持的参数)
  - [2.5 模型](#25-模型)
- [3. 响应格式（OpenAI 兼容）](#3-响应格式openai-兼容)
  - [3.1 完整响应字段](#31-完整响应字段)
  - [3.2 choices[] 字段](#32-choices-字段)
  - [3.3 usage 字段](#33-usage-字段)
- [4. Streaming 响应](#4-streaming-响应)
  - [4.1 SSE chunk 格式](#41-sse-chunk-格式)
  - [4.2 delta 字段互斥规则](#42-delta-字段互斥规则)
  - [4.3 include_usage 块](#43-include_usage-块)
  - [4.4 stream_options](#44-stream_options)
- [5. Anthropic API 兼容格式](#5-anthropic-api-兼容格式)
  - [5.1 base_url](#51-base_url)
  - [5.2 thinking content block 支持状态](#52-thinking-content-block-支持状态)
- [6. 工具调用（Tool Calls）](#6-工具调用tool-calls)
  - [6.1 thinking 模式 vs 非 thinking 模式](#61-thinking-模式-vs-非-thinking-模式)
  - [6.2 strict 模式（Beta）](#62-strict-模式beta)
  - [6.3 streaming tool chunk 格式](#63-streaming-tool-chunk-格式)
- [7. 多轮对话](#7-多轮对话)
  - [7.1 拼接方式](#71-拼接方式)
  - [7.2 reasoning_content 拼接规则](#72-reasoning_content-拼接规则)
- [8. 错误响应](#8-错误响应)
  - [8.1 HTTP 状态码](#81-http-状态码)
  - [8.2 JSON 错误体格式](#82-json-错误体格式)
- [9. Cache 机制](#9-cache-机制)
- [10. 定价与 Token 上限](#10-定价与-token-上限)
- [11. Claude Code 接入](#11-claude-code-接入)
  - [11.1 环境变量配置](#111-环境变量配置)
  - [11.2 模型映射](#112-模型映射)
  - [11.3 CLAUDE_CODE_EFFORT_LEVEL](#113-claude_code_effort_level)
- [12. 文档未覆盖内容（存疑/空白）](#12-文档未覆盖内容存疑空白)
- [13. 文档页面索引](#13-文档页面索引)

---

## 1. 认证与请求

### 1.1 认证方式

| 项目 | 值 |
|------|---|
| Security Scheme Type | `http` |
| HTTP Authorization Scheme | `bearer` |
| 传递方式 | `Authorization: Bearer <API_KEY>` |

来源：`/zh-cn/api/deepseek-api` — Security Scheme Type / HTTP Authorization Scheme

### 1.2 base_url

| 协议格式 | base_url |
|---------|----------|
| OpenAI 兼容 | `https://api.deepseek.com` |
| Anthropic 兼容 | `https://api.deepseek.com/anthropic` |
| Beta（strict 模式） | `https://api.deepseek.com/beta` |

来源：
- OpenAI base_url：`/zh-cn/api_samples/chat_curl` 示例
- Anthropic base_url：`/zh-cn/guides/anthropic_api` 正文
- Beta base_url：`/zh-cn/guides/tool_calls` — strict 模式章节

### 1.3 端点

| 协议 | 端点 | Method |
|------|------|--------|
| OpenAI | `/chat/completions` | `POST` |
| Anthropic | `/v1/messages` | `POST` |

> ⚠️ **存疑**：`/v1/messages` 是 Anthropic API 标准路径，DeepSeek 文档中未显式声明此端点，实际路径需实测确认。

来源：Anthropic API 标准约定 + `/zh-cn/guides/anthropic_api`

---

## 2. Chat Completions 请求参数（OpenAI 格式）

来源：`/zh-cn/api/create-chat-completion` — Request / Body Schema

### 2.1 必填字段

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `messages` | `object[]` | 是 | 对话消息列表，至少 1 条 |
| `model` | `string` | 是 | 模型 ID |

### 2.2 可选字段

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `thinking` | `object` | `{type: "enabled"}` | 控制思考模式开关 |
| `reasoning_effort` | `string` | `"high"` | 推理强度（请求体顶层字段，非 extra_body） |
| `max_tokens` | `integer` | — | 生成 token 上限；最大 384K（见 Pricing） |
| `response_format` | `object` | `{type: "text"}` | `{type: "text"\|"json_object"}` |
| `stop` | `string\|string[]` | — | 最多 16 个停止词 |
| `stream` | `boolean` | `false` | 是否 SSE 流式输出 |
| `stream_options` | `object` | — | `stream=true` 时，含 `include_usage` 字段 |
| `temperature` | `number` | `1` | 采样温度，0-2；**思考模式下不支持** |
| `top_p` | `number` | `1` | 核采样概率，0-1；**思考模式下不支持** |
| `presence_penalty` | `number` | — | **已废弃，传入无效；思考模式下不支持** |
| `frequency_penalty` | `number` | — | **已废弃，传入无效；思考模式下不支持** |
| `tools` | `object[]` | — | 最多 128 个 function 工具 |
| `tool_choice` | `object` | — | `none`/`auto`/`required` |
| `logprobs` | `boolean` | — | 是否返回对数概率 |
| `top_logprobs` | `integer` | — | 每个位置返回 top N token 概率（0-20），需 `logprobs=true` |
| `user_id` | `string` | — | 自定义用户 ID，最大 512 字符，用于内容审查和 KVCache 隔离 |

来源：同上 Schema 表格

### 2.3 thinking 参数

| 字段 | 可选值 | 默认值 | 说明 |
|------|--------|--------|------|
| `thinking.type` | `"enabled"` / `"disabled"` | `"enabled"` | 思考模式开关 |
| `reasoning_effort` | `"high"` / `"max"` / `"low"` / `"medium"` / `"xhigh"` | `"high"` | 推理强度；`low`/`medium`→`high`，`xhigh`→`max` |

> `reasoning_effort` 是请求体顶层字段（**不是** `extra_body`）
> `thinking` 在 OpenAI SDK 中传入方式：`extra_body={"thinking": {"type": "enabled"}}`  
> 原生 curl 中作为请求体顶层字段

来源：`/zh-cn/guides/thinking_mode` — 思考模式开关与思考强度控制

### 2.4 thinking 模式下不支持的参数

设置以下参数不会报错（兼容考虑），但**不会生效**：
- `temperature`
- `top_p`
- `presence_penalty`
- `frequency_penalty`

来源：`/zh-cn/guides/thinking_mode` — 输入输出参数

### 2.5 模型

| 模型 | 说明 |
|------|------|
| `deepseek-v4-flash` | 推荐模型，非思考模式 |
| `deepseek-v4-pro` | 推荐模型，思考模式 |
| `deepseek-chat` | **日后弃用**（无具体日期），对应 deepseek-v4-flash **非思考模式** |
| `deepseek-reasoner` | **日后弃用**（无具体日期），对应 deepseek-v4-flash **思考模式** |

来源：`/zh-cn/quick_start/pricing` — 模型细节注释

---

## 3. 响应格式（OpenAI 兼容）

来源：`/zh-cn/api/create-chat-completion` — Response Schema

### 3.1 完整响应字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | `string` | 对话唯一标识符 |
| `object` | `string` | 固定值 `chat.completion` |
| `created` | `integer` | Unix 时间戳（秒） |
| `model` | `string` | 使用的模型名 |
| `system_fingerprint` | `string` | 后端配置指纹 |
| `choices` | `object[]` | 生成的选择列表 |
| `usage` | `object` | token 用量统计 |

### 3.2 choices[] 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `choices[].index` | `integer` | 选择索引 |
| `choices[].message` | `object` | 生成的消息 |
| `choices[].message.role` | `string` | 固定值 `assistant` |
| `choices[].message.content` | `string` | 消息内容（最终回复） |
| `choices[].message.reasoning_content` | `string` | 思考模式推理过程内容；非思考模式下无此字段 |
| `choices[].message.tool_calls` | `object[]` | 模型调用的工具列表 |
| `choices[].finish_reason` | `string` | 停止原因：`stop`/`length`/`content_filter`/`tool_calls`/`insufficient_system_resource` |
| `choices[].logprobs` | `object` | 对数概率信息（当 `logprobs=true` 时） |

### 3.3 usage 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `usage.prompt_tokens` | `integer` | prompt 的 token 数 |
| `usage.prompt_cache_hit_tokens` | `integer` | 命中缓存的 prompt token 数 |
| `usage.prompt_cache_miss_tokens` | `integer` | 未命中缓存的 prompt token 数 |
| `usage.completion_tokens` | `integer` | 生成的 token 数 |
| `usage.total_tokens` | `integer` | 总 token 数 |
| `usage.completion_tokens_details.reasoning_tokens` | `integer` | 思维链 token 数 |

来源：`/zh-cn/api/create-chat-completion` — Response Schema

---

## 4. Streaming 响应

来源：`/zh-cn/api_samples/thinking_mode_api_example_streaming`

### 4.1 SSE chunk 格式

> ⚠️ **推断格式**：文档未给出原始 SSE 行文本，以下由 Python SDK 处理代码反推重建，**非文档直接来源**，建议实测确认。

```
data: {"id":"...","choices":[{"index":0,"delta":{"content":"...","reasoning_content":"..."},"finish_reason":null}],"usage":null,"created":...,"model":"...","object":"chat.completion.chunk"}

data: [DONE]
```

### 4.2 delta 字段互斥规则

- `delta.reasoning_content` 和 `delta.content` **互斥**：有值时另一个为空
- 区分方式：`if chunk.choices[0].delta.reasoning_content`
- **推理内容**通过 `reasoning_content` delta 累加
- **最终回复**通过 `content` delta 累加
- 每个 delta 块 `finish_reason` 为 `null`，最后一帧为 `stop`/`length` 等

### 4.3 include_usage 块

- `stream_options: {include_usage: true}` 时，在 `data: [DONE]` **之前**插入一个 usage 统计块
- usage 块格式：`choices: []`，`usage` 字段有值，`object: "chat.completion.chunk"`
- 终止信号：`data: [DONE]`（**字符串**，非 JSON）

### 4.4 stream_options

| 字段 | 说明 |
|------|------|
| `include_usage` | 流式响应中是否包含 usage 信息 |

来源：`/zh-cn/api/create-chat-completion` — stream_options 字段

---

## 5. Anthropic API 兼容格式

来源：`/zh-cn/guides/anthropic_api`

### 5.1 base_url

```
https://api.deepseek.com/anthropic
```

### 5.2 thinking content block 支持状态

| Field / Sub-Field | Support Status |
|---|---|
| `content` (string) | Fully Supported |
| `content`, type="text" | Fully Supported |
| `content`, type="text" (cache_control) | Ignored |
| `content`, type="text" (citations) | Ignored |
| `content`, type="image" | Not Supported |
| `content`, type="document" | Not Supported |
| `content`, type="search_result" | Not Supported |
| `content`, type="thinking" | **Supported** |
| `content`, type="redacted_thinking" | **Not Supported** |
| `content`, type="tool_use" | Fully Supported |
| `content`, type="tool_use" (type 字段) | Fully Supported |
| `content`, type="tool_result" | Fully Supported |
| `content`, type="tool_result" (type 字段) | Fully Supported |
| `content`, type="server_tool_use" | Not Supported |
| `content`, type="web_search_tool_result" | Not Supported |
| `content`, type="code_execution_tool_result" | Not Supported |
| `content`, type="mcp_tool_use" | Not Supported |
| `content`, type="mcp_tool_result" | Not Supported |
| `content`, type="container_upload" | Not Supported |

---

## 6. 工具调用（Tool Calls）

来源：`/zh-cn/guides/tool_calls`

### 6.1 thinking 模式 vs 非 thinking 模式

| 项目 | 非思考模式 | 思考模式 |
|------|-----------|---------|
| 工具调用行为 | 直接返回 tool_calls | 支持多轮推理，逐步思考后才返回最终答案 |
| reasoning_content | 不适用 | 工具调用轮次必须回传 reasoning_content |
| 版本要求 | 一直支持 | DeepSeek-V3.2+ |

来源：`/zh-cn/guides/tool_calls` — 非思考模式 / 思考模式章节

### 6.2 strict 模式（Beta）

开启条件：
1. `base_url = "https://api.deepseek.com/beta"`
2. 所有 function 均需设置 `strict: true`
3. 服务端校验 JSON Schema，不符合规范返回错误

支持的 JSON Schema 类型：
- `object`（所有属性 required，`additionalProperties` 必须为 false）
- `string`（支持 `pattern`、`format`；不支持 `minLength`、`maxLength`）
- `number` / `integer`（支持 `const`、`default`、`minimum`、`maximum`、`exclusiveMinimum`、`exclusiveMaximum`、`multipleOf`）
- `boolean`
- `array`（不支持 `minItems`、`maxItems`）
- `enum`
- `anyOf`
- `$ref` 和 `$def`（须配合使用，`$def` 是定义复用关键字，不是独立类型）

支持的 format：`email`、`hostname`、`ipv4`、`ipv6`、`uuid`

来源：`/zh-cn/guides/tool_calls` — strict 模式章节

### 6.3 streaming tool chunk 格式

**文档未涉及**，无相关说明。

---

## 7. 多轮对话

来源：`/zh-cn/guides/multi_round_chat`

DeepSeek `/chat/completions` 是**无状态 API**，服务端不记录上下文，每次请求需自行拼接所有历史消息。

### 7.1 拼接方式

```python
messages = [{"role": "user", "content": "What's the highest mountain?"}]
response = client.chat.completions.create(model="deepseek-v4-pro", messages=messages)
messages.append(response.choices[0].message)  # 直接 append 整个 message 对象

messages.append({"role": "user", "content": "What is the second?"})
response = client.chat.completions.create(model="deepseek-v4-pro", messages=messages)
messages.append(response.choices[0].message)
```

### 7.2 reasoning_content 拼接规则

**无工具调用时**：API 忽略传入的 `reasoning_content`，只需整个 message 对象 append 即可。

**有工具调用时**：
- `reasoning_content` **必须参与上下文拼接**，后续所有 user 交互轮次必须回传
- 未正确回传 → API 返回 **400 报错**

来源：`/zh-cn/guides/thinking_mode` — 多轮对话拼接章节

---

## 8. 错误响应

来源：`/zh-cn/quick_start/error_codes`

### 8.1 HTTP 状态码

| HTTP Status | 描述 | 原因 | 解决方法 |
|-------------|------|------|---------|
| 400 | 格式错误 | 请求体格式错误 | 根据错误信息提示修改 |
| 401 | 认证失败 | API key 错误 | 检查 API key 是否正确 |
| 402 | 余额不足 | 账号余额不足 | 前往充值 |
| 422 | 参数错误 | 请求体参数错误 | 根据错误信息修改参数 |
| 429 | 请求速率超限 | TPM 或 RPM 达到上限 | 合理规划请求速率 |
| 500 | 服务器故障 | 服务器内部故障 | 等待后重试 |
| 503 | 服务器繁忙 | 服务器负载过高 | 稍后重试 |

### 8.2 JSON 错误体格式

**文档未定义** `error.code` / `error.type` / `error.message` 等嵌套字段格式，仅提供 HTTP 状态码层面说明。

---

## 9. Cache 机制

来源：`/zh-cn/api/create-chat-completion` — usage 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `usage.prompt_cache_hit_tokens` | `integer` | 命中缓存的 prompt token 数 |
| `usage.prompt_cache_miss_tokens` | `integer` | 未命中缓存的 prompt token 数 |

**被动缓存**：prompt_cache_hit_tokens 由服务端自动记录，无主动 cache control 字段。

---

## 10. 定价与 Token 上限

来源：`/zh-cn/quick_start/pricing`

| 维度 | 数值 |
|------|------|
| 上下文长度 | 1M tokens |
| 输出上限（max_tokens） | 384K tokens |
| max_tokens 默认值 | **无明确默认值**（Schema 为 `nullable`，未声明 default 值） |

---

## 11. Claude Code 接入

来源：`/zh-cn/quick_start/agent_integrations/claude_code`

### 11.1 环境变量配置

```bash
export ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic
export ANTHROPIC_AUTH_TOKEN=<你的 DeepSeek API Key>
export ANTHROPIC_MODEL=deepseek-v4-pro[1m]
export ANTHROPIC_DEFAULT_OPUS_MODEL=deepseek-v4-pro[1m]
export ANTHROPIC_DEFAULT_SONNET_MODEL=deepseek-v4-pro[1m]
export ANTHROPIC_DEFAULT_HAIKU_MODEL=deepseek-v4-flash
export CLAUDE_CODE_SUBAGENT_MODEL=deepseek-v4-flash
export CLAUDE_CODE_EFFORT_LEVEL=max
```

### 11.2 模型映射

| Claude 等级 | DeepSeek 映射 |
|------------|--------------|
| Opus | deepseek-v4-pro |
| Sonnet | deepseek-v4-pro |
| Haiku | deepseek-v4-flash |
| Subagent | deepseek-v4-flash |

### 11.3 CLAUDE_CODE_EFFORT_LEVEL

- 设置值：`max`（文档无其他档位说明）
- 各档语义（max/medium/min）：**文档未说明**

---

## 12. 文档未覆盖内容（存疑/空白）

以下信息在官方文档中**未找到**，需实际调用测试或联系 DeepSeek 确认：

| 项目 | 说明 |
|------|------|
| `thinking.type` 完整可选项 | 除 `"enabled"/"disabled"` 外是否支持其他值 |
| `reasoning_effort` 完整枚举 | Schema enum：`high`/`max`；`low`/`medium`/`xhigh` 为兼容性映射（非 enum 值），文档明确说明 |
| `max_tokens` 默认值 | 文档未说明 |
| streaming SSE chunk 完整格式规范 | 仅有 delta 处理代码，无 data: 行格式说明 |
| streaming 下 tool chunk 格式 | 文档未涉及 |
| `usage.reasoning_tokens` 完整字段格式 | Schema 中有 `completion_tokens_details.reasoning_tokens` |
| JSON 错误体结构（error.code/type/message） | 仅 HTTP 状态码，无嵌套 JSON 格式 |

---

## 13. 文档页面索引

| 页面 | URL |
|------|-----|
| 首页 | `https://api-docs.deepseek.com/zh-cn/` |
| API Schema | `https://api-docs.deepseek.com/zh-cn/api/create-chat-completion` |
| Auth 安全方案 | `https://api-docs.deepseek.com/zh-cn/api/deepseek-api` |
| Thinking Mode | `https://api-docs.deepseek.com/zh-cn/guides/thinking_mode` |
| Tool Calls | `https://api-docs.deepseek.com/zh-cn/guides/tool_calls` |
| Multi-round Chat | `https://api-docs.deepseek.com/zh-cn/guides/multi_round_chat` |
| Anthropic API | `https://api-docs.deepseek.com/zh-cn/guides/anthropic_api` |
| Claude Code 接入 | `https://api-docs.deepseek.com/zh-cn/quick_start/agent_integrations/claude_code` |
| Error Codes | `https://api-docs.deepseek.com/zh-cn/quick_start/error_codes` |
| Pricing | `https://api-docs.deepseek.com/zh-cn/quick_start/pricing` |
| Chat curl 示例 | `https://api-docs.deepseek.com/zh-cn/api_samples/chat_curl` |
| Thinking 非流式示例 | `https://api-docs.deepseek.com/zh-cn/api_samples/thinking_mode_api_example_non_streaming` |
| Thinking 流式示例 | `https://api-docs.deepseek.com/zh-cn/api_samples/thinking_mode_api_example_streaming` |
| Thinking 工具调用示例 | `https://api-docs.deepseek.com/zh-cn/api_samples/thinking_mode_api_example_tool_call` |