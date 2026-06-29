# MiniMax

## 概述

MiniMax 供应商对接以 **Anthropic 协议**为主推荐。M2.7 下 thinking 以独立的 `type: thinking` 内容块默认输出，M3 需显式传 `thinking: {type: "enabled"}` 才产出 thinking 块。MiniMax 不支持 `reasoning_effort` 等 reasoning 等级控制参数。

## 架构

### 推荐协议

| 模型 | 推荐协议 | 说明 |
|------|---------|------|
| MiniMax-M2.7 | Anthropic | thinking 默认以 `type: thinking` 块独立输出，含 `signature` 签名字段 |
| MiniMax-M3 | Anthropic | 需显式传 `thinking: {type: "enabled"}` 才产出 thinking 块；禁掉 thinking 后返回空内容 |
| MiniMax-M2.1 / M2.5 | Anthropic | 理由同 M2.7 |

Anthropic 协议下 `content` 为类型化结构数组，thinking 以 `type: thinking` 块独立于 `type: text` 块输出，`signature` 字段可追溯。OpenAI 协议下 thinking 以 `<think>...</think>` 标签嵌入 `content`，无法干净分离

M3 差异：
- Anthropic 协议下必须显式传 `thinking: {type: "enabled"}` 才产出 `type: thinking` 块；不传时仅产 `type: text`
- 设 `thinking: {type: "disabled"}` 时返回空 content（仅 1 output token）
- 标准协议映射见 [protocol-mapping](../protocol-mapping.md)

### thinking 行为

- **Anthropic 协议（M2.7）**：thinking 以 `content[{type: thinking, thinking: "...", signature: "..."}]` 默认输出。工具调用多轮时，Plugin 向 `extra_body` 注入 `reasoning_split: true` 确保 thinking 以独立块输出
- **Anthropic 协议（M3）**：需显式传 `thinking: {type: "enabled"}` 才产出 `type: thinking` 块；不传仅产 `type: text`；传 `disabled` 返回空
- **OpenAI 协议**：thinking 以 `<think>...</think>` 标签嵌入 `content`。`reasoning_split` 参数不影响响应结构
- MiniMax 不支持 `reasoning_effort` 等 reasoning 等级控制参数；M3 支持 thinking 的 enabled/disabled 二元开关

### 缓存机制

MiniMax 的 Anthropic 接口实际支持显式前缀缓存，通过在 system prompt 静态区内容块上标记 `cache_control` 控制参数，后续相同前缀的请求命中缓存。缓存有效期为 5 分钟。

### 错误处理

MiniMax API 的错误模式与标准 HTTP 状态码不同：**HTTP 始终返回 200**，错误信息在响应 body 的 `base_resp` 字段中。`base_resp.status_code` 为 0 表示成功，非 0 为业务错误。

| 错误码 | 含义 |
|--------|------|
| 1004 | 认证失败（API Key 无效） |
| 1008 | 余额不足 |
| 1002 | 请求频率超限 |
| 2013 | 参数错误 |

## 数据流

```
Session 层构建请求
  → LLM Client 转发
    → 缓存适配器标记静态区 cache_control
    → MiniMax Plugin 注入 extra_body 参数（reasoning_split 等）
    → Protocol 层（Anthropic）序列化请求
    → Provider 层发送至 MiniMax API

← 非流式响应：Provider 返回 JSON → Protocol 层解析 → ModelInterpreter 归一化为统一响应
← 流式响应（SSE）：Provider 以事件流读取 → Protocol 层按 Anthropic SSE 事件序列解析
    事件序列：message_start → content_block_start(thinking) → thinking_delta
    → signature_delta → content_block_stop → content_block_start(text)
    → text_delta → content_block_stop → message_delta → message_stop
```

## 模块关系

- **上游**：LLM Client（通过 Anthropic 协议路径调用）
- **下游**：MiniMax API（`https://api.minimax.chat`）
- **引用**：[protocol-mapping](../protocol-mapping.md) — 协议→统一块映射规则
