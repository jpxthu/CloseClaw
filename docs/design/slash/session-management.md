# 会话管理

## 概述

`/new` 和 `/stop` 指令用于会话生命周期管理：创建新会话和强行终止当前运行。

## 架构

两个指令由同一个 SessionHandler 处理，但行为独立：

- **`/new`**：创建新会话，分配新会话标识（`{agent_id}_{timestamp}_{random_suffix}`），覆盖 SessionManager 的 key_registry 映射（同一 session_key 指向新 session_id）。旧会话保留，后续由 Sweeper 自然归档。新消息自动路由到新会话。
- **`/stop`**：标记为 Immediate 指令，可在 LLM 运行时立即响应。终止当前 LLM 调用和所有子 agent，清除运行队列。

```
/new
  ↓
SessionHandler 返回 NewSession
  ↓
Gateway → SessionManager.create_new(session_key)
  → 创建新 session（新 ID）
  → key_registry[session_key] = new_session_id  // 覆盖映射
  ↓
回复"已创建新 session：{id}"

/stop
  ↓
SessionHandler 返回 Stop
  ↓
Gateway 终止当前 LLM run + 所有子 agent
  ↓
清除运行队列
  ↓
回复"已停止当前任务"
```

## 数据流

- **`/new`**：无参数 → SlashResult::NewSession → Gateway 创建新会话
- **`/stop`**：无参数 → SlashResult::Stop → Gateway 终止运行并清理

`/new` 为非 Immediate 指令，LLM 忙碌时需等待；`/stop` 为 Immediate 指令，LLM 运行时也能立即执行。

## 模块关系

- **上游**：Gateway → Dispatcher → SessionHandler
- **下游**：Session 模块（`new_session()`、`stop()` 方法）；Agent 模块（子 agent 终止）
- **无关**：Processor 链（指令在 Gateway 层处理完毕，不进入 LLM）
