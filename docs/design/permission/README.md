# 权限系统

> 子功能文档：
> - [权限维度](permission-dimensions.md) — 七类操作的权限定义与关系
> - [审批工作流](approval-workflow.md) — 拒绝操作的审批全链路
> - [权限配置管理](permission-configuration.md) — 规则修改、热加载与配置保护

---

## 概述

权限系统是系统级身份型访问控制模块。它判断"某个 Agent 以某个 User 身份能否执行某个操作"，而不是判断"某个工具该不该执行"。

七类操作维度独立管理：读、写、命令行、工具、收发消息、网络、跨 Agent 通信、配置写入（详见 [权限维度](permission-dimensions.md)）。配置写入维度永远高危，只能走单次审批。

## 架构

### 身份体系

权限系统涉及两种身份，二者独立但共同决定最终权限：

**Agent 身份**：每个 Agent 实例有独立的权限配置。Agent 默认只能收发消息，无文件读写、无命令行、无网络访问。

**User 身份**：每个 User 通过 IM 渠道加 IM User ID 的组合唯一绑定。一个 User 可以绑定多个 IM 渠道（如飞书 open_id + Telegram user_id），映射为同一个 CloseClaw User ID。User 默认无任何权限，包括收发消息也需显式授予。不区分用户组，所有权限以个体为单位。

新用户首次使用时因默认无权限，连收发消息也需 Owner 事先授予初始权限后才能开始交互。

**代理 User**（Agent-as-User）：所有 Agent 操作均在某个 User 上下文中执行。User 来源取决于场景——IM 消息来自发送者、CLI 调用默认为 Owner、Heartbeat 使用任务配置中的写死 User ID、子 Agent 继承父 Agent 的 User 上下文。

Owner 的 User ID 固定为 `"owner"`，额外拥有 CLI 渠道访问方式。

### 交集模型

权限评估采用双主体单规则集架构：

```
Agent 维度规则    User 维度规则
     │                 │
     └───────┬─────────┘
             ▼
       交集求值引擎
             │
        Agent 维度 Allow AND User 维度 Allow → 放行
        任一方 Deny → 直接 Deny
```

- **Agent 规则**仅匹配 Agent ID，定义 Agent 自身的能力边界
- **User+Agent 规则**同时匹配 User ID 和 Agent ID，实现细粒度交集控制
- 两套规则在同一引擎中求值，双方都必须 Allow 操作才放行

Owner（User ID = `"owner"`）在引擎层面短路：当 caller 的 User ID 为 Owner 时，跳过所有 User 维度规则，仅评估 Agent 维度。

### 规则加载策略

**全局默认策略**在 Daemon 初始化阶段加载，与具体 Agent 无关。此时 Agent Config 尚未扫描完成，但这不影响——全局策略不涉及 Agent 维度。

**Agent 维度权限规则**采用延迟加载：不在 init 阶段全量扫描，而是在 `evaluate()` 首次查询某个 Agent 时按需从 `~/.closeclaw/agents/<agent-id>/permissions.json` 加载并缓存。此设计的优势：
- 启动速度不受 agent 数量影响
- 天然适配热重载：agent 配置或权限文件变更后，下次访问自动读到最新规则
- 未被使用的 agent 不消耗内存

代价是首次查询某 agent 时多一次文件 I/O，agent 数量通常有限，可忽略。

这意味着 Daemon 初始化顺序中 Permission Engine（阶段一 #3）在 Agent Config 扫描（阶段一 #4）之前是刻意设计，非依赖倒置。

### Workspace 路径强制授权

每个 Agent-User 组合自动获得其 workspace 路径的读写权限：

```
{数据根目录}/workspaces/{agent_id}/{user_id}/
```

这是硬编码的强制授权，不受任何规则 Deny 的影响。即使 Agent 和 User 的权限规则都未覆盖此路径，此路径仍然可读写。Agent 的 system prompt 中只注入最终路径值，不暴露权限级别或配置来源。

### 评估流程

Agent 不能直接调用权限引擎。完整调用链：

