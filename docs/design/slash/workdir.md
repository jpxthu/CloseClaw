# 工作目录操作

## 概述

`/cd`、`/pwd` 和 `/git` 指令操作 session 的工作目录字段。工作目录的定义（字段、默认值、与 system prompt 注入的关系）见 [session/working-directory.md](../session/working-directory.md)，本文档只描述这三个斜杠指令本身的处理逻辑。

## 架构

三个指令由同一个 WorkdirHandler 处理：

- **`/cd <路径>`**：变更工作目录。先校验路径存在性，不存在则回复错误；存在则切换并回复目录信息和当前 Git 分支。
- **`/pwd`**：输出当前工作目录路径。
- **`/git <args>`**：经 Permission 模块审批后执行 Git 命令。仅允许只读子命令（status、log、diff、branch、show），写操作需额外审批。

```
/cd <路径>
  ↓
WorkdirHandler 校验路径存在性
  ├── 不存在 → Reply("目录不存在")
  └── 存在 → Gateway 调用 session.set_workdir(路径)
              ↓
            Reply(目录路径 + git 分支信息)

/git <args>
  ↓
WorkdirHandler 返回 Reply(git 输出) 或 Exec
  ↓
只读命令 → 直接执行并回复
写命令 → 提交 Permission 模块审批 → 通过后执行
```

## 数据流

- **`/cd <路径>`**：校验路径 → 切换工作目录 → 回复目录状态（含 Git 分支）
- **`/pwd`**：读取当前工作目录 → 回复路径
- **`/git status`**：Permission 审批 → 执行 → 回复输出
- **`/git commit`**：Permission 审批拦截写操作，不执行

## 模块关系

- **上游**：Gateway → Dispatcher → WorkdirHandler
- **下游**：Session 模块（`set_workdir()` 方法）；Permission 模块（`/git` 写操作审批）
- **无关**：LLM 对话流程
