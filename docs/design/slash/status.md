# 状态查询

## 概述

`/status` 指令用于查看当前会话的运行状态，包括模式、模型、上下文用量、活跃子 agent 数、工作目录和 system prompt 追加内容。

## 架构

StatusHandler 从 SlashContext 读取会话状态并格式化输出。标记为 Immediate 指令，LLM 运行时也能响应。

```
/status
  ↓
StatusHandler 从 SlashContext 读取：
  - current_mode, current_model
  - context_usage, subagent_count
  - workdir, system_append
  ↓
格式化为状态文本
  ↓
返回 Reply(状态文本)
  ↓
Gateway 直接回复用户
```

## 数据流

- **输入**：无参数
- **处理**：读取 SlashContext 中的会话状态字段
- **输出**：Reply 包含当前模式、模型名称、上下文用量、活跃子 agent 数量、工作目录、system prompt 追加指令列表

## 模块关系

- **上游**：Gateway → Dispatcher → StatusHandler
- **下游**：无（仅读取 SlashContext，不触发副作用）
- **无关**：LLM 对话流程
