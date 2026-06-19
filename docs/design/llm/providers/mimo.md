# MiMo

## 概述

MiMo（小米 MiMo 开放平台）供应商对接使用 **OpenAI 协议**（略优）。MiMo 在所有场景下 thinking 是默认行为——`reasoning_content` 始终存在于 OpenAI 响应，thinking block 始终存在于 Anthropic 响应，无需 `glm-thinking` 或 `deepseek-thinking-high` 这类独立场景。

## 架构

### 推荐协议

OpenAI（略优）。理由：
- OpenAI 协议下 `reasoning_content` 为顶层独立字段，始终存在
- Anthropic 协议下 thinking block 独立但 `signature` 始终为空字符串，优势不明显
- OpenAI 路径实现与 GLM 共享代码路径
- 标准协议映射见 [protocol-mapping](../protocol-mapping.md)

### thinking 行为

- **默认行为**：thinking 在所有场景下都存在，不需要额外参数启用
- **OpenAI 协议**：`reasoning_content` 字段始终返回，即使未显式启用 thinking 也会有短推理内容
- **Anthropic 协议**：`content[].type: thinking` 始终存在，`signature` 字段始终为空字符串

### 缓存机制

支持前缀缓存命中（OpenAI `cached_tokens` / Anthropic `cache_read_input_tokens` 递增），无需客户端显式标记 `cache_control`。

### 用量/配额

MiMo 无 usage-quota API。

## 数据流

```
Session 层构建请求
  → LLM Client 转发
    → Protocol 层（OpenAI 或 Anthropic）序列化请求
    → Provider 层发送至 MiMo API
```

## 模块关系

- **上游**：LLM Client（通过 OpenAI 或 Anthropic 协议路径调用）
- **下游**：MiMo API（`https://api.xiaomimimo.com`）
- **引用**：[protocol-mapping](../protocol-mapping.md) — 协议→统一块映射规则
