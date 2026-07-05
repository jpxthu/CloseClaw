# 会话状态

## 概述

PlanState 是 Plan Mode 下的规划状态枚举，由 mode 模块管理，Session 持久化。SessionCheckpoint、SessionStatus 和 PersistResult 是与会话生命周期管理和持久化相关的辅助类型。

> **本文档定义的 PlanState、SessionCheckpoint、SessionStatus、PersistResult 在 common crate 中实现。引用本模块的下游文档通过这些链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### PlanState

PlanState 描述当前规划的阶段和未完成步骤列表。由 mode 模块管理，Session 持久化。Compaction 对此状态做隔离保护（不压缩 plan 相关消息），Session 恢复时重建 PlanState。

| 字段 | 类型 | 说明 |
|------|------|------|
| `phase` | enum | 当前阶段：Research / Design / Review / FinalPlan / Interview |
| `pending_steps` | list(string) | 未完成的规划步骤标识列表，用于 compaction 保护和恢复后继续 |
| `plan_file_path` | string | plan 文件的路径，Agent 写入和读取的唯一可写目标 |

**阶段枚举**（phase）：

| 值 | 说明 |
|----|------|
| Research | 研究和信息收集阶段 |
| Design | 设计方案阶段 |
| Review | 设计评审阶段 |
| FinalPlan | 最终计划确定阶段 |
| Interview | 用户访谈/澄清阶段 |

### SessionCheckpoint

SessionCheckpoint 是会话检查点，用于 Session 持久化和恢复。

> **文档编写中** — SessionCheckpoint 的具体字段定义待 Session 持久化方案确定后细化。

### SessionStatus

SessionStatus 是会话状态的枚举。

> **文档编写中** — SessionStatus 的具体枚举值待 Session 生命周期管理方案确定后细化。

### PersistResult

PersistResult 是会话持久化操作的结果。

> **文档编写中** — PersistResult 的具体字段定义待 Session 持久化方案确定后细化。

## 数据流

### PlanState

PlanState 的管理路径：

```
/plan 指令 → mode 模块创建 PlanState
  ↓
Session 存储 PlanState（随 checkpoint 持久化）
  ↓
Compaction 时隔离保护 PlanState 相关消息（不压缩）
  ↓
Session 恢复时从 checkpoint 重建 PlanState
  ↓
Plan Mode 结束时销毁 PlanState
```

### SessionCheckpoint

SessionCheckpoint 的持久化路径：

```
Session 创建 / 状态变更 / 停止
  ↓
Gateway 构造 SessionCheckpoint { session_id, agent_id, channel, status, last_activity }
  ↓
StorageProvider.save_checkpoint()
  ↓
持久化存储（SQLite / 文件系统）
  ↓
Session 恢复时：
  ↓
StorageProvider.load_checkpoint(session_id)
  ↓
重建 Session（恢复 agent_id, channel, status, last_activity）
```

### PersistResult

PersistResult 的处理路径：

```
StorageProvider 方法执行（save / load / delete / flush）
  ↓
返回 PersistResult
  ├── Success → 正常继续
  ├── PartialSuccess { warnings } → 记录警告日志，继续流程
  └── Failure(error) → Gateway 记录错误日志，尝试重试或降级
```

## 模块关系

### PlanState

- **生产者**：mode 模块（Plan Mode 进入时创建）
- **消费者**：Session（持久化和 compaction 保护）；mode 模块（恢复时重建、阶段切换时更新）
- **无关**：LLM Provider（PlanState 不直接传给 LLM，通过 system prompt 的 plan 上下文间接生效）、IM Adapter（消息路由不感知 PlanState）

### SessionCheckpoint

- **生产者**：Gateway（Session 创建、状态变更、停止时构造 checkpoint）
- **消费者**：StorageProvider（持久化保存和加载 checkpoint）；Gateway / SessionManager（从存储恢复时反序列化重建 Session）
- **无关**：LLM Provider（不接触 checkpoint 结构）、IM Adapter（不参与 session 持久化）

### SessionStatus

- **生产者**：Gateway（在构造 SessionCheckpoint 时设置 SessionStatus）
- **消费者**：StorageProvider（随 checkpoint 持久化）；Gateway / SessionManager（从 checkpoint 重建时还原会话状态）
- **无关**：LLM Provider（不接触 session 状态枚举）、IM Adapter（不参与 session 生命周期管理）

### PersistResult

- **生产者**：StorageProvider 所有方法（save_checkpoint / load_checkpoint / delete_checkpoint / list_checkpoints / flush 均返回 PersistResult）
- **消费者**：Gateway / SessionManager（消费 PersistResult 判断操作成功与否，做日志记录或重试）
- **无关**：LLM Provider（不与持久化层直接交互）、IM Adapter（不参与持久化）
