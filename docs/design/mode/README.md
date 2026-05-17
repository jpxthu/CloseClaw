# 模式系统

## 概述

模式系统管理 session 的运行模式，通过切换模式改变 agent 的工具可用性、系统提示词和权限边界。

## 架构

每个 session 持有一个模式标记，决定 agent 的行为约束。模式切换由用户显式触发（斜杠命令或工具调用），仅 Auto Mode 在所有任务完成后自动退出并恢复默认模式。

### 模式类型

| 模式 | 说明 |
|------|------|
| 默认模式 | 无模式标记时的行为状态——agent 按完整配置运行，全工具集可用 |
| Plan Mode | 规划与执行分离——agent 在只读约束下完成研究→设计→审批。支持标准路径和 Interview 路径。审批通过后进入 Auto Mode |
| Auto Mode | Plan Mode 审批通过后自动进入的执行模式——agent 不等用户逐步确认，按 plan 自动执行，危险操作需用户审批。本身不是独立入口（无法直接 /auto 进入），详见 execution.md |

### 模式生命周期

```
session 创建（默认模式）
  ↓ 用户 /plan
Plan Mode：
  - 标准路径：Research → Design → Review → Final Plan
  - Interview 路径：探索 → 增量更新 plan → 提问（循环直到需求收敛）
  ↓ 审批通过
Auto Mode：连续自主执行 plan tasks
  ↓ 全部完成
默认模式
```

审批栅栏是模式切换的唯一出口——Plan Mode 只有审批通过才能退出并进入 Auto Mode。

### 模式生效机制

每种模式通过三个层面影响 agent 行为：

- **工具过滤**：按模式规则限制可用工具集。Plan Mode 下工具集与只读白名单取交集，写工具不可见。Auto Mode 下完整工具集可见，但危险操作需运行时审查
- **系统提示词注入**：根据模式注入特定的行为指令。Plan Mode 注入双路径工作流指引，Auto Mode 注入连续执行指令
- **权限边界**：模式的写入范围受限。Plan Mode 下仅 plans/ 目录可写，Auto Mode 下危险操作需用户确认

三种约束在 session 创建时计算，运行期间不变。

子功能：[plan-mode.md](plan-mode.md) — Plan Mode 专项：双路径工作流、Agent 类型、审批栅栏、安全机制、多路径恢复<br>子功能：[execution.md](execution.md) — 执行引擎：ProgressTool 进度管理、Inline/Spawn 执行模式、压缩恢复、失败处理

## 数据流

### 进入 Plan Mode

```
用户 /plan "任务描述"
  →
session 设置模式标记
  →
系统判断需求清晰度 → 选择标准路径或 Interview 路径
  →
工具过滤：完整工具集取交集模式白名单
  →
系统提示词注入对应路径指令
  →
权限边界设为 plans/ 目录可写
  →
agent 进入对应路径
```

需求清晰度判断：分析用户输入中是否有明确文件/模块/接口引用、是否有可量化验收条件。清晰 → 标准路径，模糊 → Interview 路径。

### Plan Mode → Auto Mode

```
agent 调用审批工具
  →
框架弹出用户确认
  ↓ 通过
session 清除 Plan Mode 标记
  →
标记 Auto Mode
  →
恢复执行所需工具集，注入 Auto Mode 指令
  →
agent 开始执行 plan tasks
```

### 退出 Auto Mode

```
全部 tasks 完成
  →
session 清除 Auto Mode 标记
  →
恢复默认模式
  →
plan 标记为 completed
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Slash Command | `/plan`、`/execute` 命令入口 |
| CLI | 命令行传入模式参数 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Session | 存储和持久化模式标记，管理 PlanState，压缩保护，多路径恢复 |
| System Prompt | 接收模式指令，拼入最终 system prompt |
| Agent | session 创建时读取模式标记，按模式生成对应类型子 agent |
| Permission | 工具过滤；Auto Mode 下运行时审查危险操作 |

### 无关

| 模块 | 说明 |
|------|------|
| LLM Provider | 不直接调用 |
| Processor Chain / Renderer | 无关 |
| IM Adapter | 无关 |
