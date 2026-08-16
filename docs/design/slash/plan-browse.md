# plan 浏览

## 概述

- 一句话：`/plans` 指令用于查看工作区中已有的 plan，由 Gateway 拦截并分派，不进入 LLM 对话流程。

## 架构

`/plans` 是非 Immediate 指令——需等待当前 LLM 调用结束后执行。完整语法、参数和业务行为由 mode 模块定义（见 [mode §F6](../../requirements/mode.md)），本模块仅提供 Gateway 层的指令拦截和分派。

- **PlanBrowseHandler**：处理 `/plans`，消费 SlashDispatcher 解析出的可选 plan 名称参数，读取 plans 目录，返回 [SlashResult](../common/shared-types.md#slashresult)

## 数据流

1. User 发送 `/plans` 或 `/plans <名称>`
2. Gateway 拦截 → SlashDispatcher 解析指令名与参数
3. 分派给 PlanBrowseHandler
4. 无参数 → 列出所有 plan 及步骤完成情况；带名称 → 展示指定 plan 完整内容
5. 回复内容 → 出站 Processor Chain → IM 插件渲染发送

## 模块关系

- **上游**：Gateway（入站消息处理，`/` 前缀拦截分派）
- **下游**：mode 模块（plan 文件存储位置与格式由 mode 定义，`/plans` 按此读取）
- **无关**：LLM 对话流程（`/plans` 不触发 LLM 调用）
