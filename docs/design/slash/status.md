# 状态查询

## 概述

`/status` 指令用于查看当前会话的运行状态，包括模式、模型、推理深度、上下文用量、缓存命中率、缓存读写 token 累计、活跃子 agent 数、工作目录和 system prompt 追加指令列表。

## 架构

StatusHandler 从 SlashContext 读取会话状态并格式化输出。标记为 Immediate 指令，LLM 运行时也能响应。

```
/status
  ↓
StatusHandler 从 SlashContext 读取：
  - current_mode, current_model, current_reasoning
  - context_usage, subagent_count
  - workdir, system_append
  - cache_hit_rate (会话累计缓存命中率，数据来源：会话统计 RunningStats)
  - cache_read_tokens, cache_write_tokens (会话累计值，数据来源同上)
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
- **输出**：Reply 包含当前模式、模型名称、推理深度、上下文用量、缓存命中率（会话累计缓存命中 token / 会话累计 prompt token，数据来源为会话统计 RunningStats）、缓存读写 token 累计、活跃子 agent 数、工作目录、system prompt 追加指令列表

## 模块关系

- **上游**：Gateway → Dispatcher → StatusHandler
- **下游**：Gateway（Reply 经 Gateway 返回用户）
- **无关**：LLM 对话流程（/status 为 Immediate 指令，不经过 LLM 处理）
