# GLM

## 概述

GLM 供应商对接使用 **OpenAI 协议**。在 Anthropic 协议下，简单对话场景会丢失 thinking block——响应中只有 `type: text` 内容块，推理过程不返回。OpenAI 协议下 `reasoning_content` 为独立字段，thinking 与文本回复干净分离。

## 架构

### 推荐协议

OpenAI。理由：
- Anthropic 协议 simple 场景 `content: [{type:"text", text:"..."}]`，thinking 完全丢失
- Anthropic 协议 tool-use 场景需主动配置 `thinking` 参数才能保留 thinking block
- OpenAI 协议下 `choices[].message.reasoning_content` 为独立字段，与 `content` 互不干扰
- 标准协议映射见 [protocol-mapping](../protocol-mapping.md)

### thinking 行为

- **OpenAI 协议**：通过 `extra_body.thinking.type` 控制。设为 `"enabled"` 时 `reasoning_content` 字段出现；设为 `"disabled"` 时 `reasoning_content` 为空
- **Anthropic 协议**：需传 `thinking` 参数主动启用；不传时 simple 场景 thinking block 完全丢失，仅 tool-use 场景可能返回
- 极短的 reasoning_content（如仅空白或零散字符）不视为推理块，按普通文本处理

### 用量/配额

GLM 提供独立的用量查询 API（`GET /api/monitor/usage/quota/limit`），返回 Coding Plan 套餐等级和多维限额。限额包含 `TIME_LIMIT`（按小时/月/周的时间窗口）和 `TOKENS_LIMIT`（按时间窗口的 token 数），含已用百分比和下次重置时间。

## 数据流

```
Session 层构建请求
  → LLM Client 转发
    → GLM Plugin 注入 extra_body.thinking 参数
    → Protocol 层（OpenAI）序列化请求
    → Provider 层发送至 GLM API
```

## 模块关系

- **上游**：LLM Client（通过 OpenAI 协议路径调用）
- **下游**：GLM API（`https://open.bigmodel.cn`）、用量查询 API
- **引用**：[protocol-mapping](../protocol-mapping.md) — 协议→统一块映射规则
