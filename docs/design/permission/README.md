# 权限系统

## 概述

- 关联需求文档：[requirements/permission.md](../../requirements/permission.md)
- 一句话：判断某个 Agent 以某个 User 身份能否执行某个操作；允许则按执行位置落地，拒绝则按场景静默拒绝、通知 Owner 或进入审批。

## 架构

### 核心组件

```
                 ┌───────────────────────────┐
操作请求 ───────►│ 权限引擎                   │
(caller + 操作)  │ 规则集：Agent 维度 / 用户维度│
                 │ 沙盒模式 / 命令黑白名单     │
                 └─────┬───────────┬─────────┘
                       │ 允许       │ 拒绝 / 需审批
                       ▼           ▼
              执行策略(ExecutionPolicy)  审批队列（全局单号）
                       │           │            │
                       ▼           ▼            ▼
          沙盒/宿主执行        静默拒绝   推送（ApprovalNotifier）
        （SandboxExecutor）                     │
                                    IM 卡片 / CLI 终端提示
                                                │
                                  Owner 审批指令 ─┘
                                                │
                                  PermissionGate 调用返回结论
```

- **权限引擎**：加载并缓存规则集，按请求求值；不缓存评估结论。
- **沙盒引擎**：以操作系统原生隔离强制文件访问范围，经 [common SandboxExecutor](../common/core-traits.md#sandboxexecutor) 注入 tools 的文件类工具与命令执行层；执行边界随判定结论的执行策略传递（见 [沙盒](sandbox.md)）。
- **审批队列**：permission 实现的内存队列（daemon 作为 ApprovalFlow 组件持有并装配），管理需审批请求的去重、超时与决策；推送经 [common ApprovalNotifier](../common/core-traits.md#approvalnotifier) 交给 Gateway（见 [审批工作流](approval-workflow.md)）。
- **规则配置层**：解析、校验、持久化规则文件，触发热加载（见 [权限配置管理](permission-configuration.md)）。
- **调试日志**：评估结果、审批生命周期、规则与沙盒模式变更写入调试日志。

### 子功能索引

- [权限维度](permission-dimensions.md) — 七类操作的权限定义、判定语义与执行位置
- [沙盒](sandbox.md) — 沙盒模式、边界求值与操作系统级强制
- [审批工作流](approval-workflow.md) — 拒绝与请求的审批全链路
- [权限配置管理](permission-configuration.md) — 规则构成、修改入口、热加载与配置保护

### 身份体系

两种身份共同决定权限：

- **Agent 身份**：每个 Agent 有独立权限配置；其操作始终在某个 User 上下文中执行。
- **User 身份**：通过 IM 渠道 + IM User ID 绑定；一个 User 可绑定多个渠道，映射为同一 CloseClaw User。新建 User 默认无任何权限（含收发消息），需 Owner 授予后才能交互；其专属 workspace 的自动授权在无收发消息权限时不构成可交互能力。不区分用户组，权限以个人为单位。
- Owner 的 User ID 固定为 `owner`；CLI 渠道的 caller 恒为 Owner 且该渠道不向 User 开放。Owner 豁免按 `caller.user_id` 判定，与渠道无关。

**User 上下文来源**：

| 场景 | 本轮 User 上下文 |
|------|------------------|
| IM 消息 | 该消息发送者映射出的 User |
| CLI 对话 | Owner |
| 子 Session | 继承父 Session 本轮 User 上下文 |

#### 默认基线

新建 Agent 的默认权限（内置基线，Daemon 初始化阶段装配，不依赖配置文件）：

| 类别 | 内容 |
|------|------|
| 默认具备 | 收发消息；专属 workspace 的读写（自动放行，不依赖规则）；在沙盒内执行不在白名单也不在黑名单的命令；按自身配置档案的工具范围调用工具 |
| 默认拒绝 | workspace 之外的读写；在宿主上执行的命令（白名单为空）；黑名单内的命令（黑名单默认为空）；网络访问；配置写入 |

默认拒绝的操作仍可进入审批（黑名单命令、子 Session、沙盒引擎不可用、严格模式白名单外命令等场景除外，见 [审批工作流](approval-workflow.md#审批分流)）。**用户维度没有基线权限**：无用户维度允许规则即拒绝（新建 User 默认无任何权限）。

初始 Agent `master` 是显式例外：其权限配置由权限模块在初始 Agent 创建时生成并写入该 Agent 的 `permissions.json`（若文件缺失，权限模块在首次加载该 Agent 时按初始 Agent 基线补写一次），内容为七个维度规则全开放、沙盒模式放行模式（高风险基线）；黑名单命令与高风险操作名单对 `master` 同样生效（这两类硬约束不因「全开放」而失效）。此后以该文件为准。权限配置文件对任何 Agent（含 `master`）一律不可读写。例外依赖其权限配置，配置缺失退化后不再享有。

#### 调用方（caller）归因

权限评估的 caller 是**触发本轮执行的 User 上下文**，不是会话级持久化字段：

- 每轮执行开始时，Session 确定本轮 User 并固定为本轮 caller；同一轮内所有工具调用、审批提交、回调、审计共用该 caller。
- 工具结果触发的后续工具调用仍属于同一轮，沿用同一 caller。
- 多用户会话中，每条触发消息各自开启一轮、各自归因；不得用会话创建者、最近发送者或 checkpoint 中持久化的 sender 代替本轮 caller。
- Owner 豁免只在 `caller.user_id == "owner"` 时生效；会话 Owner 或注入的 `owner_id` 不等于 caller 为 Owner。非 Owner 消息触发的工具调用不享受 Owner 豁免。
- 子 Session 的 caller 继承父 Session 本轮 caller。
- caller 是权限评估的必要输入：取不到 caller 说明触发链路存在缺陷，此时一律按拒绝处理并记录调试日志，不得回落为 Agent 默认权限。

#### Owner 特权

需求 F4 的四项特权：

- **User 维度豁免**：任何 caller 为 Owner 的权限评估都跳过用户维度规则，只评估 Agent 维度；Owner 自身调用被拦截时，审批决策也只有 Agent 维度可选。
- **权限配置独占**：权限规则仅 Owner 可修改——通过权限指令、审批加入白名单或新用户注册三条路径（见 [权限配置管理](permission-configuration.md#修改入口)）。权限配置文件对任何 Agent 一律不可读写，不进入审批，也不存在放行路径。
- **审批独占**：所有审批请求只推送给 Owner（见 [审批工作流](approval-workflow.md)）。
- **命令执行豁免**：仅限 Owner 的斜杠命令类操作（`/exec`、`/git` 等）——默认放行，不进入权限评估与审批，不受 Agent 维度命令规则与沙盒模式约束；黑名单例外见 [权限维度](permission-dimensions.md#命令行)。Owner 通过 bash 等工具执行命令时仍按 Agent 与命令行维度评估（只跳过用户维度）。

Owner 的其它操作仍按正常权限评估；被拦截时仍进入审批。非命令类斜杠指令（含审批指令与权限指令）不进入权限评估（见「模块边界」）。

### 决策顺序与交集

需求 [permission §F3](../../requirements/permission.md) 定义了三段决策顺序，跨全部维度适用：

1. **权限配置文件保护**：权限配置文件的读写一律直接拒绝（硬编码保护，见 [权限配置管理](permission-configuration.md#配置保护)）。
2. **沙盒引擎不可用**：受沙盒约束的操作直接拒绝（终局，不降级、不进入审批，见 [沙盒](sandbox.md#不降级)）。
3. **高风险名单**：命中名单的操作只能单次放行，不经其它任何放行路径（名单内容见 [审批工作流](approval-workflow.md#高风险操作)）；配置写入维度的操作天然属高风险。Owner 的斜杠命令执行豁免不受此条约束。
4. **workspace 自动放行**：Agent-User 组合对其专属 workspace 路径的读写自动放行，不受任何拒绝规则影响（路径格式见下）。
5. **交集求值**：其余操作按 Agent 维度与用户维度求交集——双方都允许才放行，任一方拒绝即拒绝；Owner 作为 caller 时跳过用户维度。

- **Agent 维度规则**只匹配 Agent ID，定义 Agent 自身能力边界。
- **用户维度规则**主体为 User，只匹配 User ID，对该 User 在所有 Agent 上生效；它只能在 Agent 维度已允许的范围内起作用，不扩大 Agent 自身能力（交集语义）。命令行与工具调用两个维度只能以 Agent 为规则主体。
- **workspace 强制授权**：每个 Agent-User 组合的专属路径为 `{系统标准配置目录}/workspaces/{agent_id}/{user_id}/`（配置目录见 [platform](../platform/README.md)），由系统创建。该路径的读写直接放行，不参与维度求值，不受拒绝规则影响；命中高风险名单时仍按高风险审批处理。
- **子 Session 继承**：见「子 Agent 权限继承」。

### 规则加载策略

- **全局默认策略**为内置基线，在 Daemon 初始化阶段装配，与具体 Agent 无关；不依赖配置文件。
- **全局权限配置**（用户维度规则与审批等待时长）位于 `{配置目录}/config/permissions.json`（config 的权限配置类别），在 Daemon 初始化阶段加载并热更新，不随 Agent 数量变化。
- **Agent 的权限配置（Agent 维度规则、命令黑白名单、沙盒模式）**延迟加载：首次查询某 Agent 时，从 `{配置目录}/agents/<agent-id>/permissions.json` 读取并缓存**解析后的规则集**；启动速度不受 Agent 数量影响，未使用的 Agent 不占内存。
- **缓存边界**：缓存的是规则集，不是评估结论。每次操作都重新求值；规则文件变更后原子替换规则集，下一次评估即使用新规则。热加载机制见 [权限配置管理](permission-configuration.md#写入与热加载)。
- **审批等待时长**属权限配置的全局段配置项，定义与回退规则见 [权限配置管理](permission-configuration.md#配置文件与结构)。
- Daemon 初始化时 Permission Engine 与 AgentRegistry 同层并行；全局基线不依赖 Agent 配置，Agent 规则在首次评估时延迟加载，因此并行初始化不构成依赖倒置。

### 评估流程

```
操作请求（caller + 操作类型 + 参数）
  │
  ├─ 权限配置文件的访问？ → 硬编码拒绝（不审批、无放行路径）
  ├─ 沙盒引擎不可用且操作受沙盒约束？ → 直接拒绝（不降级、不审批）
  ├─ 命中高风险名单或属配置写入维度？ → 审批（仅单次放行）
  ├─ workspace 专属路径的读/写？ → 直接放行
  ├─ 按操作类型进入维度：文件读写 / 命令行 / 工具 / 网络 / 收发消息 / 配置写入
  ├─ 收集候选规则：Agent 维度规则 + 用户维度规则 + 子 Session 链路的拒绝约束
  ├─ Agent 维度求值（按该维度的判定语义，见下）
  ├─ 用户维度求值：caller 为 Owner → 跳过；否则按允许清单语义求值
  └─ 两维度都允许 → 放行（携带执行策略）；否则按 [审批工作流](approval-workflow.md#审批分流) 分流
```

**各维度的判定语义**分两类，评估流程按维度取对应语义：

| 语义 | 维度 | 无规则时的结论 |
|------|------|---------------|
| 允许清单 | 读、写、网络访问、命令行（白名单）、配置写入 | 空集（不允许） |
| 基线 + 收窄 | 工具调用（基线 = 配置档案的工具范围）、收发消息（基线 = Agent 允许） | 按基线（配置档案范围内可调用、Agent 可收发消息） |

- 命令行维度按序分流：黑名单（命中即拒绝，不审批）→ 命令安全解析（malicious 拦截 / uncertain 安全审批）→ 白名单（宿主或沙盒内执行，免审批）→ 其余按沙盒模式与执行位置决定（见 [权限维度](permission-dimensions.md#命令行)、[沙盒](sandbox.md#命令分流)）。
- 文件读写维度的路径范围由沙盒在操作系统层强制；规则层拒绝与沙盒边界同时生效。边界随放行结论以执行策略交给 [SandboxExecutor](../common/core-traits.md#sandboxexecutor)（放行模式下不经沙盒强制，仅按读/写规则评估）。
- 收发消息维度在消息进入/离开 Agent 时评估：User 消息送入 LLM 前、LLM/工具消息发往 User 前；消息路由与格式解析本身不评估。
- Agent 被拒绝时按最小信息原则返回（定义见 [权限配置管理](permission-configuration.md#配置保护)）。
- 评估开销仅与当前生效规则集相关，不随 Session 数量、Agent 数量、历史审批记录或运行时长增长。

### 模块边界

权限系统审查三类行为：用户指令、LLM 决策产生的工具调用，以及消息进出 Agent（收发消息维度）。系统自身的调度、路由、渲染不经过权限检查：

| 需要权限检查 | 不需要权限检查 |
|-------------|---------------|
| Agent 调用工具（含 sessions_steer / sessions_kill，按工具维度评估；目标 Session 归属关系另由 Session 校验） | Session 生命周期管理、compaction |
| 命令类斜杠指令（`/git` 写操作等；Owner 命令执行豁免除外） | `/exec`（仅 Owner 可用，非 Owner 直接权限不足，不经权限引擎） |
| 消息进出 Agent（收发消息维度） | 非命令类斜杠指令（状态查询、设置等） |
| | 审批指令与权限指令（Gateway 硬拦截；审批指令走审批队列，权限指令由权限指令处理层执行） |
| | CLI 管理命令（Owner 本地执行，不经过权限引擎） |
| | Daemon/Gateway 消息路由、消息格式解析与渲染 |
| | System Prompt 构建与注入 |
| | 权限检查逻辑本身 |
| | 系统级 Workflow 工具（按 [tools 安全边界](../tools/README.md) 的例外处理） |
| | spawn 工具（可用性由层级深度决定，不受工具范围与工具调用维度收窄；目标白名单与并发数量约束其请求能否成功） |

### 调试日志

（需求 §F11）权限系统在以下环节记录调试日志，经 [debug_log](../debug_log/README.md) 框架输出：

- 权限评估结果：放行/拒绝 + 命中的规则标识（仅内部日志，不返回给 Agent）。
- 审批生命周期：创建、推送、单次放行、加入白名单、拒绝、超时。
- 权限规则与沙盒模式变更。
- 沙盒判定与拒绝事件。

审计条目（AuditLogEntry）由 permission 实现并落盘，属共享类型契约；F11 要求的触发规则记录在权限评估调试日志中。

### 子 Agent 权限继承

父 Session spawn 子 Session 时：

- 子 Session 实际权限 = 目标 Agent 配置权限 ∩ 父 Session 实际权限 ∩ 继承的 User 上下文权限；沿 spawn 链路只能变窄。
- 交集按维度计算：路径/命令/网络按允许清单取交集；工具调用与收发消息按生效集合取交集；每一级父 Agent 的拒绝规则沿链路传播，拒绝约束只增不减。
- 某维度在一方无对应规则时，按该维度的判定语义取值（允许清单型为空集、基线型取基线），不因「无规则」额外判拒绝。超时等 Agent 执行参数不属于权限维度，不参与交集。
- 子 Session 被拒时静默返回拒绝，不进入审批。
- caller 为 Owner 时用户维度豁免，交集只取 Agent 链路；caller 继承自父 Session，不重新按会话归属判定。
- workspace 自动授权是上述交集的例外，不因父链路拒绝收窄；命中高风险名单时仍按高风险审批处理。
- spawn 本身是否被允许由父 Agent 与目标 Agent 的配置决定（层级深度、并发数量、目标白名单），不经权限评估（见 [权限维度](permission-dimensions.md#工具调用)）。

## 数据流

```
User 消息 / CLI 指令 / 子 Session 工具调用
  │
  ▼
Session 确定本轮 caller（消息发送者 / CLI Owner / 父 Session User）
  │
  ├─ 斜杠指令 → Gateway 拦截
  │     ├─ 审批指令 → 按审批单号提交决策（任意会话均可）
  │     ├─ 权限指令 → 权限指令处理层执行
  │     ├─ 非命令类 → 直接执行
  │     └─ 命令类（/git 写操作等）
  │           ├─ Owner → 直接放行（黑名单拒绝），不评估、不审批、不经沙盒
  │           └─ 非 Owner → 权限评估 → 放行执行 / 拒绝审批
  │
  └─ 普通消息 → Agent Session → LLM
        │
        ▼
     LLM 输出工具调用
        │
        ▼
     tools 解析 caller（ToolContext.user_id）+ 操作
        │
        ▼
     权限引擎评估
        │
        ├─ 放行 → 按执行策略执行（沙盒经 SandboxExecutor / 宿主直执行）
        └─ 拒绝 → 按 [审批工作流](approval-workflow.md#审批分流) 分流
```

关键分支：

- **caller 归因**：按触发本轮的消息发送者，不按会话 Owner；Owner 豁免只对 Owner caller 生效；caller 不可得即拒绝。
- **决策顺序**：权限配置文件保护 → 沙盒引擎不可用（终局拒绝）→ 高风险名单（仅单次放行）→ workspace 自动放行 → 交集（见 [权限配置管理](permission-configuration.md#配置保护)、[沙盒](sandbox.md#不降级)、[审批工作流](approval-workflow.md#高风险操作)）。
- **沙盒**：文件访问被拒（越界或规则拒绝）进入审批（不是 OS 硬拒绝）；沙盒内的命令由 OS 边界强制；宿主上执行在默认模式进审批、严格模式直接拒绝、放行模式缺省即在宿主执行。
- **审批**：子 Session 静默拒绝；黑名单命令、沙盒不可用、严格模式白名单外命令、权限配置文件访问直接拒绝；其余拒绝、宿主上执行请求、命令安全解析可疑、新用户注册进入审批。操作类审批单号全局唯一、可跨会话审批，超时自动拒绝且超时后指令返回「该审批已超时」，Owner 离线时不补推。

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

- **上游**：tools（工具调用时传入 caller + 操作）、Gateway（命令类斜杠指令拦截后传入、审批指令的决策提交）、Session（消息收发、本轮 caller 与子 Session 继承上下文）、daemon（装配 PermissionGate 适配器与 SandboxExecutor、ApprovalNotifier 注入；AuditLogger 由 permission 实现，daemon 装配并注入消费方）
- **下游**：Gateway（实现 [common ApprovalNotifier](../common/core-traits.md#approvalnotifier) 的审批推送通道与决策提交入口）、IM Adapter（经 Gateway 渲染审批卡片、终端提示）、tools（文件访问与命令执行经 [common SandboxExecutor](../common/core-traits.md#sandboxexecutor) 受沙盒约束；审计查询经 AuditLogger）、Session（审批完成后恢复执行）、CLI（`rule check/list` 只读读取规则文件）、debug_log（评估、审批、规则变更与沙盒事件）
- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：PermissionGate、SandboxExecutor、AuditLogger；消费：SessionLookup、ApprovalNotifier）、[common/shared-types](../common/shared-types.md)（PermissionRequest / PermissionOperation / PermissionDecision / ExecutionPolicy / PermissionDenied、RiskLevel、ApprovalCard / ApprovalDecision / ApprovalDelivery、ToolContext.user_id、AuditLogEntry / AuditDisposition / AuditLogFilter、UserRegistration / UserCreationRequest）
- **相关**：slash（命令类斜杠指令的语义、审批指令与权限指令，经 Gateway 间接调用）、platform（系统标准配置目录、操作系统差异隔离）
- **无关**：LLM Provider（不参与权限判定）、Session 生命周期管理（创建、压缩、归档、清理不经过权限检查；子 Session 的权限继承见上）