```
LLM 输出工具调用意图
  │
  ▼
tools 模块解析 → 生成内部消息 { agent, user, 操作 }
  │
  ▼
权限引擎 evaluate(caller, request)
  │
  ├─ Owner 短路：caller 为 Owner → 仅评估 Agent 维度
  │
  ├─ Creator 规则检查：caller 为 Agent 创建者 → 跳过 User 维度
  │
  ├─ 候选规则收集：User+Agent 索引 + Agent-Only 索引 + Glob 回退
  │
  ├─ 按优先级降序排序
  │
  ├─ 逐条匹配：规则同时满足主体匹配 + 操作匹配时参与评估
  │   遇 Deny → 立即 Deny；无 Deny 但有 Allow → Allow
  │
  └─ 默认策略：全 Deny
```

### 模块边界

权限系统只对用户行为和 LLM 行为进行审查——用户的指令和 LLM 决策产生的工具调用。系统自身的调度、路由、渲染都是固定逻辑，不经过权限检查：

| 需要权限检查 | 不需要权限检查 |
|-------------|---------------|
| Agent 调用工具 | Session 生命周期管理、compaction |
| 用户斜杠指令 | Daemon/Gateway 消息路由 |
| | IM 消息格式解析与渲染 |
| | System prompt 构建与注入 |
| | 权限检查逻辑本身 |
| | LLM 普通文本消息（非工具调用格式） |

Daemon/Gateway 将整个消息流程分为外部层（IM Adapter，负责消息渲染和用户输入解析）和内部层（Agent + 权限系统，只收发平台无关的内部消息），外部层不同 IM 只需开发自己的 Adapter。

### 状态机

```
                ┌─ 系统模块（不经过权限）→ 直接执行
                │
                ├─ 斜杠指令 → Gateway 硬拦截（不进 Agent session）
                │     ├─ Owner → 直接执行
                │     ├─ Non-owner 高危指令（/exec 等）→ 权限引擎 evaluate() → 默认 Deny
                │     └─ Non-owner 普通指令（/help、/status 等）→ 直接执行
                │
IM/CLI 入站 ────┤
                │
                └─ Agent 工具调用
                      │
                      ▼
              权限引擎 evaluate()
                      │
                 ┌────┼────┐
                 ▼    ▼    ▼
               Allow Deny  Deny
                 │    │     │
                 │    │     └─→ 分流：
                 │    │           ├─ 子 Agent → 静默 Deny
                 │    │           ├─ Heartbeat → skip/notify/ask
                 │    │           └─ 其他 → 推送 Owner 审批
                 │    │
                 ▼    ▼
               执行  执行/Deny
```

### 子 Agent 权限继承

父 Agent 发起 spawn 时，子 Agent 实际权限 = 子 Agent 自身权限 ∩ 父 Agent 权限 ∩ 继承的 User 权限。权限沿 spawn 链路只能变窄不能变宽。子 Agent 被 Deny 时静默返回，不进入审批。

## 数据流

```
User 消息 / CLI 指令
  │
  ▼
Gateway 入站路由
  │
  ├─ 斜杠指令 → Gateway 硬拦截（不进 Agent session）
  │     ├─ Owner → 验证发送者 = Owner → 直接执行
  │     ├─ Non-owner 高危指令（/exec 等）→ 权限引擎 evaluate() → 默认 Deny
  │     └─ Non-owner 普通指令（/help、/status 等）→ 直接执行
  │
  └─ 普通消息 → Agent session → LLM 推理
                    │
                    ▼
              LLM 输出工具调用
                    │
                    ▼
              tools 模块解析
                    │
                    ▼
              权限引擎 evaluate()
                    │
               ┌────┼────┐
               ▼    ▼    ▼
             Allow Deny  Deny(需审批)
               │    │     │
               ▼    ▼     ▼
             执行  静默   审批队列 → Owner 审批卡片
                   Deny        │
                          ┌────┼────┐
                          ▼    ▼    ▼
                       单次OK 白名单 拒绝
                          │    │    │
                          ▼    ▼    ▼
                        执行  写规则 Deny
```

- 关键分支：Owner 短路（跳过 User 维度）、Creator 规则（跳过 User 维度）
- 关键分支：子 Agent Deny 静默、Heartbeat Deny 按配置分流

## 模块关系

- **上游**：tools 模块（Agent 工具调用时传入 caller + 操作）、Gateway（用户斜杠指令拦截后传入）
- **下游**：审批系统（Deny 需审批时产出审批请求）、Agent Session（Allow/Deny 结果回调）
- **无关**：IM Processor（消息格式解析与渲染，不涉及权限判断；审批卡片渲染由外部层 IM Adapter 处理）
- **无关**：Session Manager（会话生命周期管理，不经过权限检查）
