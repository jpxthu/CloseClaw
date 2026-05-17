# Plan 执行引擎

## 概述

执行引擎将 Plan Mode 审批通过的 confirmed plan 逐步骤落地执行。核心职责：框架级结构化进度追踪、可配置的执行调度、压缩后进度恢复、失败处理。

## 架构

### 进度管理

进度追踪由 ProgressTool 接管，LLM 不能直接修改 plan 文件中的进度标记。

agent 调用 ProgressTool 传入步骤索引和新状态 → 框架校验后更新 PlanState → 同步写入 plan 文件。校验规则：禁止跳步（前一步骤未完成则拒绝）、禁止回退 completed → in_progress、禁止越界。

### 步骤状态

| 状态 | 含义 | 说明 |
|------|------|------|
| pending | 尚未开始 | 初始状态 |
| in_progress | 执行中 | agent 开始步骤时标记 |
| completed | 成功完成 | agent 完成步骤时标记 |
| failed | 执行失败 | 需用户介入或重试 |
| skipped | 已跳过 | 用户或 agent 显式跳过 |

状态流转单向：pending → in_progress → completed | failed。completed 后不可回退。failed → in_progress 允许（重试场景）。

### 压缩后进度恢复

四层恢复机制：

1. **PlanState 持久化保护**：PlanState 不经过 LLM summarization，compaction 时作为受保护数据保留原文
2. **System prompt 末尾注入进度摘要**：每次 API 调用时在所有静态内容之后追加当前步骤和进度，放在最末尾以最大化 KV Cache 命中率
3. **Plan 文件内容注入**：executing/paused 状态恢复时，plan 文件 Tasks 节作为上下文注入
4. **ProgressTool 调用历史**：兜底恢复源，仅当前三层均失败时使用

进度注入时机：两次压缩之间 agent 从 ProgressTool 调用历史中能看到进度，不依赖 system prompt 注入。注入的主要价值在上下文重建场景。

### 执行模式

**Inline**：主 session 直接执行。退出 plan_mode，进入 Auto Mode，逐步完成 tasks。上下文连续但越长越容易积累噪音。

**Spawn per_step**：每个 task spawn 独立子 agent。每步干净上下文，步骤间隔离。失败不影响已完成步骤，可对失败步骤单独重试。

**Spawn all_steps**：一个子 agent 执行全部 tasks。适合步骤间有强依赖、需连续上下文的场景。

默认 inline 执行，spawn 策略通过配置切换。

### 子 agent 结构化通知

子 agent 完成步骤后以结构化格式返回结果（步骤索引、状态、摘要、改动文件列表、错误信息），父 session 直接解析状态字段决定后续动作，不依赖自由文本理解。

### 步骤完成 Hook

步骤标记 completed 时自动触发后续操作：

- **verification**：非平凡任务完成后自动 spawn Verification agent
- **notify**：向用户发送进度更新
- **custom**：用户自定义脚本

触发条件可配置（非平凡任务 / 始终 / 从不）。hook 可阻止父 session 进入空闲等待。

### 失败重试

步骤失败后自动重试，次数达上限后暂停并通知用户。重试时可选 fresh spawn（默认，干净上下文）或 continue 原上下文（保留错误信息）。用户可随时决定：重试、修改 plan、跳过、放弃。

### 配置

执行模式、spawn 粒度、重试次数与策略、验证触发条件、hook 行为均可通过配置调整。默认值：inline 执行、per_step spawn、最多 3 次重试、fresh 重试、非平凡任务触发验证。

## 数据流

### Inline 执行

```
Plan 审批通过
  ↓
session 退出 plan_mode → 标记 Auto Mode
  ↓
注入进度摘要 + Auto Mode 指令 + plan 文件内容
  ↓
循环：
  取下一个 pending task
  → ProgressTool(in_progress) → 执行 → ProgressTool(completed|failed)
  ↓ 全部 completed
PlanState = completed → 可选 spawn Verification agent
```

### Spawn per_step 执行

```
父 session（执行管家角色）：
  ↓
循环 for each pending task：
  ProgressTool(in_progress)
  → spawn executor 子 agent（轻量上下文 + task 描述 + plan context）
  → 子 agent 完成 → 结构化通知返回
  → 验收结果：
    成功 → ProgressTool(completed)
    失败且未超重试 → 重新 spawn（fresh 或 continue）
    失败且超重试 → ProgressTool(failed) → 暂停 → 通知用户
  ↓ 全部 completed
PlanState = completed
```

### 压缩中断恢复

```
compaction 触发
  ↓
PlanState 受保护不送入压缩
  ↓
session 续活
  ↓
system prompt 末尾注入进度摘要（位置在静态内容之后，最大化 KV Cache 命中）
  ↓
agent 从 current_step 继续
```

### 失败处理

```
ProgressTool(failed)
  ↓
停止调度新步骤
  ↓
通知用户：失败步骤 + 原因 + 建议
  ↓
用户决策 → 重试 | 修改 plan | 跳过 | 放弃
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Plan Mode | 审批通过后触发执行引擎 |
| User | 失败/暂停时用户决策 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Agent | spawn executor 子 agent |
| Session | PlanState 持久化、compaction 保护 |
| System Prompt | 注入进度摘要 + 执行管家角色指令 |
| Tools | ProgressTool 注册和调用 |

### 模块内关系

- Plan 文件由 Plan Mode 创建，执行引擎读取和更新进度
- 失败后修改 plan 回到 Plan Mode（追加步骤，不改已完成的）

### 无关

| 模块 | 说明 |
|------|------|
| LLM Provider | 不直接调用 |
| Processor Chain / Renderer | 无关 |
| IM Adapter | 无关 |
| Permission | 执行阶段权限由 Auto Mode 配置管理 |
