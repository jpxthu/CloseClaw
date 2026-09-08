# 追加区

## 概述

追加区是 System Prompt 末尾的独立分区，持久化在会话状态中，不受上下文压缩影响。承载两类内容：**Owner 动态指令**（通过 `/system` 指令管理）与**系统注入内容**（如 workflow 上下文，由对应功能模块触发写入/移除）。

## 架构

AppendSection 位于 System Prompt 末尾，与静态层 Section 无优先级冲突——二者是独立分区。追加区与静态层、动态层是两个独立分区，见 [README.md](README.md#架构)。

### 两类内容与独立管理路径

追加区内含两个互相独立的子集，管理者和生命周期不同：

| 维度 | Owner 动态指令 | 系统注入内容 |
|------|---------------|-------------|
| 写入者 | Owner 经 `/system add` | 对应功能模块（如 workflow 模块） |
| 移除者 | Owner 经 `/system clear` | 对应功能模块 |
| 持久化 | 写入 SessionCheckpoint，恢复保留 | 运行时内容，重启后由功能模块重新注入 |
| `/system clear` 影响 | 清除本子集 | **不影响**（不删除系统注入内容，也不被其依赖） |
| 触发重新组装 | 清除时触发 | **不触发**重新组装 |

- **Owner 动态指令**：由 `/system add/clear/list` 管理，详见 [slash/system-append](../slash/system-append.md)。清除时触发静态层缓存失效与 System Prompt 重新组装（见 [static-layer.md](static-layer.md) §缓存失效）。
- **系统注入内容**：写入与移除由对应功能模块触发，写入与移除**不触发 System Prompt 重新组装**，也**不清除 Owner 动态指令**。重启后需由功能模块重新注入（系统注入内容本身不持久化）。

> 两类内容彼此独立——`/system clear` 只清 Owner 动态指令，不清除系统注入内容；功能模块移除/更新系统注入内容时不影响 Owner 动态指令。

### 系统注入内容实例：workflow 上下文

workflow 模块在进入 workflow 模式时向追加区写入 workflow context（注入时机、内容与生命周期见 [workflow/session-integration](../workflow/session-integration.md)）：它由 Engine 在模式进入/恢复/压缩后写入、在模式退出时移除，属于追加区的系统注入内容子集，遵循本节约束。

追加区持久化在 SessionCheckpoint 中，不受上下文压缩影响。会话恢复时从 checkpoint 重建，之前的 Owner 追加内容完整保留。`/system clear` 触发持久化更新。

## 数据流

### 追加

Owner 动态指令的追加经 `/system add`。系统注入内容由对应功能模块直接调用追加区的系统注入写入接口（如 workflow 模块在进入模式时写入 workflow context）。两者共用追加区写入通道，互不清除。

```
/system add <内容>
  ↓
Gateway 调用 ConversationSession.add_system_append()
  ↓
ConversationSession 更新内存中的追加条目列表（Owner 动态指令子集）
  ↓
CheckpointManager.save() 将 system_appends 写入 SessionCheckpoint
  ↓
下一次 API 调用时，ConversationSession 读取自身追加条目并拼入 AppendSection
```

### 清空

Owner 动态指令经 `/system clear` 清空；系统注入内容由对应功能模块移除。

```
/system clear
  ↓
Gateway 调用 ConversationSession.clear_system_appends()
  ↓
ConversationSession 清空内存中追加条目列表中的 Owner 动态指令子集 + 触发静态层缓存全部失效（系统注入内容不受影响）
  ↓
CheckpointManager.save() 持久化空列表
```

系统注入内容的移除（如 workflow 结束）仅更新追加区的系统注入子集，**不触发**重新组装，也不改动 Owner 动态指令子集。

## 模块关系

### 上游

- **Slash 模块**：`/system` 指令触发 Owner 动态指令子集的增删操作。详细交互见 [slash/system-append](../slash/system-append.md)。
- **workflow 模块**：流程启动/结束时向追加区写入/移除系统注入上下文。详见 [workflow/session-integration](../workflow/session-integration.md)。

### 下游

- **ConversationSession**：每次 API 调用时从自身运行时字段读取追加条目（Owner 动态指令 + 系统注入内容）并拼入 System Prompt 末尾。
- **SessionCheckpoint**：`system_appends` 字段随 session 持久化，恢复时重建（仅 Owner 动态指令持久化；系统注入内容重启后由功能模块重新注入）。

### 无关

- **静态层**：追加区与静态层是独立分区，互不覆盖。`/system clear` 会同时清空静态层缓存以触发重建，但两者内容相互独立。
- **Compaction 模块**：追加区内容不参与对话压缩，压缩不影响追加条目。
