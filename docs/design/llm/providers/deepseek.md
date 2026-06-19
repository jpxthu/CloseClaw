# DeepSeek

## 概述

DeepSeek 供应商对接**两种协议均可**，推荐 **Anthropic 协议**（略优）。Anthropic 协议下 thinking 的 `signature` 字段可追溯；OpenAI 协议下 `reasoning_content` 与 `content` 互斥。`reasoning_effort` 参数无法真正关闭 thinking——即使设为 `low` 仍返回推理内容。

## 架构

### 推荐协议

Anthropic（略优）。理由：
- Anthropic 协议下 `content[].type: thinking` 含 `signature` 签名字段，可追溯推理链
- OpenAI 协议下 `choices[].message.reasoning_content` 与 `content` 在同一 message 中共存但互斥
- 两种协议均可正常工作，OpenAI 也可接受
- 标准协议映射见 [protocol-mapping](../protocol-mapping.md)

### thinking 行为

- **OpenAI 协议**：通过 `reasoning_effort` 参数控制（`high` / `low`）。设为 `low` 时仍返回 `reasoning_content`，thinking 无法真正关闭
- **Anthropic 协议**：thinking 以 `content[].type: thinking` 独立输出，含 `signature` 签名字段
- thinking 始终存在（DeepSeek 模型的设计特性）

### 用量/配额

DeepSeek 提供余额查询 API（`GET /user/balance`），返回账户余额（含赠金和充值两部分）。不提供类似 GLM 的时间窗口限额查询。

## 数据流

```
Session 层构建请求
  → LLM Client 转发
    → DeepSeek Plugin 注入 reasoning_effort 参数
    → Protocol 层（Anthropic 或 OpenAI）序列化请求
    → Provider 层发送至 DeepSeek API
```

## 模块关系

- **上游**：LLM Client（通过 Anthropic 或 OpenAI 协议路径调用）
- **下游**：DeepSeek API（`https://api.deepseek.com`）、余额查询 API
- **引用**：[protocol-mapping](../protocol-mapping.md) — 协议→统一块映射规则
