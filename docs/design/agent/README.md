# Agent

## 概述

- 关联需求文档：[agent.md](../../requirements/agent.md)
- Agent 模块是 CloseClaw 的配置定义层——定义每个 agent 的身份和能力边界。Agent 是纯配置档案，不持有运行时可变状态、不持有进程、不持有生命周期。对话运行时和进程执行状态由 Session 模块管理。

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
    │  AgentRegistry（运行时查询入口）          │
    │       ├── 启动时填充全部 agent 配置       │
    │       ├── 运行时只读查询                  │
    │       └── 热重载时替换全部配置            │
    │                                         │
    │  Agent 配置档案（JSON）                  │
    │       ├── model / workspace / agentDir  │
    │       ├── bootstrapMode / skills        │
    │       ├── tools / disallowedTools       │
    │       ├── subagents（spawn 控制参数）    │
    │       └── memory（可选覆盖默认记忆配置） │
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

- **AgentRegistry**：运行时配置查询入口，以 agent_id 为键提供 ResolvedAgentConfig 的只读查找。启动时由 Daemon 填充，运行时只读查询。详见 [agent-registry.md](agent-registry.md)。
- **Agent 配置档案**：每个 agent 对应一个独立的配置目录（`agents/<id>/`），目录下存放 `config.json` 和 `permissions.json`。配置定义能力边界（模型、工具、workspace、spawn 控制、跨 agent 交互权限），权限独立存储。存储支持项目级和用户级两级优先级，字段级覆盖合并。详见 [agent-config.md](agent-config.md)。
- **Agent 能力模型**：Agent 能力由配置字段组合决定（详见 agent-config.md → Agent 能力模型）。初始 Agent 由 CLI 配置向导在首次运行时创建（默认 ID `master`），其他 agent 由用户通过配置文件自定义。
- **权限基线**：Agent 的 `permissions.json` 定义该 agent 的权限基线，由 Permission 模块在 spawn 时沿链路计算继承权限——子 agent 的实际权限只能收窄，不能放宽。权限热更新独立于 agent 核心配置：修改 permissions.json 不影响 config.json 加载，反之亦然。每次操作前重新评估权限，变更即时生效。权限文件缺失时 agent 正常加载，使用系统默认权限。详见 [agent-permissions.md](agent-permissions.md)。

子功能文档：

| 文档 | 内容 |
|------|------|
| `agent-config.md` | Agent JSON 配置档案：字段定义、存储位置、加载优先级、字段级合并 |
| `agent-registry.md` | AgentRegistry 运行时配置查询入口：populate / get / reload 接口、数据流 |
| `agent-spawn.md` | Spawn 机制、Fork 模式、Steer/Kill、Announce 回传、Depth 追踪、Spawn 树形拓扑（存储/查询/级联 Kill/重启恢复） |
| `agent-permissions.md` | 权限沿 spawn 链路继承、workspace 路径授权 |

## 数据流

### Agent 配置加载

配置加载流程详见 [agent-config.md](agent-config.md) → 配置加载流程。

### Session 创建时读取 Agent 配置

1. Gateway/Daemon 确定目标 agent ID，从 AgentRegistry 获取 ResolvedAgentConfig
2. Session 模块分发各字段到对应子系统：
   - model → 设置 session 默认模型
   - workspace → 设置 session 工作目录
   - bootstrapMode → 决定 bootstrap 文件加载集
   - agentDir → bootstrap 文件读取路径
   - skills → 过滤 skill 注册表
   - tools/disallowedTools → 过滤 tool 注册表
   - subagents → 注入 session 的 spawn 控制上下文
   - memory → 覆盖 MemoryMiner 配置（可选，未指定时用全局默认）
3. Permission 独立加载 permissions.json，获取 Agent 权限基线（与步骤 2 的 config 字段加载路径并行，互不影响）
4. 以上步骤完成后 Session 创建结束

### Spawn 控制流

1. 父 session 调用 sessions_spawn 工具（由 Session 模块注册到 ToolRegistry）
2. SkillTool 触发 SpawnValidator 执行前置检查：
   - depth 检查
   - 并发检查
   - requireAgentId 检查
   - agentId 解析
   - 白名单检查
   - 权限检查
3. 全部通过后，SpawnController 创建 child session（加载目标 agent 配置、注入 task、过滤工具集）
4. 子 session 执行 task
5. 子 session 完成，结果通过 announce 机制入队到父 session
6. 父 session 下一轮 turn 处理 announce

## 模块关系

### 上游（调用 Agent 模块或 Agent 消费其产出数据）

| 模块 | 调用关系 |
|------|---------|
| Config | 扫描 agent 配置目录，加载并合并所有 agent 配置档案，产出 ResolvedAgentConfig 供 Daemon 填充注册表 |
| Gateway/Daemon | 查询注册表获取目标 agent 的完整配置，以 agent 配置为输入触发 session 创建 |

### 下游（消费 Agent 配置档案产出数据）

| 模块 | 消费方式 |
|------|---------|
| Session | 创建 session 时读取 agent 配置各字段并分发到对应子系统（模型选择、工作目录、bootstrap 模式、工具/技能过滤、spawn 控制参数） |
| Permission | 读取权限基线配置，在 spawn 时与其他维度共同计算继承权限 |
| System Prompt | 读取 agent 配置中的 bootstrapMode/agentDir 字段定位 bootstrap 文件路径，加载身份人格定义 |

### 无关（无调用关系、名称或功能易混淆）

| 模块 | 说明 |
|------|------|
| Card | 卡片渲染由 renderer 处理 |
| IM Adapter | 消息路由由 gateway 处理 |
| LLM Provider | agent 模块不直接调用 LLM |
| Processor Chain / Renderer | 消息出站处理与 agent 模块无关 |
| Agent 进程生命周期 | agent 无独立进程；agent 进程由 Daemon 管理，执行状态由 Session 模块的 session-execution 机制跟踪 |

### 共享类型

Agent 模块产出的配置数据由 Config 模块加载为 `ResolvedAgentConfig`，被 Session/Permission 等多个模块消费。共享类型定义见 [agent-config.md](agent-config.md) §配置字段。
</tool_result>
