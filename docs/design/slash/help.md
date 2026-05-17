# 帮助

## 概述

`/help` 指令用于动态生成所有已注册指令的帮助文本。标记为 Immediate 指令，LLM 运行时也能响应。

## 架构

HelpHandler 遍历 HandlerRegistry 中的所有注册条目，提取每个 Handler 的指令名和描述，动态生成帮助文本。新增指令自动出现在帮助中，无需手动维护。

```
/help
  ↓
HelpHandler 遍历 HandlerRegistry
  ↓
提取每个 Entry 的 commands + description
  ↓
格式化为帮助文本
  ↓
返回 Reply(帮助文本)
```

## 数据流

- **输入**：无参数
- **处理**：遍历 HandlerRegistry，收集所有已注册指令的名称和描述
- **输出**：Reply 包含指令列表及说明

## 模块关系

- **上游**：Gateway → Dispatcher → HelpHandler
- **下游**：HandlerRegistry（读取已注册指令信息）
- **无关**：LLM 对话流程
