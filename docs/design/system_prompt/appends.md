# 追加区

## 概述

追加区是 System Prompt 末尾的独立分区，持久化在会话状态中，不受上下文压缩影响，通过 `/system` 指令增删管理。

## 架构

AppendSection 位于 system prompt 末尾，与静态层 Section 无优先级冲突——二者是独立分区。

由 `/system` 指令管理：
- `/system add <内容>`：追加文本（多次叠加，会话恢复时保留）
- `/system clear`：清空追加内容
- `/system list`：查看当前追加列表

详细设计见 [slash/system-append](docs/design/slash/system-append.md)。

追加区持久化在 SessionCheckpoint 中，不受上下文压缩影响。会话恢复时从 checkpoint 重建，之前的追加内容完整保留。`/system clear` 触发持久化更新。

## 数据流

### 追加

```
/system add <内容>
  ↓
Gateway 调用 ConversationSession.add_system_append()
  ↓
ConversationSession 更新内存中的追加条目列表
  ↓
CheckpointManager.save() 将 system_appends 写入 SessionCheckpoint
  ↓
下一次 API 调用时，ConversationSession 读取自身追加条目并拼入 AppendSection
```

### 清空

```
/system clear
  ↓
Gateway 调用 ConversationSession.clear_system_appends()
  ↓
ConversationSession 清空内存中的追加条目列表 + 触发静态层缓存全部失效
  ↓
CheckpointManager.save() 持久化空列表
```

## 模块关系

### 上游

- **Slash 模块**：`/system` 指令触发追加区的增删操作。详细交互见 [slash/system-append](docs/design/slash/system-append.md)。

### 下游

- **ConversationSession**：每次 API 调用时从自身运行时字段读取追加条目并拼入 System Prompt 末尾。
- **SessionCheckpoint**：`system_appends` 字段随 session 持久化，恢复时重建。

### 无关

- **静态层**：追加区与静态层是独立分区，互不覆盖。`/system clear` 会同时清空静态层缓存以触发重建，但两者内容相互独立。
- **Compaction 模块**：追加区内容不参与对话压缩，压缩不影响追加条目。
