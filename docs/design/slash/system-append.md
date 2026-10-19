# System Prompt 追加

## 概述

`/system` 指令用于动态管理 system prompt 中的追加区（AppendSection）。追加区是 system prompt 末尾的独立分区，位于动态层之后、对话历史之前，与 AGENTS.md 等静态内容互不干扰。**仅 Owner** 可通过此指令在运行时追加或批量清除追加指令，无需修改配置文件。

## 架构

追加区是 system prompt 末尾的独立分区，位于动态层之后、对话历史之前。以标题行开头，内容为 `[N] 内容` 格式的编号列表。由 `/system` 子指令追加或批量清除，由 system prompt builder 在每次构建 prompt 时读取并拼入。

关键属性：
- 仅 Owner 可用：非 Owner User 调用 `/system` 子指令一律拒绝，不读写追加区；该身份校验在斜杠指令处理层完成，不进入权限审批流程
- 多次 `/system add` 叠加（accumulate），不覆盖
- 持久化在会话状态中，会话恢复时保留
- 不受上下文压缩影响（压缩仅作用于对话历史，追加区独立于对话消息流）
- 与 AGENTS.md 无优先级冲突——二者是独立分区

## 数据流

```
身份校验：仅 Owner → 否则拒绝，不进入后续流程

/system add <内容>
  ↓
SystemHandler 返回 SlashResult::SystemAppend(SystemAppendAction::Add(内容))
  ↓
Gateway 构造 SideEffectContext 并触发 SlashResult 执行 → SlashEffectExecutor 调用 session.add_system_append(内容)
  ↓
回复"已追加指令 #N"

/system list 或 /system
  ↓
SystemHandler 从 SlashContext 读取 system_appends
  ↓
返回 Reply(编号列表) 或 "无追加指令"

/system clear
  ↓
SystemHandler 返回 SlashResult::SystemAppend(SystemAppendAction::Clear)
  ↓
Gateway 构造 SideEffectContext 并触发 SlashResult 执行 → SlashEffectExecutor 调用 session.clear_system_appends()
  ↓
回复"已清除 N 条追加指令"
```

`/system add` 无内容时，回复用法提示。追加内容超过 500 字符时，向用户返回错误提示，不截断。

## 模块关系

- **上游**：Gateway → Dispatcher → SystemHandler（仅 Owner 可执行）
- **下游**：ConversationSession（每次 API 请求时读取追加列表并拼入 System Prompt 末尾，见 [appends.md](../system_prompt/appends.md)）；Session 模块（[session-lifecycle.md](../session/session-lifecycle.md) 定义了 system_appends 的持久化模型，接口方法 `add_system_append()`、`clear_system_appends()`）。
- **无关**：AGENTS.md 加载（追加区与 AGENTS.md 是独立分区，无优先级冲突）；静态层 Section（追加区位于 System Prompt 末尾，动态层之后）
