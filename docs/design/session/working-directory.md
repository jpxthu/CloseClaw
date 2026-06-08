# 工作目录

## 概述

工作目录是 session 级字段，定义 agent 的默认文件操作路径。它是 `/cd`、`/pwd`、`/git` 三个斜杠指令和 system prompt 动态层注入的共同数据源——所有对工作目录的读写都指向同一个 session 字段。

## 架构

### 字段定义

工作目录是 Session 上的一个会话级状态：

- **字段**：`workdir: PathBuf`
- **默认值**：`{config_dir}/workspaces/{agent_id}/{user_id}/`
- **生命周期**：随 session 创建而初始化，随 session 销毁而释放。不进持久化存储——session 恢复后重新初始化为默认值。

### 变更方式

工作目录只能通过 `/cd <路径>` 指令变更。`/cd` 校验路径存在性后，调用 `session.set_workdir(路径)` 更新值。其他模块无权直接写入。

### 读取方式

两个消费者读取同一个字段：

- **`/pwd` 指令**：读取 `session.workdir`，回复路径字符串。
- **System prompt 动态层**：每次 API 请求时从 `session.workdir` 读取，注入工作目录路径信息。

### GitStatus 的依赖

GitStatus 是工作目录的派生信息——在 `session.workdir` 路径上执行 `git branch --show-current` 获取当前分支。GitStatus 值由工作目录决定，但工作目录本身不包含 git 信息。两者是独立的动态层注入条目。

## 数据流

```
Session 创建
  → workdir = {config_dir}/workspaces/{agent_id}/{user_id}/
  → system prompt 动态层首次注入默认路径

用户发 /cd /tmp
  → WorkdirHandler 校验 /tmp 存在
  → session.set_workdir("/tmp")
  → 下次 API 请求时 system prompt 动态层注入新路径

用户发 /pwd
  → WorkdirHandler 读取 session.workdir
  → 回复当前路径

每次 API 请求
  → ConversationSession 从自身运行时字段读取 workdir
  → 动态层注入 "当前工作目录：{path}"
```

## 模块关系

- **上游**：SessionManager（在 session 创建时初始化默认值，在 session 恢复时重新初始化）
- **下游**：
  - Slash WorkdirHandler（`/cd` 写入，`/pwd` 读取）
  - System Prompt 动态层（每次 API 请求时读取并注入）
- **相关**：GitStatus（从工作目录派生 git 分支信息，但为独立的动态层注入条目）
- **无关**：Permission 模块（`/git` 写操作的权限审批由 WorkdirHandler 提交给 Permission 模块，但工作目录字段本身不经过权限检查）
