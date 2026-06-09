# Agent

## 概述

Agent 模块是 CloseClaw 的配置定义层——定义每个 agent 的身份和能力边界。Agent struct 是纯配置档案，不持有运行时可变状态、不持有进程、不持有生命周期。对话运行时和进程执行状态由 Session 模块管理。

> **设计基调**：生命周期完全由 session 驱动，agent 就是一组 config。这条基调只有 owner 明确批准才能修改。

Agent 模块负责两件事：Agent 配置档案的定义、权限基线的提供。配置加载由 Config 模块负责（详见 [agent-config.md](agent-config.md)）。Agent 的身份人格由 Bootstrap 文件定义，加载由 System Prompt 模块处理。

## 架构

Agent 模块以纯配置层的形式嵌入系统：各方在需要时读取 agent 配置档案，按配置决定行为。

```
        用户 / 外部事件
              │
              ▼
        Gateway / Daemon ─── 确定目标 agent，触发 session 创建
              │
              ▼
    ┌─────────────────────────────────────────┐
    │           Agent 模块（纯配置层）          │
    │                                         │
    │  Agent 配置档案（JSON）                  │
    │       ├── model / workspace / agentDir  │
    │       ├── bootstrapMode / skills        │
    │       ├── tools / disallowedTools       │
    │       ├── subagents（spawn 控制参数）    │
    │       └── communication（跨 agent 交互权限）│
    │                                         │
    │  权限基线（permissions.json）            │
    └──────────────┬──────────────────────────┘
                   │
      ┌────────────┼────────────┐
      ▼            ▼            ▼
   Session     Permission    System Prompt
   模块         模块           模块
```

核心组件：

- **Agent 配置档案**：每个 agent 对应一个独立的配置目录（`agents/<id>/`），目录下存放 `config.json` 和 `permissions.json`。配置定义能力边界（模型、工具、workspace、spawn 控制、跨 agent 交互权限），权限独立存储。存储支持项目级和用户级两级优先级，字段级覆盖合并。详见 [agent-config.md](agent-config.md)。
- **Agent 能力模型**：Agent 能力由配置字段组合决定（详见 agent-config.md → Agent 能力模型）。初始 Agent 由 CLI 配置向导在首次运行时创建（默认 ID `master`），其他 agent 由用户通过配置文件自定义。
- **权限基线**：Agent 的 `permissions.json` 定义该 agent 的权限基线，由 Permission 模块在 spawn 时沿链路计算继承权限——子 agent 的实际权限只能收窄，不能放宽。详见 [agent-permissions.md](agent-permissions.md)。

子功能文档：

| 文档 | 内容 |
|------|------|
| `agent-config.md` | Agent JSON 配置档案：字段定义、存储位置、加载优先级、字段级合并 |
| `agent-spawn.md` | Spawn 机制、Fork 模式、Steer/Kill、Announce 回传、Depth 追踪 |
| `agent-permissions.md` | 权限沿 spawn 链路继承、workspace 路径授权 |

## 数据流

### Agent 配置加载

配置加载流程详见 [agent-config.md](agent-config.md) → 配置加载流程。

### Session 创建时读取 Agent 配置

```
Gateway/Daemon 确定目标 agent ID
  ↓
Session 模块读取该 agent 的 ResolvedAgentConfig
  ↓ 分发各字段到对应模块
  model        → 设置 session 默认模型
  workspace    → 设置 session 工作目录
  bootstrapMode → 决定 bootstrap 文件加载集
  agentDir     → bootstrap 文件读取路径
  skills       → 过滤 skill 注册表
  tools/disallowedTools → 过滤 tool 注册表
  subagents    → 注入 session 的 spawn 控制上下文
  communication → 注入 session 的跨 agent 交互权限

permissions.json → Permission 独立加载 Agent 权限基线
  ↓
Session 创建完成
```

### Spawn 控制流

```
父 session 调用 sessions_spawn 工具（由 Session 模块注册到 ToolRegistry）
  ↓
Session 模块读取父 agent 配置中的 subagents 参数
  → 前置检查（depth/并发/白名单/requireAgentId/权限）
  ↓ （全部通过）
Session 模块创建 child session（加载目标 agent 配置、注入 task、过滤工具集）
  ↓
子 session 执行 task
  ↓
子 session 完成 → announce 入队到父 session
  ↓
父 session 下一轮 turn 处理 announce
```

## 模块关系

### 上游（谁消费 Agent 配置）

| 模块 | 调用关系 |
|------|---------|
| Config | 启动时加载 Agent 注册清单和配置目录，生成 ResolvedAgentConfig |
| Gateway/Daemon | 外部消息到达时确定目标 agent ID，触发 session 创建 |
| Session | session 创建时读取 agent 配置档案并分发各字段；spawn 时读取 subagents / communication 控制参数；注册 sessions_spawn 等工具到 ToolRegistry |
| Permission | 从 permissions.json 加载 Agent 维度权限基线，在 spawn 时沿链路计算继承权限 |

### 下游（Agent 配置被谁消费）

| 模块 | 消费方式 |
|------|---------|
| Session | 读取 model、workspace、subagents、communication |
| System Prompt | 读取 bootstrapMode、agentDir |
| Permission | 读取 permissions.json 的权限基线规则 |
| Skill Registry | 读取 skills 白名单 |
| Tool Registry | 读取 tools 白名单、disallowedTools 黑名单 |

### 无关（无调用关系、名称或功能易混淆）

| 模块 | 说明 |
|------|------|
| Card | 卡片渲染由 renderer 处理 |
| IM Adapter | 消息路由由 gateway 处理 |
| LLM Provider | agent 模块不直接调用 LLM |
| Processor Chain / Renderer | 消息出站处理与 agent 模块无关 |
| Agent 进程生命周期 | agent 无独立进程；agent 进程由 Daemon 管理，执行状态由 Session 模块的 session-execution 机制跟踪 |
</tool_result>
