# 上下文压缩

## 概述

`/compact` 指令用于手动触发对话历史的上下文压缩，将 LLM 上下文中的历史对话进行摘要压缩以释放 token 空间。

## 架构

CompactHandler 接收可选的自定义保留指令，交由 Gateway 调用会话压缩引擎执行。为非 Immediate 指令，LLM 忙碌时需等待。

```
/compact [可选保留指令]
  ↓
CompactHandler 返回 Compact { instruction }
  ↓
Gateway 调用 session.compact(instruction)
  ↓
压缩引擎对对话历史进行摘要
  ↓
回复压缩前后字符数
```

无参数时使用默认压缩策略；带参数时（如 `/compact 保留 API 列表`），自定义保留指令传入压缩引擎，用于指导摘要保留的重点内容。

## 数据流

- **`/compact`**（无参数）：Compact { instruction: None } → 默认压缩
- **`/compact 保留 API 列表`**：Compact { instruction: Some("保留 API 列表") } → 携带保留指令压缩

## 模块关系

- **上游**：Gateway → Dispatcher → CompactHandler
- **下游**：Session 模块（`compact()` 方法，执行上下文压缩引擎）
- **无关**：system prompt 静态区（压缩仅作用于对话历史，静态内容保持不变）
