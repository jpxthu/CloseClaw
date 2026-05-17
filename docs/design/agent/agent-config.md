# Agent 配置档案

## 概述

Agent 配置档案定义每个 agent 的静态属性和能力边界。每个 agent 对应一份 JSON 配置文件，在 session 创建时被读取并分发到各相关模块。

Agent 配置（JSON）和 Bootstrap 文件（Markdown）是两层独立的事：配置定义"能力边界"（模型、工具、权限、spawn 控制），Bootstrap 定义"身份人格"（AGENTS.md、SOUL.md 等）。配置不混入 Markdown 正文。

## 架构

### 配置字段

| 字段 | 含义 | 必填 | 默认值 |
|------|------|------|--------|
| `id` | agent 唯一标识 | 是 | - |
| `name` | 显示名称 | 否 | 同 id |
| `model` | 默认 LLM 模型及 fallback 列表 | 否 | 系统默认模型 |
| `workspace` | 工作目录路径 | 否 | 系统默认 workspace |
| `agentDir` | bootstrap 文件所在目录 | 否 | 系统默认 agent 目录 |
| `bootstrapMode` | bootstrap 加载模式 | 否 | `"full"` |
| `skills` | 可用 skill 名称列表，`"*"` 表示全部可用 | 否 | `["*"]` |
| `tools` | 可用工具名称白名单 | 否 | `["*"]` |
| `disallowedTools` | 禁用工具黑名单 | 否 | `[]` |
| `permissions` | Agent 维度权限基线规则 | 否 | 默认仅消息 |
| `subagents` | spawn 控制参数 | 否 | 见子字段 |

`bootstrapMode` 取值：
- `"full"`：加载完整 bootstrap 文件集（AGENTS.md、SOUL.md、IDENTITY.md、USER.md、TOOLS.md、MEMORY.md 等）
- `"minimal"`：仅加载核心文件，减少上下文占用

`subagents` 子字段：

| 字段 | 含义 | 默认值 |
|------|------|--------|
| `allowAgents` | 允许 spawn 的目标 agent ID 白名单 | `["*"]` |
| `requireAgentId` | spawn 时是否必须显式指定 agentId | `false` |
| `maxSpawnDepth` | 最大 spawn 嵌套深度 | `1` |
| `maxChildren` | 最大并发活跃子 session 数 | `5` |
| `defaultChildAgent` | 默认子 agent ID（spawn 不指定时使用） | 无 |
| `model` | 子 agent 的默认模型覆盖 | 无（用子 agent 自身配置） |

`allowAgents` 为 `["*"]` 时不限制；为空数组 `[]` 时禁止 spawn 任何子 agent。

### 配置存储位置和优先级

采用字段级覆盖合并：高优先级配置中未指定的字段回退到低优先级配置的值。

```
项目级：<repo>/.closeclaw/agents/<id>.json   ← 最高优先级
用户级：~/.closeclaw/agents/<id>.json        ← 次优先级
系统内置：框架预定义的 agent 定义             ← 最低优先级
```

### Agent 能力模型

Agent 能力完全由配置字段组合决定，不依赖预定义的类型枚举：

| 能力维度 | 配置字段 | 效果 |
|---------|---------|------|
| 行为限制 | `tools` / `disallowedTools` | 控制可见工具集 |
| 上下文大小 | `bootstrapMode` | `"minimal"` 减少 context 占用 |
| 权限边界 | `permissions` | 控制文件/网络/命令等权限 |
| 繁衍能力 | `subagents.allowAgents` / `maxSpawnDepth` | 控制能否 spawn 子 agent |

### 配置示例

```json
{
  "id": "code-reviewer",
  "name": "代码审查助手",
  "model": "deepseek/deepseek-chat",
  "workspace": null,
  "agentDir": null,
  "bootstrapMode": "minimal",
  "skills": ["code-review"],
  "tools": ["read", "grep", "glob", "web_search", "web_fetch"],
  "disallowedTools": [],
  "permissions": {
    "file_read": { "allowed": true },
    "file_write": { "allowed": false },
    "exec": { "allowed": false },
    "network": { "allowed": false }
  },
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

模板不影响 agent 配置，仅在 sessions_spawn 调用时作为 prompt 前缀注入。

## 数据流

### 配置加载流程

```
系统启动
  ↓
加载内置 agent 定义（仅 general-purpose）
  ↓
扫描用户级 agent 配置（~/.closeclaw/agents/*.json）
  ↓
扫描项目级 agent 配置（<cwd>/.closeclaw/agents/*.json）
  ↓
按优先级合并同 ID 的 agent 配置（字段级覆盖）
  ↓
生成 ResolvedAgentConfig（所有字段已补齐默认值）
  ↓
注册到内存配置注册表
```

### 模型解析优先级

spawn 子 agent 时，模型的最终选择按以下顺序确定：

```
显式 model 参数 > 父 agent.subagents.model > 目标 agent.model > 系统默认
```

### 配置生效路径

```
ResolvedAgentConfig 各字段分发到对应模块
  ↓
  model        → Session：设置默认 LLM 模型
  bootstrapMode → System Prompt：决定 bootstrap 文件加载集
  agentDir     → System Prompt：bootstrap 文件读取路径
  permissions  → Permission：Agent 维度权限基线
  skills       → Skill Registry：过滤可见 skill 列表
  tools        → Tool Registry：过滤可见工具白名单
  disallowedTools → Tool Registry：排除禁用工具
  subagents    → Agent：注入 session 的 spawn 控制上下文
```

## 模块关系

### 上游（谁调用 Agent 配置）

| 模块 | 调用关系 |
|------|---------|
| 初始化阶段 | 由 Agent 模块在启动时加载配置文件，生成 ResolvedAgentConfig |
| Session | 创建 session 时读取 agent 配置，分发各字段 |
| Gateway/Daemon | 外部消息到达时确定目标 agent ID |

### 下游（Agent 配置被谁消费）

| 模块 | 消费方式 |
|------|---------|
| Session | 读取 model、workspace |
| System Prompt | 读取 bootstrapMode、agentDir |
| Permission | 读取 permissions 基线规则 |
| Skill Registry | 读取 skills 白名单 |
| Tool Registry | 读取 tools 白名单、disallowedTools 黑名单 |
| Agent Spawn | 读取 subagents 控制参数 |

### 无关

Agent 配置档案是纯数据定义层，不调用任何模块。配置加载后由各消费模块自行读取。
