# Agent 配置档案

## 概述

Agent 配置档案定义每个 agent 的静态属性和能力边界。每个 agent 对应一个独立的配置目录，在 session 创建时被读取并分发到各相关模块。

Agent 配置（JSON）和 Bootstrap 文件（Markdown）是两层独立的事：配置定义"能力边界"（模型、工具、权限、spawn 控制），Bootstrap 定义"身份人格"（AGENTS.md、SOUL.md 等）。配置不混入 Markdown 正文。

Agent 配置档案是纯配置值对象，不包含运行时可变字段。运行时状态（执行状态、对话状态）由 Session 模块管理。

Agent 权限规则存储在独立的 `permissions.json` 中，与 `config.json` 分离，支持独立热更新和故障隔离。

## 架构

### 配置字段（config.json）

| 字段 | 含义 | 必填 | 默认值 |
|------|------|------|--------|
| `id` | agent 唯一标识 | 是 | - |
| `name` | 显示名称 | 否 | 同 id |
| `parentId` | 父 agent ID（agent 创建时写入，运行时不变） | 否 | null |
| `model` | 默认 LLM 模型及 fallback 列表 | 否 | 系统默认模型 |
| `workspace` | 工作目录路径 | 否 | 系统默认 workspace |
| `agentDir` | bootstrap 文件所在目录 | 否 | 系统默认 agent 目录 |
| `bootstrapMode` | bootstrap 加载模式 | 否 | `"full"` |
| `skills` | 可用 skill 名称列表，`"*"` 表示全部可用 | 否 | `["*"]` |
| `tools` | 可用工具名称白名单 | 否 | `["*"]` |
| `disallowedTools` | 禁用工具黑名单 | 否 | `[]` |
| `subagents` | spawn 控制参数 | 否 | 见子字段 |
| `memory` | per-agent memory 配置覆盖（active_searcher 参数等），仅在明确需要覆盖默认值时使用，详见 [memory 模块设计文档](../memory/config.md) | 否 | null |

`bootstrapMode` 取值：
- `"full"`：加载完整 bootstrap 文件集（AGENTS.md、SOUL.md、IDENTITY.md、USER.md、TOOLS.md、BOOTSTRAP.md、MEMORY.md）
- `"minimal"`：仅加载核心文件，减少上下文占用

`subagents` 子字段：

| 字段 | 含义 | 默认值 |
|------|------|--------|
| `allowAgents` | 允许 spawn 的目标 agent ID 白名单 | `["*"]` |
| `requireAgentId` | spawn 时是否必须显式指定 agentId | `false` |
| `maxSpawnDepth` | 本 agent 允许的最大子孙层级数（1=仅子 agent，0=禁止 spawn） | `1` |
| `maxChildren` | 最大并发活跃子 session 数 | `5` |
| `defaultChildAgent` | 默认子 agent ID（spawn 不指定时使用） | 无 |
| `model` | 子 agent 的默认模型覆盖，优先级低于 sessions_spawn 调用时的显式 model 参数 | 无（用子 agent 自身配置） |

`allowAgents` 为 `["*"]` 时不限制；为空数组 `[]` 时禁止 spawn 任何子 agent。

### 权限配置（permissions.json）

权限规则独立存储于 `permissions.json`，与 `config.json` 同目录。两个文件独立读写、独立热更新：修改权限不影响 agent 配置，反之亦然。权限文件不存在时不阻塞 agent 加载，使用系统默认权限基线。

### 配置存储位置和优先级

每个 Agent 的配置存放在独立的目录中，注册由 `config/agents.json`（用户级）和 `<repo>/.closeclaw/agents.json`（项目级）两个清单文件显式控制。只有清单中列出的 Agent ID 才会被加载，不在清单中的目录即使存在也忽略。

注册清单格式（JSONC，支持 `//` 单行注释）：

```jsonc
{
  "agents": ["agent-id-1", "agent-id-2"]
}
```

清单仅包含 agent ID 列表，不包含内联配置（如模型名称、工具列表等）。每个 agent 的详细配置存储在独立目录 `agents/<id>/config.json` 中。被注释掉的 ID 视为未注册，对应的配置目录会被跳过。

采用字段级覆盖合并：高优先级配置中未指定的字段回退到低优先级配置的值。

```
项目级：<repo>/.closeclaw/agents/<id>/{config.json, permissions.json}  ← 最高优先级
用户级：~/.closeclaw/agents/<id>/{config.json, permissions.json}      ← 次优先级
```

初始 Agent（ID 为 `master`）由 CLI 配置向导在首次运行时创建。

### Agent 能力模型

