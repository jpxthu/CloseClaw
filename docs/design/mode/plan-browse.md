# Plan 浏览与管理

## 概述

- 一句话：User 可随时查看和管理工作区中已有的 plan——列出、查看、废弃，通过 `/plans` 命令或自然语言触发。

## 架构

plan 以独立文件持久化在 `workspace/plans/`（格式见 [plan-mode.md](plan-mode.md)）。浏览与管理通过两种入口触发，共享同一套 plan 文件：

- **`/plans` 命令**：系统级指令，由 Gateway 拦截、分派给 PlanBrowseHandler，不进入 LLM 对话流程。非 Immediate——需等待当前 LLM 调用结束后执行（分派见 [slash §F12](../../requirements/slash.md)）
- **自然语言**：User 直接对 Agent 表达浏览/废弃意图，Agent 用文件工具直接操作 plans 目录

### 操作类型

| 操作 | 触发方式 | 行为 |
|------|---------|------|
| 列出 | `/plans`（无参数）、自然语言 | 列出 `workspace/plans/` 下所有 plan 及各自步骤完成情况 |
| 查看 | `/plans <名称>`、自然语言 | 展示指定 plan 的完整内容 |
| 废弃 | 自然语言 | 删除指定 plan 文件 |

步骤完成情况从 plan 文件的 Tasks 节读取，完成标记格式见 [execution.md](execution.md)。

## 数据流

### `/plans` 列出

1. User 发送 `/plans`（无参数）
2. Gateway 拦截 → 分派给 PlanBrowseHandler（非 Immediate，排队执行）
3. 扫描 `workspace/plans/` 目录，逐个读取 plan 文件 Tasks 节的步骤完成标记
4. 汇总每个 plan 的标题与步骤完成情况（已完成/总数）
5. 回复 User

### `/plans <名称>` 查看

1. User 发送 `/plans <名称>`
2. Gateway 拦截 → 分派给 PlanBrowseHandler
3. 定位 `workspace/plans/{名称}.md`
4. 读取并展示 plan 完整内容
5. 回复 User；plan 不存在时提示未找到

### 自然语言浏览与废弃

- User 表达浏览意图（列出或查看）→ Agent 用只读工具扫描/读取 plans 目录
- User 表达废弃意图 → Agent 删除对应 plan 文件

废弃是删除操作：删除后 plan 不再出现在列表中，不可恢复。plan 的自动归档（全部步骤终态 + 最后访问超配置天数）是独立机制，见 [README.md](README.md) 与 [execution.md](execution.md)。

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Slash Command | `/plans` 命令入口 |
| User | 自然语言浏览/废弃意图 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| 无 | 本子功能不调用其他模块，仅读写 `workspace/plans/` 目录文件 |

### 无关

| 模块 | 说明 |
|------|------|
| Permission | 废弃删除的是 User 自己的 plan 文件，非系统危险操作，无需审批 |
| LLM Provider | 不直接调用 |
| Processor Chain / Renderer | 无关 |
| IM Adapter | 无关 |
