# Plan Mode

## 概述

Plan Mode 将任务规划与代码执行强制分离。规划阶段 agent 只读（仅 plan 文件可写），经过审批栅栏后方可进入 Auto Mode 执行。可靠性由代码层保证（工具过滤、审批拦截），不依赖 prompt 建议。

支持两条路径：标准路径（需求明确）和 Interview 路径（需求模糊），由系统自动判断入口。

## 架构

### 双路径

```
需求明确 → 标准 4 阶段 → 审批 → Auto Mode 执行
需求模糊 → Interview 循环 → 对接标准后段 → 审批 → Auto Mode 执行
```

**标准路径**：

| Phase | 目标 | 机制 |
|-------|------|------|
| Research | 理解需求 + 探索代码库 | 并行 spawn Explore agent（只读） |
| Design | 生成实现方案 | spawn Plan agent，架构师视角（只读） |
| Review | 对齐需求 + 澄清问题 | AskUserQuestion（仅限需求澄清） |
| Final Plan | 写入 plan 文件 | 唯一可写操作 |

**Interview 路径**：无固定阶段。agent 循环"探索代码→增量更新 plan 文件→向用户提问"直到需求收敛，然后对接标准路径的 Review 和 Final Plan 阶段。每轮探索后增量写入 plan 文件。

需求清晰度判断：系统在进入 Plan Mode 时分析用户输入——含明确文件/模块/接口引用且有可量化验收条件 → 标准路径；否则 → Interview 路径。用户可通过命令参数显式指定路径，此时系统直接采用指定路径，不再做自动判断。

阶段切换由 agent 自行判断，无代码层阶段状态机。

### Agent 类型

Plan Mode 各阶段通过 spawn 子 agent + 特定 system prompt 实现不同角色。每种 agent 类型对应 agent 模块的一套固定 prompt 模板和工具白名单：

| 类型 | 阶段 | 能力 | 职责 |
|------|------|------|------|
| Explore agent | Research | 只读工具 | 并行探索代码库，理解现有实现和依赖 |
| Plan agent | Design | 只读工具（禁止写和审批工具） | 架构师视角生成实现方案，输出关键文件列表 |
| Verification agent | 执行后验证 | 只读工具，仅可写临时测试脚本 | 独立验证实现，尝试打破而非确认能用 |
| Executor agent | Auto Mode | 完整工具集，危险操作受审查 | 按 plan tasks 逐步实施 |

Plan Mode 不引入 Teammate（持久进程）或 Fork（上下文继承）机制。

### 能力约束

Plan Mode 下的工具限制由 Permission 层硬拦截——写工具不在白名单中则不可见，agent 无法调用：

| 约束 | 机制 |
|------|------|
| 只读工具 | 与 plan mode 白名单取交集 |
| plan 文件写 | plans/ 目录作为独立可写区域 |
| 子 agent 继承 | spawn 出的子 agent 继承只读约束 |

### Plan 文件

**路径**：`workspace/plans/{identifier}.md`，identifier 支持时间戳格式（如 `2026-05-17-14-30-05-user-auth-flow`）或随机词组格式（如 `calm-wave-oven`），由配置决定。

**文件内容**：

- 任务标题、创建/更新时间
- 状态标记：draft → confirmed → executing → paused → completed
- Context 节：背景、约束、已确认决策
- Tasks 节：有序步骤列表，每步带 checkbox
- Verification 节：端到端验证方式
- Notes 节：执行备注

### 审批栅栏

审批工具是 Plan Mode 的唯一出口。代码层保证：

- Permission 层在 Plan Mode 下拦截所有写工具，仅 plans/ 目录放行
- 审批工具调用时框架弹出用户确认对话框
- 审批通过 → plan 标记 confirmed → session 退出 plan_mode → 进入 Auto Mode
- 审批拒绝 → plan 保持 draft → agent 继续修改
- 禁止用 AskUserQuestion 替代审批——AskUserQuestion 仅用于需求澄清

### 多路径恢复

Plan 内容在以下场景丢失时按优先级恢复（任一可用即可）：

1. **PlanState 持久化**：session 模块维护的 PlanState 字段，压缩时完整保护
2. **Plan 文件磁盘**：独立于 session 的持久化副本
3. **审批工具调用记录**：审批时携带的 plan 摘要
4. **消息历史**：用户消息中的 plan 引用

### 状态持久化

PlanState（plan 文件路径、当前步骤、状态等）由 session 模块持久化，压缩时完整保护，不经过 LLM 总结。步骤推进时框架自动更新。

### Auto Mode（执行阶段）

