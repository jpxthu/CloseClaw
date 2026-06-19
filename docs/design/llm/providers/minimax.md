# MiniMax

## 概述

MiniMax 供应商对接使用 **Anthropic 协议**。在 OpenAI 协议下，thinking 内容混在 `content` 字段中用 `‖think...‖` 标签包裹，无法与文本回复干净分离；Anthropic 协议下 thinking 以独立的 `type: thinking` 内容块承载，结构清晰。同时 MiniMax 的 Anthropic 接口支持 `cache_control: ephemeral` 主动前缀缓存。

## 架构

### 推荐协议

Anthropic。理由：
- OpenAI 协议下 `choices[].message.content` 包含 `‖think...‖\n\n\n实际回复`，thinking 以特殊标签混在文本回复中，需要额外提取和清理
- Anthropic 协议下 `content: [{type:"thinking", thinking:"..."}, {type:"text", text:"..."}]`，thinking 独立为结构化的内容块
- 标准协议映射见 [protocol-mapping](../protocol-mapping.md)

### thinking 行为

- **OpenAI 协议**：thinking 内容嵌入 `content` 字段，以 `‖think...‖` 标签包裹；`reasoning_split: true` 后 `reasoning_details` 数组独立出现
- **Anthropic 协议**：thinking 以 `content[].type: thinking` 独立输出，包含 `thinking` 文本和 `signature` 签名字段
- 工具调用多轮时需传 `extra_body.reasoning_split: true` 使思维过程以 `reasoning_details` 数组承载

### 缓存机制

Anthropic 接口支持显式前缀缓存，通过在 system prompt 静态区内容块上标记 `cache_control` 控制参数，后续相同前缀的请求命中缓存。缓存有效期为 5 分钟。

## 数据流

```
Session 层构建请求
  → LLM Client 转发
    → 缓存适配器标记静态区 cache_control
    → MiniMax Plugin 注入 extra_body 参数（reasoning_split 等）
    → Protocol 层（Anthropic）序列化请求
    → Provider 层发送至 MiniMax API
```

## 模块关系

- **上游**：LLM Client（通过 Anthropic 协议路径调用）
- **下游**：MiniMax API（`https://api.minimax.chat`）
- **引用**：[protocol-mapping](../protocol-mapping.md) — 协议→统一块映射规则
