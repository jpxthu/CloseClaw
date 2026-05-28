# Agent

## 概述

Agent 模块是 CloseClaw 中"谁在执行任务"的定义层。它负责三件事：Agent 配置档案管理、Agent 孵化（spawn）协调、权限沿 spawn 链路继承。

Agent 本身不管理运行时状态——运行时由 session 模块管理。Agent 不持有对话历史——对话历史在 session 的 transcript 中。Agent 的身份人格由 Bootstrap 文件定义，加载由 system prompt 模块处理。

## 架构

Agent 模块以配置和协调逻辑的形式嵌入系统：session 创建时读 agent 配置，spawn 时介入控制 child session 的创建参数，权限评估时提供 agent 维度的基线规则。

```
        用户 / 外部事件
              │
              ▼
        Gateway / Daemon ─── 确定目标 agent，触发 session 创建
              │
              ▼
    ┌─────────────────────────────────────────┐
    │           Agent 模块                     │
    │                                         │
    │  Agent 配置档案（JSON）                  │
    │       ├── model / workspace / agentDir  │
    │       ├── bootstrapMode / skills        │
    │       ├── tools / permissions baseline  │
    │       └── subagents config（spawn 控制） │
    │                                         │
    │  Spawn 协调                              │
    │       ├── sessions_spawn 工具入口       │
    │       ├── 前置检查（depth/并发/白名单）  │
    │       ├── child session 参数组装        │
    │       └── announce 结果回传注册          │
    │                                         │
    │  权限沿链路继承                          │
    └──────────────┬──────────────────────────┘
                   │
      ┌────────────┼────────────┐
      ▼            ▼            ▼
   Session     Permission    System Prompt
   模块         模块           模块
```

核心组件：

- **Agent 配置档案**：每个 agent 对应一份 JSON 配置，定义其能力边界（模型、工具、workspace、权限基线、spawn 约束）。存储支持项目级、用户级、系统内置三级优先级，字段级覆盖合并。
- **Agent 类型**：框架内置仅一个通用 agent（全工具、全 bootstrap），其他 agent 由用户通过 JSON 配置文件自定义。Agent 能力完全由配置字段组合决定。
- **Spawn 协调**：父 agent 通过 sessions_spawn 工具创建子 session。协调层负责前置检查（深度、并发、白名单）、参数组装、announce 回传注册。参见 `agent-spawn.md`。
- **Fork 模式**：spawn 的变体，在子 session 中注入父 agent 的对话历史，使子 agent 继承上下文认知。参见 `agent-spawn.md`。
- **权限继承**：子 agent 的实际权限沿 spawn 链路收窄——只能收窄，不能放宽。参见 `agent-permissions.md`。

子功能文档：

| 文档 | 内容 |
|------|------|
| `agent-config.md` | Agent JSON 配置档案：字段定义、存储位置、加载优先级、字段级合并 |
| `agent-spawn.md` | Spawn 机制、Fork 模式、Steer/Kill、Announce 回传、Depth 追踪 |
| `agent-permissions.md` | 权限沿 spawn 链路继承、workspace 路径授权 |

## 数据流

### Agent 配置加载

系统启动时加载所有 agent 配置，生成 ResolvedAgentConfig 注册到内存：

```
内置 agent 定义（仅 general-purpose）
  +
扫描用户级配置（~/.closeclaw/agents/*.json）
  +
扫描项目级配置（<cwd>/.closeclaw/agents/*.json）
  ↓
按优先级合并同 ID 的 agent 配置（字段级覆盖）
  ↓
生成 ResolvedAgentConfig（所有字段已补齐默认值）
  ↓
注册到内存配置注册表
```

### Session 创建时读取 Agent 配置

```
Gateway/Daemon 确定目标 agent ID
  ↓
Session 模块读取该 agent 的 ResolvedAgentConfig
  ↓ 分发各字段到对应模块
  model        → 设置 session 默认模型
  bootstrapMode → 决定 bootstrap 文件加载集
  agentDir     → bootstrap 文件读取路径
  permissions  → 传递 agent_id 给权限模块，Permission 自行加载 Agent 权限规则
  skills       → 过滤 skill 注册表
  tools/disallowedTools → 过滤 tool 注册表
  subagents    → 注入 session 的 spawn 控制上下文
  ↓
Session 创建完成
```

### Spawn 运行时流程

```
父 agent 调用 sessions_spawn 工具
  ↓
Agent 协调层前置检查（depth/并发/白名单/requireAgentId/权限）
  ↓ （全部通过）
创建 child session（加载目标 agent 配置、注入 task、过滤工具集、workspace fallback）
  ↓
子 agent 执行 task
  ↓
子 session 完成 → announce 入队到父 session
  ↓
父 agent 下一轮 turn 处理 announce
```

## 模块关系

### 上游（谁调用 Agent 模块）

| 模块 | 调用关系 |
|------|---------|
| Gateway/Daemon | 外部消息到达时确定目标 agent，触发 session 创建 |
| Session | session 创建时读取 agent 配置档案，决定 bootstrap、模型、工具集、skills 过滤 |
| Skills | skill 在 fork 模式下通过 agent spawn 机制创建子 session |
| Tools | sessions_spawn / sessions_steer / sessions_kill 注册在 tools 模块，执行逻辑由 agent 模块提供 |

### 下游（Agent 模块调用谁）

| 模块 | 调用关系 |
|------|---------|
| Session | spawn 时创建 child session；steer/kill 时操作子 session |
| SkillRegistry | session 创建时根据 agent.skills 配置过滤可用 skill |
| System Prompt | agent 配置的 bootstrapMode 和 agentDir 决定 bootstrap 文件加载策略 |

### 无关（无调用关系、名称或功能易混淆）

| 模块 | 说明 |
|------|------|
| Card | 卡片渲染由 renderer 处理 |
| IM Adapter | 消息路由由 gateway 处理 |
| LLM Provider | agent 模块不直接调用 LLM |
| Permission | Agent 不直接调用 Permission；Permission 从 Agent 配置加载权限基线规则并在 spawn 时沿链路计算继承权限，属于数据依赖关系 |
| Processor Chain / Renderer | 消息出站处理与 agent 模块无关 |
