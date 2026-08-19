# 工作目录操作

## 概述

- 一句话：提供 `/cd`、`/pwd`、`/git` 三个斜杠指令，操作 session 的工作目录字段。工作目录的定义（字段、默认值、与 system prompt 注入的关系）见 [session/working-directory.md](../session/working-directory.md)，本文档只描述这三个斜杠指令本身的处理逻辑。

## 架构

三个指令由同一个 WorkdirHandler 处理：

- **`/cd <路径>`**：校验路径存在性 → 不存在则回复"目录不存在"；存在则切换工作目录 → 回复目录路径 + Git 分支信息
- **`/pwd`**：读取当前工作目录 → 回复路径
- **`/git <args>`**：返回 [Git](../common/shared-types.md#slashresult) 变体。WorkdirHandler 解析后判断读写：只读子命令（status、log、diff、branch、show）不携带操作描述，直接执行；写操作携带操作描述（标记为需评估的操作），由 Gateway 提交 Permission 模块评估。执行流程：

1. WorkdirHandler 解析参数、判断读写性质，返回 Git 变体
2. Gateway 检查变体是否携带操作描述：
   - 未携带（只读子命令，如 `/git status`）→ 直接执行并回复输出
   - 携带（写操作，如 `/git commit`）→ 提交 Permission 模块评估：Allow → 执行并回复；Deny → 回复"权限不足"

## 数据流

- **`/cd <路径>`**：校验路径 → 存在则切换工作目录并回复目录状态（含 Git 分支）；不存在则回复错误
- **`/pwd`**：读取当前工作目录 → 回复路径
- **`/git status`**：WorkdirHandler 判断为只读、不携带操作描述 → Gateway 直接执行 → 回复输出
- **`/git commit`**：WorkdirHandler 判断为写操作、携带操作描述 → Gateway 提交 Permission 权限评估 → 通过则执行并回复；拒绝则回复"权限不足"

## 模块关系

- 与模块内其他子功能无直接交互
- **上游**：Gateway → Dispatcher → WorkdirHandler
- **下游**：Session 模块（工作目录切换接口）
- **间接下游**（通过 Gateway 调用）：Permission 模块（Gateway 读取 Git 变体携带的操作描述后，统一提交权限引擎评估——写操作评估，只读子命令不携带描述直接执行）
- **无关**：LLM 对话流程