Agent 能力完全由配置字段组合决定，不依赖预定义的类型枚举：

| 能力维度 | 配置字段 | 效果 |
|---------|---------|------|
| 行为限制 | `tools` / `disallowedTools` | 控制可见工具集 |
| 上下文大小 | `bootstrapMode` | `"minimal"` 减少 context 占用 |
| 繁衍能力 | `subagents.allowAgents` / `maxSpawnDepth` | 控制能否 spawn 子 agent |

### 配置示例

```jsonc
{
  "id": "code-reviewer",
  "name": "代码审查助手",
  "parentId": null,
  "model": "deepseek/deepseek-chat",
  "workspace": null,
  "agentDir": null,
  "bootstrapMode": "minimal",
  "skills": ["code-review"],
  "tools": ["read", "grep", "glob", "web_search", "web_fetch"],
  "disallowedTools": [],
  "subagents": {
    "allowAgents": [],
    "maxSpawnDepth": 0,
    "maxChildren": 0
  }
}
```

### Prompt 模板

框架提供嵌入式 prompt 模板，用于 spawn 子 agent 时自动注入任务要求：

| 模板 | 用途 | 效果 |
|------|------|------|
| `explore` | 只读研究 | 注入"只做研究不修改文件"的行为约束 |
| `validation` | 校验审计 | 注入"逐条校验并报告差异"的结构化输出要求 |

模板不影响 agent 配置，仅在 spawn 子 agent 时作为 prompt 前缀注入。

## 数据流

### 配置加载流程

```
读取注册清单
  ↓
加载用户级清单（~/.closeclaw/config/agents.json）
  ↓
加载项目级清单（<repo>/.closeclaw/agents.json，存在时）
  ↓
ID 取并集，去重（注册清单中注释掉的 ID 跳过）
  ↓
对每个注册 ID：
  优先加载项目级 agents/<id>/config.json（不存在回退用户级）
  permissions.json 同路径同优先级加载
  ↓
字段级覆盖合并（项目级字段覆盖用户级字段）
  ↓
ID 在注册表但无 config.json → WARN 跳过
目录中有 config.json 但 ID 不在注册表 → 忽略
  ↓
生成 ResolvedAgentConfig（所有字段已补齐默认值），返回给调用方（Daemon → AgentRegistry）
```

### ResolvedAgentConfig

ResolvedAgentConfig 是 Agent 配置加载流程的最终产物——经过项目级与用户级字段合并、补齐所有默认值后的完整配置。与 raw config.json 的区别：

- **config.json**：用户手写的配置文件，部分字段可留空（有默认值时可为 null）
- **ResolvedAgentConfig**：所有字段已完成解析——每个字段均已应用合并结果并按需填充默认值（null 是部分字段的合法确定值，如 parentId 为 null 表示无父 agent）

字段语义与 [§配置字段](#配置字段configjson) 完全一致，差异仅在于每个字段一定是 resolved 状态——合并完成、默认值填充完成。

ResolvedAgentConfig 由 Config 模块的配置读取组件产出，存入 Agent 模块的配置查询数据源，被 Session/Permission/System Prompt 等多个下游模块消费。

### 模型解析优先级

spawn 子 agent 时，模型的最终选择按以下顺序确定：

```
显式 model 参数 > 父 agent.subagents.model > 目标 agent.model > 系统默认
```

### 配置生效路径

配置加载完成后，Session 模块在创建 session 时从 AgentRegistry 读取配置各字段并分发到对应子系统。详见 [agent/README.md](README.md) → 模块关系 → 下游 → Session。

## 模块关系

### 上游（谁调用 Agent 配置）

| 模块 | 调用关系 |
|------|---------|
| Config | 启动时通过 AgentDirectoryProvider 加载配置文件，生成 ResolvedAgentConfig |
| CLI config wizard | 首次运行时创建初始 Agent（默认 ID `master`），写入注册清单和配置文件 |

### 下游（Agent 配置被谁消费）

| 模块 | 消费方式 |
|------|---------|
| Session | 创建 session 时读取 agent 配置，分发各字段到对应子系统 |
| Gateway/Daemon | 入站消息到达时查询 agent 配置确定目标 agent |
| AgentRegistry | 接收 Config 填充的 ResolvedAgentConfig 作为运行时查询数据源 |
| Skills Registry | 查询 agent 的 skills 白名单过滤可用 skill 列表 |
| Tools Registry | 查询 agent 的 tools/disallowedTools 过滤工具列表 |

### 无关

Agent 配置档案是纯数据定义层，不调用任何模块。配置加载后由各消费模块自行读取。
