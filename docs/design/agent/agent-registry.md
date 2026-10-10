# AgentRegistry

## 概述

AgentRegistry 是 Agent 模块的运行时配置查询入口，以 `agent_id` 为键提供 [ResolvedAgentConfig](../common/shared-types.md#resolvedagentconfig--subagentsconfig--memoryconfig) 的只读查找。启动时由 Daemon 从 Config 加载结果一次性填充，运行时只读查询。查询返回的是 Agent 的静态配置档案（不含运行时派生能力，见 [agent §F6](../../requirements/agent.md)）——`tools` / `skills` 为静态字面值，权限过滤后的工具清单、运行模式决定的工具范围等派生视图由下游消费模块计算。

## 架构

AgentRegistry 是轻量级内存查找表，不管理 agent 生命周期、不做权限校验、不做配置合并——这些职责分别属于 Session、Permission、Config 模块。

数据流：Config 加载所有 agent 配置 → AgentRegistry 接收填充 → 各消费模块按 agent_id 查询只读配置。

接口职责：

- 启动时由 Daemon 从 Config 加载结果一次性填充注册表
- 运行时各消费模块通过 agent_id 查询获取只读配置，命中的返回完整配置，未命中由调用方自行处理
- 提供遍历所有已注册 agent 配置的能力，用于启动时全量初始化等批量操作场景（消费方见下游表）。AgentRegistry 通过 common 核心 trait 暴露按 agent 的查询接口（接口契约定义见 [common 模块](../common/core-traits.md)）：合并查询接口 [AgentRegistryQuery](../common/core-traits.md#agentregistryquery)（自身不新增方法，合并 [AgentLookup](../common/core-traits.md#agentlookup) / [AgentSkillsQuery](../common/core-traits.md#agentskillsquery) / [AgentToolsConfigQuery](../common/core-traits.md#agenttoolsconfigquery)），另实现最小配置查询接口 [AgentConfigLookup](../common/core-traits.md#agentconfiglookup)。skills/tools 白名单为 `["*"]` 或空时返回不限制，黑名单为空时同样不限制。当白名单和黑名单同时非空且有交集时，黑名单优先——黑名单中列出的工具即使在白名单中也不可用（显式拒绝优先于显式允许）。返回的配置为静态视图——`tools` / `skills` 保留静态字面值（`["*"]` 即字面 `["*"]`），权限过滤、运行模式适配等派生工具/技能集由消费方计算。

**热重载策略**：Config Hot Reload 检测到 agent 配置变更 → 重新加载 → 通知 Daemon → Daemon 触发全量替换。已运行的 session 是否感知变更由各消费模块自行决定——AgentRegistry 只负责提供最新数据，不推送变更通知。

## 数据流

### 启动填充

1. Daemon 启动
2. Config 按注册清单加载所有已注册 Agent 的 config.json，补齐默认值生成 [ResolvedAgentConfig](../common/shared-types.md#resolvedagentconfig--subagentsconfig--memoryconfig) 列表
3. AgentRegistry 接收填充
4. 注册表就绪，各消费模块可查询

### 运行时查询

1. 消费模块发起 agent 配置查询
2. AgentRegistry 按 agent_id 查找：
   - 命中 → 返回只读配置
   - 未命中 → 由调用方自行处理（通常是配置缺失错误）

### 热重载

1. Config Hot Reload 检测到 agent 配置变更
2. Config 按最新注册清单重新加载（补齐默认值），生成新的配置列表
3. Daemon 触发 AgentRegistry 全量替换
4. 注册表内容替换，消费模块下次查询获取新数据

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Daemon | 启动时填充注册表；热重载时触发全量替换 |
| Config Hot Reload | 检测到 agent 配置变更后通知 Daemon 触发更新 |
| Config | 提供 agent 配置数据源（Config 只负责文件加载与默认值补齐，不参与注册） |

### 下游

| 模块 | 消费方式 |
|------|---------|
| Session | 创建 session 时查询 agent 配置（模型、workspace、工具集、skill 列表等） |
| System Prompt | 经 [AgentLookup](../common/core-traits.md#agentlookup) 查询 bootstrap 模式配置（agent_id 由 SessionManager 在 Session 创建时传入，结果缓存于 ConversationSession） |
| Skills Registry | 通过 [AgentSkillsQuery](../common/core-traits.md#agentskillsquery) 接口查询 agent 的 skills 白名单，白名单为 `["*"]` 或空时返回「不限制」 |
| Tools Registry | 通过 [AgentToolsConfigQuery](../common/core-traits.md#agenttoolsconfigquery) 接口查询 agent 的 tools 白名单和 disallowedTools 黑名单，白名单为 `["*"]` 或空时返回「不限制」，黑名单为空时同样不限制 |
| Gateway / Daemon | 经 [AgentRegistryQuery](../common/core-traits.md#agentregistryquery) 查询 agent 配置与模型规格（Daemon 另消费 [AgentConfigLookup](../common/core-traits.md#agentconfiglookup)） |

### 无关

| 模块 | 说明 |
|------|------|
| Gateway | AgentRegistry 不参与消息路由 |
| IM Adapter | AgentRegistry 不涉及平台通信 |
| Processor Chain | AgentRegistry 不参与消息处理 |
| Spawn 树形拓扑 | spawn 树的父子关系、级联 kill、重启恢复等运行时拓扑由 Session 模块的 spawn_tree 子组件管理，AgentRegistry 不持有运行时状态、不参与树形拓扑查询。AgentRegistry 仅负责根据 agent_id 查询静态配置（这个 agent 能干什么） |

### 共享类型

AgentRegistry 以 agent_id 为键存储 agent 完整配置（[ResolvedAgentConfig](../common/shared-types.md#resolvedagentconfig--subagentsconfig--memoryconfig)），供 Session/System Prompt/Skills Registry/Tools Registry 查询。共享类型定义见 [common/shared-types](../common/shared-types.md)。
