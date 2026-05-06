# GLM API Summary

> 来源：https://docs.bigmodel.cn/ 官方文档
> 整理日期：2026-05-06
> 文档版本：Phase 0.3（官方文档核实修正版）

## 修订记录

| 版本 | 日期 | 修正内容 |
|------|------|---------|
| Phase 0.2 | 2026-05-06 | 初始版本 |
| Phase 0.3 | 2026-05-06 | Anthropic URL 路径修正；tool_stream 支持模型修正（GLM-5.1 不支持）；多轮工具调用 Round 2 格式修正 |

---

## 目录

1. [认证与请求](#1-认证与请求)
2. [Chat Completions 请求](#2-chat-completions-请求)
3. [Reasoning/Thinking 参数](#3-reasoningthinking-参数)
4. [响应格式](#4-响应格式)
5. [Streaming 响应](#5-streaming-响应)
6. [错误响应](#6-错误响应)
7. [Cache 机制](#7-cache-机制)
8. [工具调用（Function Calling）](#8-工具调用function-calling)
9. [多轮对话](#9-多轮对话)
10. [厂商特有字段](#10-厂商特有字段)
11. [视觉理解（多模态）](#11-视觉理解多模态)
12. [具体文档 URL 索引](#12-具体文档-url-索引)

---

## 1. 认证与请求

### 1.1 API Key 格式

- 使用 HTTP Bearer Token 认证
- 请求头：`Authorization: Bearer YOUR_API_KEY`
- API Key 在 [智谱AI开放平台](https://bigmodel.cn/usercenter/proj-mgmt/apikeys) 创建

### 1.2 Base URL

| 协议 | Base URL |
|------|----------|
| OpenAI 兼容 | `https://open.bigmodel.cn/api/paas/v4/` |
| Anthropic 兼容 | `https://open.bigmodel.cn/api/anthropic/v1/messages` |
| GLM Coding Plan | `https://open.bigmodel.cn/api/coding/paas/v4/` |

> ⚠️ Coding Plan 端点仅限 Coding 场景，不适用通用 API 场景。
> 来源：[使用概述 - API 端点](https://docs.bigmodel.cn/cn/api/introduction.md)

### 1.3 认证要求

- 不需要单独生成 Authentication Token，直接使用 API Key
- 建议使用环境变量存储 API Key

---

## 2. Chat Completions 请求

### 2.1 端点与方法

| 协议 | 端点 | 方法 |
|------|------|------|
| OpenAI 兼容 | `/paas/v4/chat/completions` | POST |
| Anthropic 兼容 | `/v1/messages` | POST |

### 2.2 必填字段

| 字段 | 类型 | 说明 |
|------|------|------|
| model | string | 模型名称，如 `glm-5.1`、`glm-4.7` |
| messages | array | 对话消息列表，最少 1 条 |

### 2.3 可选字段（OpenAI 协议）

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| stream | boolean | false | 是否启用流式输出 |
| thinking | object | `{"type": "enabled"}` | 深度思考控制 |
| temperature | float | 模型相关 | 采样温度，GLM-5.1/5/4.7/4.6 系列默认 1.0，GLM-4.5 系列默认 0.6，GLM-4 系列默认 0.75；do_sample=false 时不适用 |
| top_p | float | 0.95 | 核采样参数，建议区间 [0.01, 1.0]，建议与 temperature 二选一 |
| max_tokens | integer | 模型相关 | 最大输出 token 数，详见模型参数表 |
| do_sample | boolean | true | 是否启用采样策略。为 false 时忽略 temperature 和 top_p |
| tools | array | - | 可调用的函数列表，最多 128 个 |
| tool_choice | string | "auto" | 工具选择策略，**仅支持 auto** |
| tool_stream | boolean | false | 工具流式输出（仅 **GLM-5 / GLM-5-Turbo / GLM-4.7 / GLM-4.6**，GLM-5.1 **不支持**） |
| stop | array | - | 停止词列表，最多 1 个 |
| response_format | object | `{"type": "text"}` | 输出格式：text 或 json_object |
| request_id | string | 自动生成 | 请求唯一标识符 |
| user_id | string | - | 终端用户 ID，长度 6-128 |

> 来源：[对话补全 - OpenAPI Schema](https://docs.bigmodel.cn/api-reference/模型-api/对话补全.md)

### 2.4 支持的模型列表

**文本模型：**

| 模型 | 上下文 | 最大输出 | 默认 max_tokens |
| :--- | :--- | :--- | :--- |
| glm-5.1 | 200K | 128K | 65536 |
| glm-5 | 200K | 128K | 65536 |
| glm-5-turbo | 200K | 128K | 65536 |
| glm-4.7 | 200K | 128K | 65536 |
| glm-4.7-flashx | 200K | 128K | 65536 |
| glm-4.7-flash | 200K | 128K | 65536 |
| glm-4.6 | 200K | 128K | 65536 |
| glm-4.5-air | 128K | 96K | 65536 |
| glm-4.5-airx | 128K | 96K | 65536 |
| glm-4.5-flash（即将下线） | 128K | 96K | 65536 |
| glm-4-long | 1M | 4K | 16384 |
| glm-4-flashx-250414 | 128K | 16K | 32768 |
| glm-4-flash-250414 | 128K | 16K | 32768 |

**视觉模型：**

| 模型 | 上下文 | 最大输出 | 默认 max_tokens |
| :--- | :--- | :--- | :--- |
| glm-5v-turbo | 200K | 128K | 65536 |
| glm-4.6v | 128K | 32K | 16384 |
| glm-4.6v-flashx | 128K | 32K | 16384 |
| glm-4.6v-flash | 128K | 32K | 16384 |
| glm-4.1v-thinking-flashx | 64K | 16K | 32768 |
| glm-4.1v-thinking-flash | 64K | 16K | 32768 |
| glm-4v-flash | 16K | 1K | 1024 |
| autoglm-phone | 20K | 2048 | - |

> 来源：[模型概览](https://docs.bigmodel.cn/cn/guide/start/model-overview.md)、[核心参数](https://docs.bigmodel.cn/cn/guide/start/concept-param.md)

> 来源：[对话补全 - ChatCompletionTextRequest](https://docs.bigmodel.cn/api-reference/模型-api/对话补全.md)

---

## 3. Reasoning/Thinking 参数

### 3.1 thinking 参数结构

```json
"thinking": {
    "type": "enabled",  // 或 "disabled"
    "clear_thinking": false  // 可选，启用 Preserved Thinking 时使用
}
```

> `clear_thinking` 字段：控制是否清除推理历史。`false` 启用 Preserved Thinking（保留推理过程）。

在 OpenAI 协议下，通过 `extra_body` 传入：
```python
extra_body={
    "thinking": {
        "type": "enabled",
        "clear_thinking": False  // 可选，用于 Preserved Thinking
    }
}
```

### 3.2 支持的模型

- GLM-5.1
- GLM-5
- GLM-5-Turbo
- GLM-5V-Turbo
- GLM-4.7
- GLM-4.6
- GLM-4.5

### 3.3 thinking.type 选项

| 值 | 说明 |
|----|------|
| `enabled` | 启用深度思考。GLM-5.1/5/4.7 系列默认强制思考；GLM-4.6/4.5 自动判断是否需要 |
| `disabled` | 禁用深度思考，直接给出回答 |

### 3.4 响应字段

启用 thinking 后，响应中的 `reasoning_content` 字段包含思考过程：

**非流式响应：**
```json
{
  "choices": [{
    "message": {
      "role": "assistant",
      "content": "最终回答内容",
      "reasoning_content": "思考过程内容..."
    }
  }]
}
```

**流式响应：**
```json
{
  "choices": [{
    "delta": {
      "reasoning_content": "思考过程增量...",
      "content": "回答内容增量..."
    }
  }]
}
```

> 来源：[深度思考 - 响应示例](https://docs.bigmodel.cn/cn/guide/capabilities/thinking.md)

### 3.5 思考模式（Thinking Mode）

#### 3.5.1 交错式思考（Interleaved Thinking）

- 从 GLM-4.5 开始支持
- 允许模型在工具调用之间继续思考
- **必须显式保留 Reasoning content，并在返回工具结果时一并返回**
- 返回时需将 `reasoning_content` 放在 assistant 消息的**顶层字段**中（与 `content` 同级），并在下一条 tool 消息中直接返回工具结果，无需单独传递 reasoning

#### 3.5.2 保留式思考（Preserved Thinking）

- 在 Coding Plan 端点默认开启
- 标准 API 端点默认关闭
- 通过 `clear_thinking: False` 开启
- **必须将完整、未修改的 reasoning_content 传回 API**
- ⚠️ **不得对 reasoning_content 进行重新排序或修改**，否则会降低效果并影响缓存命中。reasoning_content 必须与模型原始生成序列完全一致
- **Round 2 时 assistant 消息格式**：必须包含 `reasoning_content` 和 `tool_calls` 数组（见 8.7 节）
- ⚠️ **不得对 reasoning_content 进行重新排序或修改**，否则会降低效果并影响缓存命中。reasoning_content 必须与模型原始生成序列完全一致

#### 3.5.3 轮级思考（Turn-level Thinking）

- GLM-4.7 新引入
- 同一会话中每轮可独立选择开启/关闭思考
- 不需要手动区分 Interleaved/Preserved Thinking

> 来源：[思考模式](https://docs.bigmodel.cn/cn/guide/capabilities/thinking-mode.md)

---

## 4. 响应格式

### 4.1 OpenAI 兼容协议响应

```json
{
  "id": "xxx",
  "created": 1677652288,
  "model": "glm-5.1",
  "choices": [{
    "index": 0,
    "message": {
      "role": "assistant",
      "content": "回答内容",
      "reasoning_content": "思考过程（如果启用）"
    },
    "finish_reason": "stop"
  }],
  "usage": {
    "completion_tokens": 239,
    "prompt_tokens": 8,
    "prompt_tokens_details": {
      "cached_tokens": 0
    },
    "total_tokens": 247
  }
}
```

### 4.2 Anthropic 兼容协议

- Base URL: `https://open.bigmodel.cn/api/anthropic`
- 使用 `messages.create` 而非 `chat.completions.create`
- 模型名称使用 GLM 模型编码

```python
from anthropic import Anthropic

client = Anthropic(
    api_key="your-zhipuai-api-key",
    base_url="https://open.bigmodel.cn/api/anthropic"
)

message = client.messages.create(
    model="glm-5.1",
    max_tokens=1024,
    messages=[{"role": "user", "content": "Hello!"}]
)
```

> ⚠️ **官方文档缺失**：Anthropic 兼容接口的响应格式在 docs.bigmodel.cn 上**没有详细文档**，以下信息待实测验证（参考 Claude API 兼容页 + Claude 官方文档）：
> 以下信息待实测验证（参考 Claude 官方 API 文档）：
>
> - 响应根字段：`id`（string）、`type: "message"`、`role: "assistant"`、`content[]`（数组）
> - `content[].type` 可能值：`text`（文本内容）、`thinking`（思考过程）
> - `stop_reason`：停止原因，如 `end_turn`、`max_tokens`
> - `usage` 字段结构**未文档化**，可能使用与 OpenAI 协议不同的字段名（如 `input_tokens`/`output_tokens` 而非 `prompt_tokens`/`completion_tokens`）
> - `cache_control` 字段格式**未文档化**
>
> 建议：对 Anthropic 兼容接口进行实测，确认实际响应结构。
>
> 来源：[Claude API 兼容](https://docs.bigmodel.cn/cn/guide/develop/claude/introduction.md)、[Claude 官方响应格式](https://docs.anthropic.com/en/api/messages-response)（参考）

### 4.3 usage 字段说明

| 字段 | 类型 | 说明 |
|------|------|------|
| prompt_tokens | integer | 输入 token 数 |
| completion_tokens | integer | 输出 token 数 |
| total_tokens | integer | 总 token 数 |
| prompt_tokens_details.cached_tokens | integer | 缓存命中的 token 数 |

---

## 5. Streaming 响应

### 5.1 启用方式

```python
stream=True
```

### 5.2 SSE Chunk 格式

```json
data: {"id":"1","created":1677652288,"model":"glm-4.7","choices":[{"index":0,"delta":{"content":"春"},"finish_reason":null}]}

data: {"id":"1","created":1677652288,"model":"glm-4.7","choices":[{"index":0,"delta":{"content":"天"},"finish_reason":null}]}

...

data: {"id":"1","created":1677652288,"model":"glm-4.7","choices":[{"index":0,"finish_reason":"stop","delta":{"role":"assistant","content":""}}],"usage":{"prompt_tokens":8,"completion_tokens":262,"total_tokens":270,"prompt_tokens_details":{"cached_tokens":0}}}

data: [DONE]
```

### 5.3 delta 字段说明

| 字段 | 出现时机 | 说明 |
|------|---------|------|
| delta.content | 始终 | 回答内容增量 |
| delta.reasoning_content | 启用 thinking 时 | 思考过程增量 |
| delta.role | 最后一块 | 固定为 "assistant" |
| finish_reason | 最后一块 | 完成原因，如 "stop" |

### 5.4 使用统计（usage）出现位置

- **仅在最后一块 chunk 中出现**

> 来源：[流式消息](https://docs.bigmodel.cn/cn/guide/capabilities/streaming.md)

---

## 6. 错误响应

### 6.1 HTTP 状态码

| 状态码 | 原因 | 解决方法 |
|--------|------|----------|
| 200 | 业务处理成功 | - |
| 400 | 参数错误 / 文件内容异常 | 检查接口参数 |
| 401 | 鉴权失败或 Token 超时 | 确认 API KEY 正确 |
| 404 | 微调功能未开放 / 任务不存在 | 联系客服或检查 ID |
| 429 | 请求被限流（见 1302/1303/1305 说明） | 降低并发或频率，稍后重试 |
| 434 | API 权限未开放 | 联系客服申请内测 |
| 435 | 文件大小超过 100MB | 使用更小的文件 |
| 500 | 服务器内部错误 | 稍后重试或联系客服 |

> ⚠️ 官方速率限制文档（docs.bigmodel.cn/cn/api/rate-limit.md）**未明确标注 429 对应的业务错误码**。
> 实际限流由业务错误码 1302/1303/1305/1312 标识，HTTP 状态码可能为 200 或 429（取决于具体场景）。

### 6.2 速率限制详细说明

**并发数**：同一时刻正在处理中的请求数量。超出并发上限时触发限流。

| 错误码 | 类型 | 说明 | 建议处理 |
|--------|------|------|----------|
| 1302 | 账户级限流 | 并发数达到上限 | 降低并发、增加排队间隔 |
| 1303 | 请求频率过高 | 短时间请求过于密集 | 减少请求频率 |
| 1305 | 平台级保护 | 模型整体访问激增、算力高负载 | 稍后再试 |
| 1312 | 模型访问量过大 | 特定模型请求量超限 | 切换模型或稍后重试 |

> 来源：[速率限制](https://docs.bigmodel.cn/cn/api/rate-limit.md)

### 6.3 业务错误码

| 错误分类 | 错误码 | 说明 |
|----------|--------|------|
| 身份验证错误 | 1000 | 身份验证失败 |
| | 1001 | Header 中未收到 Authentication 参数 |
| | 1002 | Authentication Token 非法 |
| | 1003 | Authentication Token 已过期 |
| | 1004 | Authentication Token 验证失败 |
| 账户错误 | 1110 | 账户非活动状态 |
| | 1111 | 账户不存在 |
| | 1112 | 账户已被锁定 |
| | 1113 | 账户已欠费 |
| | 1121 | 账户存违规行为 |
| API 调用错误 | 1210 | API 调用参数有误 |
| | 1211 | **模型不存在** |
| | 1212 | 当前模型不支持该调用方式 |
| | 1213 | 未接收到必需参数 |
| | 1214 | 参数非法 |
| | 1215 | 两参数不能同时设置 |
| | 1261 | Prompt 超长 |
| API 策略阻止 | 1301 | 输入/生成内容包含不安全内容 |
| | 1304 | 今日调用次数限额已用完 |
| | 1308 | 达到使用上限 |

### 6.4 错误响应格式

```json
{
  "error": {
    "code": "1002",
    "message": "Authorization Token非法，请确认Authorization Token正确传递。"
  }
}
```

> ⚠️ 使用流式（SSE）调用时，如果 API 在推理过程中异常终止，不会返回错误码，而是在 `finish_reason` 中返回异常原因。
> 来源：[错误码](https://docs.bigmodel.cn/cn/api/api-code.md)

---

## 7. Cache 机制

### 7.1 缓存类型

**被动缓存（自动缓存识别）**
- 系统自动识别重复的上下文内容
- 无需手动配置
- 基于内容相似度触发

### 7.2 缓存字段

```json
{
  "usage": {
    "prompt_tokens": 1200,
    "completion_tokens": 300,
    "total_tokens": 1500,
    "prompt_tokens_details": {
      "cached_tokens": 800
    }
  }
}
```

### 7.3 缓存命中标识

- 字段：`usage.prompt_tokens_details.cached_tokens`
- 表示从缓存中复用的 token 数量
- 缓存命中的 token 按优惠价格计费（通常为标准价格的 50%）

### 7.4 适用场景

- 系统提示词复用（多轮对话中系统提示词不变）
- 重复任务（相似的指令处理相似内容）
- 多轮对话历史（历史消息中的重复信息）

> 来源：[上下文缓存](https://docs.bigmodel.cn/cn/guide/capabilities/cache.md)

---

## 8. 工具调用（Function Calling）

### 8.1 tools 参数格式

```json
{
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "get_weather",
        "description": "获取指定城市的天气信息",
        "parameters": {
          "type": "object",
          "properties": {
            "city": {
              "type": "string",
              "description": "城市名称"
            }
          },
          "required": ["city"]
        }
      }
    }
  ]
}
```

### 8.2 tool_choice

- **仅支持 `auto`**，默认值也是 `auto`
- 不支持强制调用特定函数

### 8.3 响应格式

```json
{
  "choices": [{
    "message": {
      "role": "assistant",
      "content": null,
      "tool_calls": [
        {
          "id": "call_xxx",
          "type": "function",
          "function": {
            "name": "get_weather",
            "arguments": "{\"city\":\"北京\"}"
          }
        }
      ]
    }
  }]
}
```

### 8.4 tool_result 格式

```json
{
  "role": "tool",
  "tool_call_id": "call_xxx",
  "content": "{\"temperature\":\"25°C\",\"condition\":\"晴天\"}"
}
```

### 8.5 工具流式输出（tool_stream）

- 参数：`tool_stream=True`（需配合 `stream=True`）
- 支持模型：GLM-5.1、GLM-5、GLM-5-Turbo、GLM-4.7、GLM-4.6
- 流式响应中 `delta.tool_calls` 包含工具调用增量信息

```python
response = client.chat.completions.create(
    model="glm-4.7",
    messages=[{"role": "user", "content": "北京天气怎么样"}],
    tools=tools,
    stream=True,
    tool_stream=True
)
```

### 8.6 工具调用中的 Thinking

- 支持交错式思考（Interleaved Thinking）
- 在工具调用之间继续思考
- **必须保留并返回 reasoning_content**

### 8.7 多轮工具调用 Round 2 格式（关键修正）

⚠️ **与 MiniMax / DeepSeek 不同**：GLM 在 Round 2 回传时，assistant 消息必须包含完整的 `tool_calls` 数组，而不仅仅是 `content`。

```python
# 错误：只传 content
messages.append({"role": "assistant", "content": "..."})

# 正确：传完整 assistant 消息（含 tool_calls 和 reasoning_content）
messages.append({
    "role": "assistant",
    "content": content,
    "reasoning_content": reasoning,  # 如果启用了 thinking
    "tool_calls": [{"id": "call_xxx", "type": "function", "function": {"name": "...", "arguments": "{}"}}]
})
```

> 来源：[思考模式 - 使用示例](https://docs.bigmodel.cn/cn/guide/capabilities/thinking-mode.md)

> 来源：[工具调用](https://docs.bigmodel.cn/cn/guide/capabilities/function-calling.md)、[工具流式输出](https://docs.bigmodel.cn/cn/guide/capabilities/stream-tool.md)

---

## 9. 多轮对话

### 9.1 上下文记忆方式

- 通过传递完整的 `messages` 历史列表实现
- 不需要 session ID 或特殊处理
- 模型根据 messages 数组中的历史消息理解上下文

### 9.2 历史消息格式

```json
[
  {"role": "system", "content": "你是一个专业的编程助手"},
  {"role": "user", "content": "什么是递归？"},
  {"role": "assistant", "content": "递归是一种..."},
  {"role": "user", "content": "能给我一个 Python 例子吗？"}
]
```

### 9.3 带 Reasoning 的多轮对话

```python
# 第一轮
response = client.chat.completions.create(
    model="glm-4.7",
    messages=messages,
    tools=tools,
    stream=True,
    extra_body={"thinking": {"type": "enabled", "clear_thinking": False}}
)
# ... 处理响应，保留 reasoning_content

# 添加助手消息（必须包含 reasoning_content）
messages.append({
    "role": "assistant",
    "content": content,
    "reasoning_content": reasoning,
    "tool_calls": [...]
})

# 添加工具结果
messages.append({
    "role": "tool",
    "tool_call_id": tool_call.id,
    "content": json.dumps(tool_result)
})

# 第二轮
response = client.chat.completions.create(
    model="glm-4.7",
    messages=messages,
    tools=tools,
    stream=True,
    extra_body={"thinking": {"type": "enabled", "clear_thinking": False}}
)
```

> ⚠️ 返回历史的 `reasoning_content` 以保持推理连贯性。
> 来源：[思考模式 - 使用示例](https://docs.bigmodel.cn/cn/guide/capabilities/thinking-mode.md)

---

## 10. 厂商特有字段

### 10.1 reasoning_content

- 位置：`choices[0].message.reasoning_content`（非流式）或 `choices[0].delta.reasoning_content`（流式）
- 说明：模型的思考过程内容
- 仅在启用 thinking 时出现

### 10.2 clear_thinking

- 位置：`extra_body.thinking.clear_thinking`
- 类型：boolean
- 说明：控制是否清除推理历史。`False` 启用 Preserved Thinking

### 10.3 tool_stream

- 位置：请求参数
- 类型：boolean
- 说明：启用工具流式输出，仅支持特定模型

### 10.4 do_sample

- 位置：请求参数
- 类型：boolean
- 默认值：`true`
- 说明：是否启用采样策略。为 `false` 时忽略 temperature 和 top_p

---

## 11. 视觉理解（多模态）

### 11.1 支持的输入类型

视觉模型（glm-4.6v 等）支持在 `messages[].content` 数组中传入多模态内容：

| type | 说明 | 字段 |
|------|------|------|
| `text` | 文本内容 | `content[].text` |
| `image_url` | 图像（URL 或 Base64） | `content[].image_url.url` |
| `video_url` | 视频 | `content[].video_url.url` |
| `file_url` | 文档（PDF/DOCX 等） | `content[].file_url.url` |

### 11.2 图像输入格式

**网络图片：**
```json
{
  "role": "user",
  "content": [
    {"type": "image_url", "image_url": {"url": "https://example.com/image.jpg"}},
    {"type": "text", "text": "请描述这张图片"}
  ]
}
```

**本地图片（Base64）：**
```json
{
  "role": "user",
  "content": [
    {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,/9j/4AAQ..."}},
    {"type": "text", "text": "这张图片中有哪些文字？"}
  ]
}
```

### 11.3 视频/文档输入

```json
{
  "role": "user",
  "content": [
    {"type": "video_url", "video_url": {"url": "https://example.com/video.mp4"}},
    {"type": "text", "text": "请分析这个视频的主要内容"}
  ]
}
```

> 来源：[视觉理解](https://docs.bigmodel.cn/cn/guide/capabilities/vision.md)

## 12. 具体文档 URL 索引

| 文档内容 | URL |
|---------|-----|
| 对话补全 API | https://docs.bigmodel.cn/api-reference/模型-api/对话补全.md |
| 深度思考 | https://docs.bigmodel.cn/cn/guide/capabilities/thinking.md |
| 思考模式 | https://docs.bigmodel.cn/cn/guide/capabilities/thinking-mode.md |
| 上下文缓存 | https://docs.bigmodel.cn/cn/guide/capabilities/cache.md |
| 工具调用 | https://docs.bigmodel.cn/cn/guide/capabilities/function-calling.md |
| 工具流式输出 | https://docs.bigmodel.cn/cn/guide/capabilities/stream-tool.md |
| 流式消息 | https://docs.bigmodel.cn/cn/guide/capabilities/streaming.md |
| OpenAI API 兼容 | https://docs.bigmodel.cn/cn/guide/develop/openai/introduction.md |
| Claude API 兼容 | https://docs.bigmodel.cn/cn/guide/develop/claude/introduction.md |
| 错误码 | https://docs.bigmodel.cn/cn/api/api-code.md |
| 速率限制 | https://docs.bigmodel.cn/cn/api/rate-limit.md |
| 核心参数 | https://docs.bigmodel.cn/cn/guide/start/concept-param.md |
| 使用概述 | https://docs.bigmodel.cn/cn/api/introduction.md |
| 模型概览 | https://docs.bigmodel.cn/cn/guide/start/model-overview.md |
| 视觉理解 | https://docs.bigmodel.cn/cn/guide/capabilities/vision.md |