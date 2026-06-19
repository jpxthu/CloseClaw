# VolcEngine

## 概述

VolcEngine（火山方舟）供应商对接使用 **OpenAI 协议**。豆包（Doubao）系列模型通过火山方舟平台提供，API 兼容 OpenAI 格式。

## 架构

### 推荐协议

OpenAI。标准协议映射见 [protocol-mapping](../protocol-mapping.md)。

## 数据流

```
Session 层构建请求
  → LLM Client 转发
    → Protocol 层（OpenAI）序列化请求
    → Provider 层发送至 VolcEngine API
```

## 模块关系

- **上游**：LLM Client（通过 OpenAI 协议路径调用）
- **下游**：VolcEngine API
- **引用**：[protocol-mapping](../protocol-mapping.md) — 协议→统一块映射规则
