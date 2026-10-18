# 追加区

## 概述

追加区是 System Prompt 末尾的独立分区，不受上下文压缩影响。承载两种独立管理路径的内容：**Owner 动态指令**（通过 `/system` 指令管理，持久化在会话状态中）与**系统注入内容**（如 workflow 上下文，运行时存在，由对应功能模块写入/移除）。

## 架构

追加区（AppendSection）位于 System Prompt 末尾，与静态层 Section 无优先级冲突。追加区、静态层、动态层是三个独立分区，见 [README.md](README.md#架构)。追加区位于边界标记之后，属于动态区字段（`system_dynamic`），不参与静态前缀缓存。

### 两类内容与独立管理路径

追加区内含两个互相独立的子集，管理者和生命周期不同：

| 维度 | Owner 动态指令 | 系统注入内容 |
|------|---------------|-------------|
| 写入者 | Owner 经 `/system add` | 对应功能模块（如 workflow 模块） |
| 移除者 | Owner 经 `/system clear` | 对应功能模块 |
| 持久化 | 写入 SessionCheckpoint 的 `system_appends`，恢复保留 | 运行时内容，不持久化；重启后由功能模块重新注入 |
| `/system clear` 影响 | 清除本子集 | **不影响**（不删除、不修改系统注入内容） |
| 触发重新组装 | 清除时触发 | **不触发**重新组装 |

- **Owner 动态指令**：由 `/system add/clear/list` 管理（仅 Owner 可用），详见 [slash/system-append](../slash/system-append.md)。清除时触发静态层缓存失效与 System Prompt 重新组装（见 [static-layer.md](static-layer.md) §缓存失效）。
- **系统注入内容**：写入与移除由对应功能模块经 ConversationSession 的系统注入接口触发；内容存放在追加区的运行时容器中，不写入 SessionCheckpoint。写入与移除**不触发 System Prompt 重新组装**，也**不清除 Owner 动态指令**。

> 两类内容彼此独立——`/system clear` 只清 Owner 动态指令，不清除系统注入内容；功能模块移除/更新系统注入内容时不影响 Owner 动态指令。

**追加区内部顺序**：系统注入内容在前，Owner 动态指令在后；Owner 动态指令始终是追加区的末尾编号列表。

**访问边界**：`/system add`、`/system list`、`/system clear` 仅 Owner 可用；非 Owner User 调用被拒绝，不读写追加区。Session 路由键含发送者维度，同一 Session 只属于一个 User；追加区内容不跨 Session 共享，因此 Owner 的追加指令不会出现在非 Owner 的 Session 中（[requirements/system_prompt.md 安全性](../../requirements/system_prompt.md#安全性)）。

### 系统注入内容实例：workflow 上下文

workflow 模块在进入 workflow 模式时向追加区写入 workflow context（注入时机、内容与生命周期见 [workflow/session-integration](../workflow/session-integration.md)）：它由 Engine 在模式进入/恢复/压缩后写入、在模式退出时移除，属于追加区的系统注入内容子集，遵循本节约束。通用契约是「重启后由功能模块自行重新注入」；具体注入时机由各功能模块定义。

追加区的 Owner 动态指令持久化在 SessionCheckpoint 中，不受上下文压缩影响。会话恢复时从 checkpoint 重建，之前的 Owner 追加内容完整保留。`/system clear` 触发持久化更新。

## 数据流

### 追加

Owner 动态指令的追加经 `/system add`。系统注入内容由对应功能模块调用 ConversationSession 的系统注入接口写入。两者共用追加区容器，互不清除。

```
/system add <内容>
  ↓
Slash 模块返回 SlashResult::SystemAppend(SystemAppendAction::Add(内容))
  ↓
Gateway 构造 SideEffectContext 并触发 SlashResult 执行 → SlashEffectExecutor 调用 ConversationSession.add_system_append()
  ↓
ConversationSession 更新追加条目列表中的 Owner 动态指令子集
  ↓
CheckpointManager.save() 将 system_appends 写入 SessionCheckpoint
  ↓
下一次 API 调用时，ConversationSession 读取追加条目并拼入追加区
```

### 查看

```
/system 或 /system list
  ↓
SystemHandler 从 SlashContext 读取 system_appends（会话当前的 Owner 动态指令子集）
  ↓
返回编号列表，或「无追加指令」
```

### 清空

Owner 动态指令经 `/system clear` 清空；系统注入内容由对应功能模块移除。

```
/system clear
  ↓
Slash 模块返回 SlashResult::SystemAppend(SystemAppendAction::Clear)
  ↓
Gateway 构造 SideEffectContext 并触发 SlashResult 执行 → SlashEffectExecutor 调用 ConversationSession.clear_system_appends()
  ↓
ConversationSession 清空 Owner 动态指令子集 + 触发静态层缓存全部失效与 System Prompt 重新组装（系统注入内容不受影响）
  ↓
CheckpointManager.save() 持久化空列表
```

系统注入内容的移除（如 workflow 结束）仅更新系统注入子集，**不触发**重新组装，也不改动 Owner 动态指令子集。

## 模块关系

### 上游

- **Slash 模块**：`/system` 指令触发 Owner 动态指令子集的增删查操作。详细交互见 [slash/system-append](../slash/system-append.md)。
- **workflow 模块**：流程启动/结束时向追加区写入/移除系统注入上下文。详见 [workflow/session-integration](../workflow/session-integration.md)。

### 下游

- **ConversationSession**：每次 API 调用时读取追加条目（系统注入内容 + Owner 动态指令）并拼入 System Prompt 末尾。
- **SessionCheckpoint**：`system_appends` 字段仅持久化 Owner 动态指令，恢复时重建；系统注入内容运行时存在，重启或 System Prompt 重建后由功能模块重新注入。

### 无关

- **静态层**：追加区与静态层是独立分区，互不覆盖。`/system clear` 按需求 §F5 触发静态层缓存失效与重新组装，但两者内容相互独立。
- **Session 压缩流程（Compaction）**：压缩不修改 Owner 动态指令；压缩后 System Prompt 重建时，系统注入内容由功能模块重新注入。
