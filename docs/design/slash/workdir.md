# 工作目录操作

## 概述

- 一句话：提供 `/cd`、`/pwd`、`/git` 三个斜杠指令，操作 session 的工作目录字段。工作目录的定义（字段、默认值、与 system prompt 注入的关系）见 [session/working-directory.md](../session/working-directory.md)，本文档只描述这三个斜杠指令本身的处理逻辑。

## 架构

三个指令由同一个 WorkdirHandler 处理：

- **`/cd <路径>`**：变更工作目录。先校验路径存在性，不存在则回复错误；存在则切换并回复目录信息和当前 Git 分支。
- **`/pwd`**：输出当前工作目录路径。
- **`/git <args>`**：经 Permission 模块审批后执行 Git 命令。只读子命令（status、log、diff、branch、show）可直接执行，写操作需经权限审批。

```
/cd <路径>
  ↓
WorkdirHandler 校验路径存在性
  ├── 不存在 → 回复"目录不存在"
  └── 存在 → 切换工作目录 → 回复目录路径 + Git 分支信息

/pwd → WorkdirHandler 读取当前工作目录 → 回复目录路径

/git <args>
  ↓
WorkdirHandler 判断命令类型
  ├── 只读命令 → 直接执行并回复
  └── 写命令 → 提交 Permission 模块审批
        ├── 通过 → 执行并回复
        └── 拒绝 → 回复"权限不足"
```

## 数据流

- **`/cd <路径>`**：校验路径 → 存在则切换工作目录并回复目录状态（含 Git 分支）；不存在则回复错误
- **`/pwd`**：读取当前工作目录 → 回复路径
- **`/git status`**：只读命令直接执行 → 回复输出
- **`/git commit`**：写命令提交 Permission 审批 → 通过则执行并回复；拒绝则回复"权限不足"

## 模块关系

- 与模块内其他子功能无直接交互
- **上游**：Gateway → Dispatcher → WorkdirHandler
- **下游**：Session 模块（工作目录切换接口）
- **间接下游**（通过 Gateway 调用）：Permission 模块（`/git` 写操作审批）
- **无关**：LLM 对话流程
