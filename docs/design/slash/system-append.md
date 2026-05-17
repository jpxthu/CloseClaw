# System Prompt 追加

## 概述

`/system` 指令用于动态管理 system prompt 中的追加区（append section）。追加区位于 system prompt 静态部分的末尾，与 AGENTS.md 等静态内容互不干扰，是独立分区。owner 可通过此指令在运行时增删 system prompt 指令，无需修改配置文件。

## 架构

追加区是 system prompt 中的一个独立分区，位于静态内容末尾、对话历史之前。以标题行开头，内容为 `[N] 内容` 格式的编号列表。由 `/system` 子指令增删，由 system prompt builder 在每次构建 prompt 时读取并拼入。

关键属性：
- 多次 `/system add` 叠加（accumulate），不覆盖
- 持久化在会话状态中，会话恢复时保留
- 不受上下文压缩影响（压缩仅作用于对话历史，静态区不变）
- 与 AGENTS.md 无优先级冲突——二者是独立分区

## 数据流

```
/system add <内容>
  ↓
SystemHandler 返回 SystemAppend::Add(内容)
  ↓
Gateway 调用 session.add_system_append(内容)
  ↓
回复"已追加指令 #N"

/system list 或 /system
  ↓
SystemHandler 从 SlashContext 读取 system_append
  ↓
返回 Reply(编号列表) 或 "无追加指令"

/system clear
  ↓
SystemHandler 返回 SystemAppend::Clear
  ↓
Gateway 调用 session.clear_system_append()
  ↓
回复"已清除 N 条追加指令"
```

`/system add` 无内容时，回复用法提示。

## 模块关系

- **上游**：Gateway → Dispatcher → SystemHandler
- **下游**：Session 模块（`add_system_append()`、`clear_system_append()` 方法）；system prompt builder（构建时读取追加列表并拼入静态区末尾）
- **无关**：AGENTS.md 加载（追加区与 AGENTS.md 是独立分区，无优先级冲突）
