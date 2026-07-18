# Plan 执行引擎

## 概述

执行引擎将 User 满意的 plan 逐步骤落地执行。核心职责：执行触发、进度追踪、中断恢复、失败处理。

## 架构

### 执行触发

plan 写完后，User 通过以下方式触发执行：

**斜杠指令**：通过 `/execute` 命令触发执行。若当前处于 Plan Mode 则先退出。

**自然语言**：Agent 调用执行触发工具，参数与 `/execute` 斜杠指令相同。该工具自动发起 User 确认交互——支持交互的通道弹出确认卡片，不支持的通道以自然语言让 User 回复确认。确认后进入 Auto Mode 开始执行。

**执行路径**：

- **同 session 执行**：当前 session 进入 Auto Mode（若处于 Plan Mode 则先退出），继承规划上下文
- **新 session 执行**：创建新 session，注入 plan 文件内容作为初始上下文，新 session 直接进入 Auto Mode

同一 plan 的并发执行不做系统级锁定，由 User 自行管理。

### Auto Mode 行为原则

Agent 在 Auto Mode 下以连续自主方式执行 plan 步骤。行为原则：

- 低风险操作直接执行，不等 User 逐步确认
- 常规决策自主做出，不升级给 User
- 不在执行中途主动重新进入 Plan Mode（User 在失败后显式选择回退修改 plan 的情形除外）
- 接受 User 随时发来的修正建议
- 危险操作（删数据、改生产配置、向外部平台发消息）必须经 User 确认
- 不擅自向外部平台发送消息

### 进度管理

执行进度由 Agent 自行管理——Agent 在 plan 文件中以约定的格式标记步骤完成状态。系统不介入进度判断，Agent 是步骤完成与否的唯一判断者。

- Agent 按 plan 文件的 Tasks 节顺序执行步骤
- 每步完成后 Agent 在 plan 文件中更新对应步骤标记
- 步骤状态由 Agent 自行判断：未开始 → 进行中 → 已完成 / 失败 / 已跳过

### 步骤状态

| 状态 | 含义 | 标记格式 |
|------|------|---------|
| 未开始 | 步骤尚未执行 | `[ ]` |
| 进行中 | 步骤正在执行 | `[-]` |
| 已完成 | 步骤成功完成 | `[x]` |
| 失败 | 步骤执行失败，需 User 介入 | `[!]` |
| 已跳过 | User 或 Agent 显式跳过 | `[~]` |

状态流转单向：未开始 → 进行中 → 已完成 / 失败 / 已跳过。失败后 User 可决定重试（失败 → 进行中）。已完成不允许回退。已跳过的步骤可由 User 显式选择恢复执行（已跳过 → 进行中）。

### 执行方式

执行方式完全由 User 通过自然语言指令决定，没有固定的模式约束：

- User 可指定在同 session 或新 session 中执行
- User 可指定执行全部步骤或部分步骤
- User 可要求 Agent spawn 子 Agent 来执行特定步骤

### 中断恢复

执行过程中的中断可无缝恢复：

- User 可随时暂停执行，当前进度被保存
- User 恢复执行时，Agent 从暂停时的当前步骤继续，不重复已完成步骤
- session 压缩或重启后 Agent 仍然知道当前执行进度（当前第几步、哪些已完成、哪些待继续），不需要从第 1 步重新执行
- plan 文件本身具备独立于 session 的恢复保障——即使 session 完全丢失，仍可基于 plan 文件内容重建执行上下文

### 失败处理

步骤是否失败由 Agent 自行判断，不存在系统级的重试次数限制或自动重试机制。Agent 认为某步骤失败后，User 可自由决定下一步操作——重试、显式选择回到 Plan Mode 修改后续步骤、跳过该步骤、或放弃。

### 拒绝日志

Auto Mode 下被拦截的危险操作会生成拒绝日志，User 可查看。

- 危险操作范围：删除数据、修改生产配置、向外部平台发送消息，以及 Permission 模块标记为需审批的操作
- 每条被拒绝的操作记录：工具名、操作描述、拒绝原因、时间戳
- 按时间倒序排列
- 持久化（本地文件），跨 session 可见
- User 可配置日志存储上限

### 配置

拒绝日志存储上限和 plan 归档天数由 User 配置，详见 [config](../config/README.md) 模块。

## 数据流

### 同 session 执行（斜杠指令）

1. User `/execute`
2. 若处于 Plan Mode → 退出 Plan Mode
3. session 标记 Auto Mode
4. 注入 Auto Mode 指令 + plan 文件内容
5. Agent 按 plan Tasks 节顺序逐步执行
6. 每步完成后 Agent 更新 plan 文件步骤标记
7. 全部步骤完成 → session 退出 Auto Mode → 恢复默认模式

### 同 session 执行（自然语言触发）

1. User 自然语言要求执行
2. Agent 调用执行触发工具 → User 确认
3. 若处于 Plan Mode → 退出 Plan Mode
4. session 标记 Auto Mode
5. 注入 Auto Mode 指令 + plan 文件内容
6. Agent 按 plan Tasks 节顺序逐步执行
7. 每步完成后 Agent 更新 plan 文件步骤标记
8. 全部步骤完成 → session 退出 Auto Mode → 恢复默认模式

### 新 session 执行

1. User 指定新 session 执行（通过 /execute 或自然语言）
2. 创建新 session，直接进入 Auto Mode
3. 注入 plan 文件内容作为初始上下文
4. Agent 按 plan Tasks 节顺序逐步执行
5. 每步完成后 Agent 更新 plan 文件步骤标记
6. 全部步骤完成 → session 退出 Auto Mode → 恢复默认模式

### Spawn 子 Agent 执行

1. User 指定 spawn 执行特定步骤
2. 父 session 处于 Auto Mode
3. spawn executor 子 Agent（传入步骤描述 + plan 上下文）
4. 子 Agent 执行 → 结果通知父 session
5. 父 session 更新 plan 文件对应步骤标记

### 中断恢复

1. User 暂停 → Agent 停止当前步骤，记录进度到 plan 文件
2. session 标记保持 Auto Mode
3. User 恢复 → Agent 从当前步骤继续执行

### session 压缩后恢复

1. compaction 触发 → plan 文件不受压缩影响，独立于 session 持久化
2. session 续活时重新读取 plan 文件 Tasks 节
3. Agent 识别最后完成的步骤，从下一步继续

### 失败处理

1. Agent 判定步骤失败 → 标记 plan 文件对应步骤为失败
2. 停止后续步骤 → 通知 User：失败步骤 + 原因
3. User 决策：重试 / 回到 Plan Mode 修改后续步骤 / 跳过 / 放弃

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Plan Mode | User 满意 plan 后触发执行（触发途径之一，非唯一入口） |
| Slash Command | `/execute` 命令入口 |
| User | 自然语言触发执行、暂停/恢复、失败决策 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Agent | spawn executor 子 Agent |
| Session | auto_mode 标记持久化、compaction 保护 |
| System Prompt | 注入 Auto Mode 指令 |
| Permission | Auto Mode 下运行时审查危险操作 |
| Tools | 执行触发工具注册和调用 |

### 模块内关系

- Plan 文件由 Plan Mode 阶段创建，执行引擎读取和更新步骤状态
- 失败后 User 可显式选择回到 Plan Mode 修改后续步骤（追加或修改未完成步骤，不改已完成的）

### 无关

| 模块 | 说明 |
|------|------|
| LLM Provider | 不直接调用 |
| Processor Chain / Renderer | 无关 |
| IM Adapter | 无关 |
