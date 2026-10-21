# 权限系统

> 子功能文档：
> - [权限维度](permission-dimensions.md) — 八类操作的权限定义与执行位置
> - [沙盒](sandbox.md) — 操作系统级文件访问范围强制
> - [审批工作流](approval-workflow.md) — 拒绝操作的审批全链路
> - [权限配置管理](permission-configuration.md) — 规则修改、热加载、加载容错与配置保护

---

## 概述

- 关联需求文档：[requirements/permission.md](../../requirements/permission.md)
- 一句话：判断某个 Agent 以某个 User 身份能否执行某个操作；允许则执行，拒绝则按场景静默 Deny、通知 Owner 或进入审批。

## 架构

### 核心组件

```
                    ┌──────────────────────────────┐
操作请求 ──────────►│ 权限引擎                      │
(caller + 操作)     │  规则集：Agent / User+Agent   │
                    │  沙盒模式 / 命令黑白名单      │
                    │  评估：交集 + 强制授权 + Deny │
                    └──────┬───────────────┬───────┘
                           │ Allow         │ Deny
                           ▼               ▼
                      tools/命令执行   审批工作流 / 静默 / 通知
                                           │
                                     Owner 决策
                                           │
                                     Gateway 回调
```

- **权限引擎**：加载并缓存规则文件，按请求求值；不缓存评估结论。
- **沙盒引擎**：以操作系统原生隔离强制文件访问范围，经 [common/core-traits](../common/core-traits.md) 的沙盒执行接口注入 tools 与命令执行（见 [沙盒](sandbox.md)）。
- **审批队列**：内存队列，管理 Deny 操作与新用户注册请求的去重、超时与回调（见 [审批工作流](approval-workflow.md)）。
- **规则配置层**：解析、校验、持久化规则文件，触发热加载（见 [权限配置管理](permission-configuration.md)）。
- **调试日志**：评估结果、审批生命周期、规则变更与心跳处置写入调试日志。

### 身份体系

两种身份共同决定权限：

- **Agent 身份**：每个 Agent 有独立权限配置，默认只能收发消息，无文件读写、命令行、网络访问。
- **User 身份**：通过 IM 渠道 + IM User ID 绑定；一个 User 可绑定多个渠道，映射为同一 CloseClaw User。新建 User 默认无任何权限（含收发消息），需 Owner 授予后才能交互。不区分用户组，权限以个人为单位。
- Owner 的 User ID 固定为 `owner`，额外拥有 CLI 渠道访问方式。

**User 上下文来源**：

| 场景 | 本轮 User 上下文 |
|------|------------------|
| IM 消息 | 该消息发送者映射出的 User |
| CLI 对话 | Owner |
| 心跳 | 任务配置中指定的 User ID；未配置或未注册时跳过任务并通知 Owner |
| 子 Session | 继承父 Session 本轮 User 上下文 |

#### 调用方（caller）归因

权限评估的 caller 是**触发本轮执行的 User 上下文**，不是会话级持久化字段：

- 每轮执行开始时，Session 确定本轮 User 并固定为本轮 caller；同一轮内所有工具调用、审批提交、回调、审计共用该 caller。
- 工具结果触发的后续工具调用仍属于同一轮，沿用同一 caller。
- 多用户会话中，每条触发消息各自开启一轮、各自归因；不得用会话创建者、最近发送者或 checkpoint 中持久化的 sender 代替本轮 caller。
- Owner 豁免只在 `caller.user_id == "owner"` 时生效；会话 Owner 或注入的 `owner_id` 不等于 caller 为 Owner。非 Owner 消息触发的工具调用不享受 Owner 豁免。
- 心跳没有消息发送者，caller 取任务配置的 User；子 Session 的 caller 继承父 Session 本轮 caller。

这条归因规则解决的是「工具调用属于谁」：工具调用不因会话归属于某个人而获得那人的权限。

#### Owner 特权

需求 F4 的四项特权：