审批通过后自动进入 Auto Mode。Auto Mode 的行为约束和两种执行方式（Inline/Spawn）详见 execution.md。

简要约束：连续自主执行，低风险工作直接做，常规决策不升级，危险操作（删数据、改生产配置）必须用户确认。

### 安全机制

三层防护：

| 层级 | 机制 | 说明 |
|------|------|------|
| 工具过滤 | Permission 层硬拦截 | Plan Mode 下写工具不可见 |
| 审批栅栏 | 审批工具弹出确认 | 无审批不退出 |
| 执行审查 | Auto Mode 下危险操作拦截 | 运行时审查每条命令，被拒绝的操作记录拒绝日志（含工具名、操作描述、拒绝原因、时间戳） |

### 状态机

```
draft ──审批通过──→ confirmed ──进入 Auto Mode──→ executing ──全部完成──→ completed
  ↑                     |                               ↓
  └───被拒绝────────────┘                           /pause
                                                       ↓
                                                    paused
```

被拒绝后回到 draft，可重新修改提交。confirmed 后中断走 paused，续活时从当前步骤恢复。

### Plan 归档

completed plan 最后访问超过配置天数后自动归档到 `workspace/plans/archive/`，由 session 模块的后台任务处理。

## 数据流

### 进入 Plan Mode

```
用户 /plan "任务描述"
  →
session 设置 plan_mode 标记
  →
系统 prompt 组装：
  - 分析用户输入清晰度
  - 清晰 → 注入标准路径 4 阶段指令
  - 模糊 → 注入 Interview 循环指令
  →
工具过滤取交集白名单 + 权限边界设为 plans/ 目录可写
  →
agent 进入对应路径
```

### Research 阶段

```
spawn Explore agent（指定只读 agent 配置 + 探索任务）
  →
子 session：轻量上下文，只读工具集，探索行为约束
  →
Explore agent 完成探索 → 结果通知父 session
```

### Design 阶段

```
spawn Plan agent（指定设计 agent 配置 + 探索结果作输入）
  →
子 session：轻量上下文，只读工具集，架构师设计约束
  →
Plan agent 输出方案 → 结果通知父 session
```

### 审批 → Auto Mode（Inline）

```
审批工具调用 → 用户确认通过
  →
plan 状态 = confirmed（写入磁盘）
  →
session 退出 plan_mode → 标记 Auto Mode
  →
注入 Auto Mode 指令 + plan 文件上下文
  →
恢复完整工具集，危险操作受审查
  →
逐步执行 tasks，每步完成自动更新 checkbox
  →
全部完成 → plan 状态 = completed
```

### 审批 → Auto Mode（Spawn）

```
审批通过 → spawn executor 子 agent（传入 plan 文件 + 执行指令）
  →
子 session：Auto Mode 标记，完整工具集，危险操作受审查
  →
逐步执行 plan tasks → 完成通知父 session
  →
plan 状态 = completed
```

### Interview 路径

```
需求模糊 → 进入 Interview 循环
  →
每轮：spawn Explore agent 探索 → 增量更新 plan 文件 → 向用户提问
  → 用户回复 → 评估 ambiguities
  → 仍有模糊点 → 继续循环
  → ambiguities 消除 → Review + Final Plan → 审批出口（与标准路径相同）
```

### 验证阶段

```
执行完成后 → 主 session 判断是否需要验证（非平凡任务触发）
  →
spawn Verification agent（传入任务描述 + 改动文件 + 方案）
  →
子 session：只读工具集，可写 /tmp 测试脚本，独立审视约束
  →
每个检查：检查项 + 命令 + 输出 + PASS/FAIL
  →
最终裁决：PASS / FAIL / PARTIAL → 通知父 session
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Slash Command | `/plan` `/execute` 入口 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Agent | 各阶段 spawn Explore/Plan/Verification/Executor 子 agent |
| Session | plan_mode 标记、PlanState 持久化、压缩保护、多路径恢复、归档 |
| System Prompt | 双路径指令、Auto Mode 指令、plan 文件上下文注入 |
| Permission | 工具过滤、Auto Mode 下运行时审查 |
| Tools | 审批工具注册与调用 |

### 模块内关系

- 依赖模式系统的模式切换机制进入和退出 Plan Mode
- Agent 类型由 agent 模块的配置系统定义，Plan Mode 消费已有配置
- Plan 文件读写由 tool 模块代理

### 无关

| 模块 | 说明 |
|------|------|
| LLM Provider | 不直接调用 |
| Processor Chain / Renderer | 无关 |
| IM Adapter | 无关 |
