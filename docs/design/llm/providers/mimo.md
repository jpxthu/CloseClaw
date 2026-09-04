# MiMo

## 概述

MiMo（小米 MiMo 开放平台）供应商对接**两种协议均可**，推荐 **OpenAI 协议**（略优）。thinking 由 `thinking.type` 二元开关控制，主模型默认启用、flash 默认禁用，无分级档位（无需 `glm-thinking` 或 `deepseek-thinking-high` 这类独立场景）。

## 架构

### 推荐协议

OpenAI（略优）。理由：
- OpenAI 协议下 `reasoning_content` 为顶层独立字段，与 `content` 干净分离
- Anthropic 协议下 thinking block 独立但 `signature` 为空（无可追溯签名），优势不明显
- OpenAI 路径实现与 GLM 共享代码路径
- 标准协议映射见 [protocol-mapping](../protocol-mapping.md)

### thinking 行为

- **控制方式**：`thinking.type` 二元开关（enabled / disabled），无分级档位
- **档位映射**：off → disabled；low / medium / high / max 降级为 enabled（最高档）
- **供应商默认**（未显式传 `thinking.type` 时）：主模型（mimo-v2.5-pro / mimo-v2.5 / mimo-v2-pro / mimo-v2-omni）enabled；mimo-v2-flash disabled
- **OpenAI 协议**：enabled 时 `reasoning_content` 字段返回；disabled 时不返回
- **Anthropic 协议**：enabled 时 `content[].type: thinking` 存在，`signature` 为空（无可追溯签名）；disabled 时不返回

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

← 非流式响应：Provider 返回 JSON → Protocol 层解析为内部响应结构 → ModelInterpreter 归一化为统一响应
← 流式响应（SSE）：Provider 以 SSE 流读取原始数据块 → Protocol 层解析 SSE 原生事件 → ModelInterpreter 归一化为统一流式事件，推理以 reasoning_content 或 thinking block 承载
```

## 模块关系

- **上游**：LLM Client（通过 OpenAI 或 Anthropic 协议路径调用）
- **下游**：MiMo API（`https://api.xiaomimimo.com`）
- **引用**：[protocol-mapping](../protocol-mapping.md) — 协议→统一块映射规则