- **User 维度豁免**：任何 caller 为 Owner 的权限评估都跳过 User 维度规则，只评估 Agent 维度。
- **权限配置独占**：权限配置文件仅 Owner 可直接写入；Agent 读取默认拒绝，仅经 Owner 单次放行；Agent 写入一律拒绝（见 [权限配置管理](permission-configuration.md#配置保护)）。
- **审批独占**：所有审批请求只推送给 Owner（见 [审批工作流](approval-workflow.md)）。
- **命令执行豁免**：Owner 的斜杠命令类操作默认放行，不进入权限评估与审批；黑名单例外与沙盒细节见 [权限维度](permission-dimensions.md#命令行)。

Owner 的其它操作仍按正常权限评估；被拦截时仍进入审批。非命令类斜杠指令不进入权限评估（见「模块边界」）。

### 交集模型

权限评估采用 Agent 维度与 User 维度的交集：

```
Agent 维度规则    User+Agent 维度规则
     │                    │
     └─────────┬──────────┘
               ▼
         交集求值引擎
     Agent Allow AND User Allow → Allow
     任一方 Deny → Deny
```

- **Agent 维度规则**只匹配 Agent ID，定义 Agent 自身能力边界。
- **User+Agent 规则**同时匹配 User ID 与 Agent ID，实现个人粒度的收窄；对某个 User 的授权只作用于该 Agent 的规则集，需要跨 Agent 时分别授权。
- Agent 维度规则与 User+Agent 规则在同一引擎求值，双方都 Allow 才放行（workspace 强制授权除外，直接 Allow）。
- **Owner 豁免**：caller 为 Owner 时跳过所有 User 维度规则，仅评估 Agent 维度（见「Owner 特权」）。
- **workspace 强制授权**：每个 Agent-User 组合的专属路径固定为 `{数据根目录}/workspaces/{agent_id}/{user_id}/`，由系统创建。该路径的读/写直接 Allow，不参与 Agent/User 维度求值，不受任何 Deny 规则（含沿 spawn 链路传播的 Deny）影响，也不随 `/cd` 或 spawn 工作目录参数变化；命中高风险操作名单（如版本控制目录读写）时仍按高风险审批处理。
- **默认基线**：见 [权限配置管理](permission-configuration.md#配置文件与结构)。
- **子 Session 继承**：见「子 Agent 权限继承」。

### 规则加载策略

- **全局默认策略**为内置基线，在 Daemon 初始化阶段装配，与具体 Agent 无关；不依赖配置文件。
- **Agent 的权限规则（含 Agent 维度与 User+Agent 规则）**延迟加载：首次查询某 Agent 时，从 `~/.closeclaw/agents/<agent-id>/permissions.json` 读取并缓存**解析后的规则集**；启动速度不受 Agent 数量影响，未使用的 Agent 不占内存。
- **缓存边界**：缓存的是规则集，不是评估结论。每次操作都重新求值；规则文件变更后原子替换规则集，下一次评估即使用新规则。热加载机制见 [权限配置管理](permission-configuration.md#写入与热加载)。
- Daemon 初始化时 Permission Engine 与 AgentRegistry 同层并行；全局基线不依赖 Agent 配置，Agent 规则在首次评估时延迟加载，因此并行初始化不构成依赖倒置。

### 评估流程

```
操作请求（caller + 操作类型 + 参数）
  │
  ├─ 权限配置文件读写？ → 硬编码保护：默认 Deny（Owner 单次放行除外，不可白名单）
  │
  ├─ workspace 专属路径的读/写？ → 直接 Allow（不参与维度求值；命中高风险仍审批）
  │
  ├─ 按操作类型进入维度：工具 / 文件读写 / 命令行 / 网络 / 跨 Agent / 配置写入 / 收发消息
  │
  ├─ 收集候选规则：Agent 规则 + User+Agent 规则 + 子 Session 链路的 Deny 约束
  │
  ├─ Agent 维度求值：命中 Deny → Deny；无 Deny 且有 Allow → Allow；否则 Deny
  │
  ├─ User 维度求值：caller 为 Owner → 跳过；否则同样求值
  │
  ├─ 两维度都 Allow → Allow；任一方 Deny → Deny
  │
  └─ Deny → 按 [审批工作流](approval-workflow.md#审批分流) 分流
```

- 命令行维度先判黑名单（命中即 Deny，不审批），再判白名单（宿主执行），其余按沙盒模式决定执行位置（见 [权限维度](permission-dimensions.md#命令行)）。
- 文件读写维度的路径范围由沙盒在操作系统层强制；沙盒把规则允许的文件范围落到 OS 层，规则层 Deny 与沙盒边界同时生效（workspace 专属路径与 `always allow` 模式除外：workspace 专属路径直接 Allow，但高风险命中仍审批）。
- 收发消息维度在消息进入/离开 Agent 时评估：User 消息送入 LLM 前、LLM/工具消息发往 User 前；消息路由与格式解析本身不评估。
- Agent 被拒绝时只收到操作被拒绝的结果，不暴露规则名称、优先级或配置路径；细节见 [权限配置管理](permission-configuration.md#配置保护)。
- 评估开销仅与当前生效规则集相关，不随 Session 数量、Agent 数量、历史审批记录或运行时长增长。

### 模块边界

权限系统只审查两类行为：用户指令与 LLM 决策产生的工具调用。系统自身的调度、路由、渲染不经过权限检查：

| 需要权限检查 | 不需要权限检查 |
|-------------|---------------|
| Agent 调用工具 | Session 生命周期管理、compaction |
| 命令类斜杠指令（`/exec`、`/git` 写操作等；Owner 命令执行豁免除外） | 非命令类斜杠指令（状态查询、设置等） |
| | 用户注册等权限管理指令（Gateway 硬拦截并验证 Owner；不经权限引擎评估，注册请求走审批队列） |
| | CLI Admin 命令（Owner 本地执行，不经过权限引擎） |
| | Daemon/Gateway 消息路由 |
| | IM 消息格式解析与渲染 |
| | System Prompt 构建与注入 |
| | 权限检查逻辑本身 |
| | LLM 普通文本消息（非工具调用格式） |
| | 系统级 Workflow 工具（按 [tools 安全边界](../tools/README.md) 的例外处理） |

### 调试日志

（需求 §F11）权限系统在以下环节记录调试日志，经 [debug_log](../debug_log/README.md) 框架输出：

- 权限评估结果：放行/拒绝 + 命中的规则标识（仅内部日志，不返回给 Agent）。
- 审批生命周期：创建、推送、单次放行、加入白名单、拒绝、超时。
- 权限规则变更。
- 心跳相关的权限决策与处置（拒绝/静默跳过/通知 Owner）。
- 沙盒判定与拒绝事件。

审计条目（AuditLogEntry）由 permission 实现并落盘，属共享类型契约；F11 要求的触发规则记录在权限评估调试日志中。

### 子 Agent 权限继承

父 Session spawn 子 Session 时：

- 子 Session 实际权限 = 目标 Agent 配置权限 ∩ 父 Session 实际权限 ∩ 继承的 User 上下文权限。
- 权限沿 spawn 链路只能变窄，不能变宽；每一级父 Agent 的 Deny 沿链路传播，Deny 约束只增不减。
- 交集按维度字段计算：路径/命令/工具/网络主机端口取双方允许范围的交集，布尔维度任一方 Deny 即 Deny；任一维度在一方缺失视为 Deny。timeout_ms 属 Agent 执行参数，不在权限维度内。
- 子 Session 的权限评估除交集外，还须检查父链传入的 Deny 约束。
- 子 Session 被拒时静默返回 Deny，不进入审批。
- caller 为 Owner 时 User 维度豁免，子 Session 的交集只取 Agent 链路；caller 继承自父 Session，不重新按会话归属判定。
- workspace 强制授权是上述交集的例外，不因父链路 Deny 收窄。

## 数据流

```
User 消息 / CLI 指令 / 心跳
  │
  ▼
Session 确定本轮 caller（消息发送者 / CLI Owner / 心跳任务 User / 父 Session User）
  │
  ├─ 斜杠指令 → Gateway 拦截
  │     ├─ 审批指令 → 审批工作流
  │     ├─ 非命令类 → 直接执行
  │     └─ 命令类
  │           ├─ Owner → 直接放行（黑名单拒绝），不评估、不审批、不受沙盒约束
  │           └─ 非 Owner → 权限评估 → Allow 执行 / Deny 审批
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
        ├─ Allow → 工具执行 / 宿主或沙盒命令执行
        └─ Deny → 按 [审批工作流](approval-workflow.md#审批分流) 分流
```

关键分支：

- **caller 归因**：按触发本轮的消息发送者，不按会话 Owner；Owner 豁免只对 Owner caller 生效。
- **沙盒**：文件越界与沙盒内命令由 OS 级边界强制；移出沙盒执行走审批。
- **审批**：子 Session 与心跳不进审批；黑名单与 `whitelist` 模式白名单外命令直接拒绝。

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

- **上游**：tools（工具调用时传入 caller + 操作）、Gateway（命令类斜杠指令拦截后传入、审批指令与审批回调）、daemon（装配 PermissionEvaluator / ApprovalSubmission 适配器；AuditLogger 由 permission 实现，daemon 装配并注入消费方）
- **下游**：Gateway（审批卡片推送通道、审批结果回调通道；Agent Session 经 Gateway 接收回调）、Session（spawn 权限判定经 PermissionChecker、审批完成后恢复执行）、tools（审计查询经 AuditLogger）、CLI Admin（`rule check/list` 只读读取规则文件）、debug_log（评估、审批、规则变更与沙盒事件）
- **共享类型 / 核心 trait**：[common/core-traits](../common/core-traits.md)（实现：AuditLogger；消费：SessionLookup。PermissionEvaluator / ApprovalSubmission 由 daemon 适配，PermissionChecker 由 Gateway 适配；三者分别是 agent 间消息评估、审批提交、spawn 校验的窄接口；文件/命令行/网络等通用评估经 common 的权限评估接口（待补，见 common doc-gap）调用权限引擎，tools 不直接依赖 permission crate）、[common/shared-types](../common/shared-types.md)（CallerInfo、ToolContext.user_id、RiskLevel、PermissionEvalResponse、PermissionDenied、SpawnPermissionError、AuditLogEntry / AuditDisposition / AuditLogFilter、UserRegistration / UserCreationRequest / InitialPermissionSet）
- **相关**：slash（命令类斜杠指令的语义与审批入口，经 Gateway 间接调用）
- **无关**：IM Adapter（审批卡片的平台渲染由外部层 Adapter 处理，不涉及权限判断）、LLM Provider（不参与权限判定）、Session 生命周期管理（创建、压缩、归档、清理不经过权限检查；spawn 权限继承由 Session 的 SpawnController 经 PermissionChecker 执行）
