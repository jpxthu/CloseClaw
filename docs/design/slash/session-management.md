# 会话管理

## 概述

`/new` 和 `/stop` 指令用于会话运行管理：创建新会话和强制终止当前运行。

## 架构

两个指令由各自独立的 Handler 处理——NewSessionHandler 负责 `/new`，StopHandler 负责 `/stop`。两者行为独立：

- **`/new`**：创建新会话，分配新会话标识（`{agent_id}_{timestamp}_{random_suffix}`，格式定义见 [session/README.md](../session/README.md)），覆盖 SessionManager 的会话路由映射。旧会话保留，后续由 Sweeper 自然归档。新消息自动路由到新会话。
- **`/stop`**：标记为 Immediate 指令，可在 LLM 运行时立即响应。无参数、无标记，固定 Forceful 语义：终止当前 LLM 调用、终止所有工具进程（前台+后台）、清空统一消息队列中的排队消息、级联终止所有子 session。停止的是运行而非会话——对话历史完整保留，session 转 idle 待命，用户可继续对话；会话结束统一走闲置归档（见 [session/README.md](../session/README.md)）。

`/new` 流程：

1. NewSessionHandler 返回 NewSession
2. Gateway 请求 SessionManager 创建新 session（新 ID），覆盖会话路由映射
3. 回复「已创建新 session：{id}」

`/stop` 流程：

1. StopHandler 返回 Stop
2. Gateway 请求 Session 强制停止（固定 Forceful）：cancel 当前 LLM 请求、终止所有工具进程（前台+后台）、清空统一消息队列中的排队消息、级联终止所有子 session（递归整棵 spawn 树）
3. 执行状态归零（LLM、工具、子 Session 追踪全部清空），session 转 idle 待命（会话保留）
4. 回复「已停止当前任务」

## 数据流

- **`/new`**：无参数 → SlashResult::NewSession → Gateway 创建新会话
- **`/stop`**：无参数 → SlashResult::Stop → Gateway 强制终止当前运行并级联清理（见 [session/session-execution.md](../session/session-execution.md) 停止入口节）

`/new` 为非 Immediate 指令，LLM 忙碌时需等待；`/stop` 为 Immediate 指令，LLM 运行时也能立即执行。

## 模块关系

- **上游**：Gateway → Dispatcher → NewSessionHandler / StopHandler
- **下游**：Session 模块（`/new` 创建新会话；`/stop` 强制停止当前运行，固定 Forceful 语义级联终止子 session）
- **无关**：Processor 链（指令在 Gateway 层处理完毕，不进入 LLM）
