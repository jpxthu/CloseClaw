# 协议映射

## 概述

协议映射层定义 OpenAI 和 Anthropic 两种 LLM 协议到统一内容块和统一流式事件的转换规则。这是 LLM 模块五层架构中 Protocol 层和 ModelInterpreter 层的桥梁规范——Protocol 层按协议解析原始响应，ModelInterpreter 按此映射将协议原生字段归一化为统一内容块。

此规范中立于具体供应商，只描述协议本身的映射关系。各供应商的特有行为（如 MiniMax OpenAI 协议下 thinking 混在文本中的标签格式）在各 provider 文档中说明。

## 架构

### 统一内容块

所有协议的响应内容归一化为四种内容块类型，上层业务只看到这四种类型：

| 统一块 | 含义 |
|--------|------|
| Text | 模型生成的文本回复 |
| Thinking | 模型的推理过程（对用户不可见） |
| ToolUse | 模型请求调用工具 |
| ToolResult | 工具执行结果（作为历史消息传入） |

### 协议→统一块映射

**OpenAI 协议**：

| 统一块 | 协议来源 |
|--------|---------|
| Text | `choices[].message.content` |
| Thinking | `choices[].message.reasoning_content` |
| ToolUse | `choices[].message.tool_calls[]` |
| ToolResult | 消息数组中以 `role: "tool"` 出现 |

OpenAI 协议下，content 和 reasoning_content 在同一个 message 对象中共存。提取规则：content 非空则优先取 content；content 为空或仅空白时回退到 reasoning_content。

**Anthropic 协议**：

| 统一块 | 协议来源 |
|--------|---------|
| Text | `content[].type: "text"` → `text` |
| Thinking | `content[].type: "thinking"` → `thinking`（含 `signature` 签名字段） |
| ToolUse | `content[].type: "tool_use"` |
| ToolResult | `content[].type: "tool_result"` |

Anthropic 协议下，content 是类型化结构数组。每个元素通过 `type` 字段区分内容块类型，thinking 块独立于 text 块。signature 是 thinking 块的可追溯签名字段，与 thinking 文本一并保留。

### 统一流式事件

流式输出通过五种统一事件传递，屏蔽两种协议 SSE 在事件粒度上的差异：

| 统一事件 | OpenAI SSE 来源 | Anthropic SSE 来源 |
|---------|----------------|-------------------|
| 内容块开始 | 首个 `delta.role` 出现 / `delta.tool_calls[0]` 首次出现 | `content_block_start` 事件 |
| 内容增量 | `delta.content` / `delta.reasoning_content` / `delta.tool_calls` | `content_block_delta` 事件（`text_delta` / `thinking_delta` / `signature_delta`） |
| 内容块结束 | `finish_reason` 首次出现 | `content_block_stop` 事件 |
| 消息结束 | `finish_reason=stop` 后接 `[DONE]` | `message_delta(stop_reason)` 后接 `message_stop` |
| 错误事件 | HTTP 错误状态码 + 错误 body | HTTP 错误状态码 + `error` 事件 |

Anthropic SSE 事件序列的典型顺序：`message_start` → `content_block_start(thinking)` → 若干 `thinking_delta` → 一个 `signature_delta` → `content_block_stop` → `content_block_start(text)` → 若干 `text_delta` → `content_block_stop` → `message_delta` → `message_stop`。

### 多轮对话增量处理

多轮对话中，每轮请求的消息列表是上一轮的追加——历史消息不修改，只追加新消息。这是前缀缓存生效的前提。

多轮场景的 fixture 使用 `turns` 结构记录每轮的完整 messages 数组和对应响应，展示消息列表在各轮之间的增量变化。

### 消息历史缓存

各协议在请求 messages 数组上的缓存标记策略不同。消息历史的缓存标记使下一轮请求的完整前缀（上轮的 system prompt + 全部 messages）命中 cache，仅新增的消息尾部按全价计费。

**Anthropic 协议**：支持显式缓存标记。在 messages 数组的尾部消息上打 `cache_control: {"type": "ephemeral"}`，指向前缀匹配点。每轮请求只在尾部打一个标记——消息数组只追加不修改，前缀稳定使缓存持续命中。

**OpenAI / DeepSeek 协议**：使用服务端自动前缀缓存，无需客户端在 messages 上显式标记。消息数组只追加、前缀不变，历史消息的前缀部分自动被服务端缓存覆盖。

> 消息历史缓存标记属于 Protocol 层的序列化行为，与缓存适配器（处理 system prompt 静态区缓存）职责分离。缓存适配器详见 [cache-adapter](cache-adapter.md)。

## 数据流

```
供应商 API 返回原始响应
  → Provider 层返回 HTTP body（JSON 或 SSE 流）
    → Protocol 层按协议格式解析为内部结构
      → ModelInterpreter 按本映射表归一化为统一内容块/流式事件
        → 上层业务只看到统一模型
```

## 模块关系

- **上游**：Protocol 层（提供按协议解析后的内部结构）
- **下游**：ModelInterpreter（消费映射规则，产出统一内容块和流式事件）
- 本规范被各 provider 文档引用
