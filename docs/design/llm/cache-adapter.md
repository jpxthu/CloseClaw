# 缓存适配器

> **注意**：本文档描述的缓存适配器是 API 缓存优化层（在对话请求热路径上注入供应商缓存参数），与代码中 `src/llm/adapter/` 目录（旧架构 Provider 桥接 `LegacyProviderAdapter`）是完全不同的概念，不要混淆。

## 概述

缓存适配器为不同 LLM 供应商统一缓存策略，最大化 system prompt 静态区的缓存命中，降低 Token 计费成本。不同供应商的缓存机制差异巨大，适配器层的职责是让上游业务代码无需感知供应商差异。

## 架构

缓存适配器采用 Adapter 模式，为每个供应商提供独立的缓存实现。缓存适配器在 Plugin Pipeline 的 `before_request` 之前执行——它是请求发送前的独立前置处理层，不是 Plugin 的一部分。缓存适配器仅处理静态区内容（标记缓存控制参数等），动态区内容透传不做修改。

```
System Prompt 构建
  → 加载静态区内容
  → 组装动态区内容
  → 缓存适配器（请求前置处理，在 Plugin Pipeline 之前）
    ├─ Anthropic：静态区标记 cache_control（显式前缀缓存）
    ├─ Kimi：传入 prompt_cache_key（服务端自动前缀缓存）
    └─ OpenAI / DeepSeek / 其他：标准请求（无显式缓存参数）
  → Plugin Pipeline（before_request → 后续 LLM Client 标准链路）
  → 发送到 LLM API
```

### Anthropic 适配

Anthropic 支持显式前缀缓存。在系统提示的静态区内容块上标记缓存控制参数，服务端对相同前缀的后续请求自动命中缓存。首次请求以全价建立缓存，后续命中仅收取约 10% 的费用。缓存有效期为 5 分钟，可付费延长。

### Kimi 适配

Kimi 支持服务端自动前缀缓存。请求中传入会话标识作为缓存键，服务端据此关联同一会话的请求，自动匹配前缀。命中后 token 费用降低 75%–87%。无需拆分系统提示块，适配成本最低。

### 其他供应商

OpenAI 和 DeepSeek 采用自动 Exact Match 缓存，要求完整 payload 哈希完全匹配。对话历史持续增长的场景下几乎无法命中。这两家不做主动适配，接受低命中率的现实。

> **注意**：缓存策略取决于供应商 API 的缓存特性，与协议格式选择无关。DeepSeek 在对话调用中使用 Anthropic 协议格式（便于签名追溯），但 DeepSeek 的 API 不支持 Anthropic 的显式前缀缓存（cache_control）。缓存适配器按供应商 API 的能力选择策略，不因协议格式而改变。

## 数据流

```
Session 构建请求
  → 系统提示拆分：静态区（role + workspace + tools）／ 动态区（channel_context + session_state）
  → 缓存适配器接收（供应商 ID、静态区内容、动态区内容、会话 ID）
    → Anthropic：静态区标记缓存控制 → 标准协议请求
    → Kimi：注入缓存键到请求体 → 标准协议请求
    → 其他：不做额外处理 → 标准协议请求
  → 进入 LLM Client 标准调用链路
  → 发送请求
  → 从用量字段监控缓存命中（cached_tokens 计数）
```

## 模块关系

- **上游**：system_prompt 模块（提供静态区和动态区内容）、Session 层（提供会话 ID）
- **下游**：LLM Client（由适配器处理后的请求进入统一的 LLM 调用路径）
- **无关**：provider-config-wizard、model-discovery（缓存适配器运行在对话请求热路径上，与配置阶段工具无关）
