# Plan Mode

## 概述

Plan Mode 将任务规划与代码执行强制分离——规划阶段 Agent 只读（仅 plan 文件可写），User 说"执行"时才进入 Auto Mode 开始实施。Plan Mode 没有审批栅栏，User 可反复审阅和修改 plan，直到满意为止。

支持两条路径：标准路径（需求明确）和 Interview 路径（需求模糊），由系统自动判断入口。

## 架构

### 双路径

```
需求明确 → 标准 4 阶段 → User 满意后触发执行
需求模糊 → Interview 循环 → 对接标准后段 → User 满意后触发执行
```

**标准路径**：

| 阶段 | 目标 | 机制 |
|------|------|------|
| Research | 理解需求 + 探索代码库 | 并行 spawn Explore Agent（只读） |
| Design | 生成实现方案 | spawn Plan Agent，架构师视角（只读） |
| Review | 展示方案 + 澄清模糊点 | User 审阅方案、提出修改意见。不是一次性审批——User 可反复审阅，Agent 持续调整 plan |
| Final Plan | 写入 plan 文件 | Agent 将最终方案写入 plan 文件，唯一可写操作 |

**Interview 路径**：无固定阶段。Agent 循环"spawn Explore Agent 探索 → 父 Agent 根据探索结果增量更新 plan 文件 → 向 User 提问澄清"直到需求收敛，然后对接标准路径的 Review 和 Final Plan 阶段。每轮探索后由父 Agent（非子 Agent）增量写入 plan 文件。

**需求清晰度判断**：系统在进入 Plan Mode 时分析 User 输入——含明确文件/模块/接口引用且有可量化验收条件 → 标准路径；否则 → Interview 路径。

**阶段切换**：由 Agent 自行判断，无代码层阶段状态机。Research 和 Design 阶段 Agent 可 spawn 子 Agent 并行工作。

### Agent 类型

Plan Mode 各阶段通过 spawn 子 Agent + 特定 system prompt 实现不同角色。每种 Agent 类型对应 Agent 模块的一套固定 prompt 模板和工具白名单：

| 类型 | 阶段 | 能力 | 职责 |
|------|------|------|------|
| Explore Agent | Research | 只读工具（并发受 [agent §F9](../../requirements/agent.md) 控制） | 并行探索代码库，理解现有实现和依赖 |
| Plan Agent | Design | 只读工具 | 架构师视角生成实现方案，输出关键文件列表 |
| Executor Agent | Auto Mode | 完整工具集，危险操作受审查 | 按 plan 步骤逐步实施 |

Plan Mode 不引入持久进程或上下文继承机制。spawn 出的子 Agent 在 Plan Mode 下也只读（详见 [permission §F9](../../requirements/permission.md)），spawn 的并发上限和创建控制详见 [agent §F9](../../requirements/agent.md)。

### 能力约束

Plan Mode 下的工具限制由模式系统自身执行——写工具中仅 plan 文件写工具可见，其余写工具不可见：

| 约束 | 机制 |
|------|------|
| 只读工具 | 与 plan mode 白名单取交集 |
| plan 文件写 | plans/ 目录作为独立可写区域，plan 文件写工具是写工具集中唯一可见的 |
| 子 Agent 继承 | spawn 出的子 Agent 继承只读约束（不含 plan 文件写权限） |

### 模式标记持久化

模式标记由 session 模块持久化，压缩时完整保护，不经过 LLM 总结。

### Plan 文件

**路径**：`workspace/plans/{identifier}.md`，identifier 格式由配置决定。

**文件内容**：

- 任务标题、创建和更新时间
- Context 节：背景、约束、已确认决策
- Tasks 节：有序步骤列表，每步带完成标记（`[ ]` 未开始 / `[-]` 进行中 / `[x]` 已完成）
- Verification 节：端到端验证方式
- Notes 节：执行备注

plan 本身无全局状态——只有步骤级别状态。已完成若干步后 User 发现设计有问题，可回 Plan Mode 修改未完成的步骤，不影响已完成步骤。

### 安全机制

两层防护：

| 层级 | 机制 | 说明 |
|------|------|------|
| 工具过滤 | 模式系统自身执行 | Plan Mode 下仅 plan 文件写工具可见 |
| 执行确认 | 执行触发工具弹出确认 | User 确认后才退出 Plan Mode 进入执行 |

### 多路径恢复

Plan 内容在以下场景丢失时按优先级恢复（任一可用即可）：

1. **Plan 文件磁盘**：独立于 session 的持久化副本
2. **消息历史**：User 消息中的 plan 引用
3. **执行触发时的上下文注入**：触发执行时重新读取 plan 文件

## 数据流

### 进入 Plan Mode

1. User `/plan "任务描述"`
2. session 设置 plan_mode 标记
3. 系统 prompt 组装：分析 User 输入清晰度 → 清晰则注入标准路径 4 阶段指令，模糊则注入 Interview 循环指令
4. 工具过滤取交集白名单：仅 plan 文件写工具可见
5. Agent 进入对应路径

### Research 阶段

1. spawn Explore Agent（指定只读 Agent 配置 + 探索任务），子 session 轻量上下文 + 只读工具集
2. Explore Agent 完成探索 → 结果通知父 session

### Design 阶段

1. spawn Plan Agent（指定设计 Agent 配置 + 探索结果作输入），子 session 轻量上下文 + 只读工具集
2. Plan Agent 输出方案 → 结果通知父 session

### Review 阶段

1. Agent 展示方案
2. User 审阅 → 提出修改意见
3. Agent 调整 plan
4. 循环直到 User 满意并决定执行

### Final Plan 阶段

1. Agent 将最终方案写入 plan 文件（`workspace/plans/{identifier}.md`）
2. 文件包含 Context、Tasks、Verification、Notes 四节
3. 写入完成后，Agent 通知 User plan 已就绪，等待 User 决定执行

### Interview 路径

进入循环：
1. spawn Explore Agent 探索 → 父 Agent 根据结果增量更新 plan 文件 → 向 User 提问
2. User 回复 → 评估模糊点
3. 仍有模糊点 → 回到步骤 1
4. 模糊点消除 → 对接标准路径的 Review + Final Plan 阶段

### 触发执行

1. User 满意 → /execute 或自然语言触发
2. Agent 调用执行触发工具 → User 确认
3. session 退出 plan_mode → 标记 auto_mode
4. 详见 [execution.md](execution.md)

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Slash Command | `/plan` 入口 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Agent | 各阶段 spawn Explore/Plan 子 Agent |
| Session | plan_mode 标记持久化、压缩保护 |
| System Prompt | 双路径指令注入 |
| Permission | Auto Mode 下运行时审查危险操作 |
| Tools | 执行触发工具注册与调用 |

### 模块内关系

- 通过模式系统的模式切换机制进入和退出 Plan Mode
- Agent 类型由 Agent 模块的配置系统定义，Plan Mode 消费已有配置
- Plan 文件读写由 Agent 直接执行（Plan Mode 下 plans/ 目录可写）

### 无关

| 模块 | 说明 |
|------|------|
| LLM Provider | 不直接调用 |
| Processor Chain / Renderer | 无关 |
| IM Adapter | 无关 |
