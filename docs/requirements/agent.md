# Agent 需求

## 概述

Agent 模块定义每个 AI Agent 的身份和能力边界，通过配置档案定义差异化的 Agent，并通过 Agent 间的层级协作完成复杂任务。

Agent 是静态的配置身份；Session 是 Agent 的运行时实例，由 Session 模块定义。本文档中的 spawn 指以目标 Agent 的身份创建子 Session——创建控制由本文档定义，创建后的运行时行为由 Session 模块定义（详见 [session](session.md)）。

## 功能需求

### F1. Agent 配置档案

每个 Agent 对应一份独立的配置档案，包含以下能力维度：

- **身份标识**：Agent 的唯一 ID 和显示名称
- **模型选择**：Agent 使用的默认 LLM 模型及备用模型列表
- **工作目录**：Agent 的默认工作目录
- **身份加载模式（Bootstrap 模式）**：完整模式或精简模式，控制上下文注入文件的数量
- **Bootstrap 文件目录**：Bootstrap 文件所在目录
- **工具白名单/黑名单**：Agent 可以使用的工具范围
- **技能白名单**：Agent 可以使用的技能范围。技能的发现、目录结构和多 Agent 隔离详见 [skills §F1](skills.md)（技能即插即用）、[skills §F8](skills.md)（多 Agent 隔离）
- **子 Agent 控制**：Agent 创建子 Agent 的限制规则（目标白名单、层级深度、并发数等）
- **记忆配置**：Agent 的记忆模块参数（可选覆盖默认值）

Agent 的配置档案为纯静态定义，不包含运行时可变状态。Agent 的运行时行为由 Session 模块驱动。

> **交叉引用**：配置档案的目录布局与注册清单详见 [config §F1](config.md)（多文件配置结构）。

### F2. 身份与人格分离

Agent 的能力边界（配置档案）和身份人格（Bootstrap 文件）是两层独立的概念：

- **配置档案**定义 Agent 的能力边界——模型、工具、权限、spawn 控制
- **Bootstrap 文件**定义 Agent 的身份人格——操作规程、角色定义、用户偏好等

Agent 的身份人格文件通过配置指定，包括身份加载模式和身份人格文件目录。

> **交叉引用**：完整模式加载哪些文件、精简模式只加载哪些文件，详见 [system_prompt §F1](system_prompt.md)（身份与行为准则定义）。

### F3. Agent 能力组合

Agent 的能力（行为边界）由其配置字段的组合决定，不依赖预定义的类型标签：

- 工具白名单/黑名单控制 Agent 可以执行的操作范围
- Bootstrap 模式控制 Agent 的上下文规模
- 子 Agent 控制参数决定 Agent 的派生能力
- 权限基线（独立于 Agent 配置存储）定义 Agent 的安全边界

框架提供一组预置的行为模板（如"只读研究"、"校验审计"），创建子 Session 时可通过 spawn 参数选择注入对应的行为约束；行为模板是创建时注入的约束，不改变 Agent 的静态配置定义。

### F4. 配置层级与优先级

> **交叉引用**：Agent 配置的目录布局、注册清单加载和项目级/用户级字段合并机制详见 [config §F1](config.md)（多文件配置结构）。

### F5. 初始 Agent 创建

首次使用系统时，CLI 配置向导引导创建一个初始 Agent。初始 Agent 的 ID 默认为 `master`，具备全部工具和技能的访问权限，作为系统的基础入口。

### F6. 运行时配置查询

系统运行时，各模块通过 Agent ID 查询 Agent 的完整配置档案。查询为只读操作，返回该 Agent 在 F1 定义的全部能力维度——身份标识、模型选择、工作目录、身份加载模式、Bootstrap 文件目录、工具白名单/黑名单、技能白名单、子 Agent 控制参数、记忆配置。查询返回 Agent 的静态配置档案，不包含运行时派生的能力——如权限过滤后的实际工具清单、运行模式决定的工具范围（运行模式定义见 [mode §F1](mode.md)）。

配置变更的检测与重载通知由 Config 模块负责，详见 [config §F4](config.md)（配置重载）。注册清单与 Agent 配置变更后，新创建的 Session 使用最新配置，已运行的 Session 沿用创建时的配置。

### F7. 子 Agent 创建（Spawn）

Agent 可以创建子 Session 来执行子任务。默认创建的子 Session 为一次性执行（完成后自动结束并回传结果），也可选择持久存活（见 F11「持久子 Session 控制」）。创建时调用方指定：

- **目标 Agent**：spawn 的目标 Agent，使用其配置档案创建子 Session（未指定时默认使用当前 Agent 的 ID，即 spawn 一个自己的分身）
- **任务描述**：子 Session 要完成的任务
- **上下文模式**：子 Session 仅接收任务描述，还是继承父 Agent 的对话历史（详见「子 Agent 上下文继承（Fork）」）
- **上下文精简**：子 Session 是否以 Bootstrap 精简模式启动
- **模型覆盖**：可选覆盖目标 Agent 的默认模型
- **行为模板**：可选注入预置的行为模板（如"只读研究"、"校验审计"）
- **工作目录覆盖**：可选为子 Session 指定独立的工作目录
- **超时预警**：子 Session 的预期执行时长。未指定时按以下优先级确定：spawn 显式参数 > 目标 Agent 配置 > 全局默认值
- **硬超时**：子 Session 的绝对最大执行时长。未指定时按以下优先级确定：spawn 显式参数 > 目标 Agent 配置 > 全局默认值（默认 48 小时）

> **交叉引用**：超时预警和硬超时的执行行为（预警通知、终止与级联终止）详见 [session §F4](session.md)（子 Session 委托与协调）。

子 Session 的最终模型按以下优先级确定：显式指定的模型 > 父 Agent 配置中的子 Session 默认模型 > 目标 Agent 配置的模型 > 系统默认模型。

### F8. 子 Agent 上下文继承（Fork）

Fork 模式对应 F7「上下文模式」的继承对话历史分支：子 Session 在创建时继承父 Agent 的完整对话历史，使子 Session 理解已发生的上下文后再执行新任务；普通 Spawn 不装载父 Session 的对话历史，Fork 模式将父 Agent 的完整对话历史装载到子 Session 的对话消息区。

> **交叉引用**：任务描述的注入方式（注入 system prompt、不属于对话消息、压缩时不受影响）详见 [session §F4](session.md)（子 Session 委托与协调）。

### F9. Spawn 创建控制

父 Agent 的配置控制子 Agent 的创建行为：

- **目标白名单**：限制可以 spawn 的目标 Agent 范围（通配符 `*` 表示不限制，空列表表示禁止 spawn）
- **层级深度**：限制嵌套的最大层数（0 表示禁止 spawn 任何子 Agent）
- **并发数量**：限制同时存活的子 Session 数量上限
- **目标 Agent 必须已注册**：spawn 的目标 Agent（显式指定，或按 F7 默认值取当前 Agent）必须是注册清单中已注册、配置可加载的 Agent
- **子 Session 默认模型**：子 Session 的默认模型覆盖（优先级低于 spawn 时显式指定的模型）

层级深度受父 Agent 配置和目标 Agent 配置的双重约束：取两者中更严格的值生效。即使父 Agent 允许更多层级，目标 Agent 的配置也可以约束其作为父 Agent 时的子树深度。

### F10. 子 Session 结果回传

> **交叉引用**：一次性执行的子 Session 完成后，执行结果自动回传给父 Session，带去重保护。详见 [session §F4](session.md)（子 Session 委托与协调）。

### F11. 持久子 Session 控制

> **交叉引用**：持久子 Session 的 steer/kill 操作语义、级联清理、生命周期联动详见 [session §F4](session.md)（子 Session 委托与协调）。系统重启恢复时的降级处理详见 [session §F1](session.md)（对话持久化与恢复）。

### F12. 子 Agent 权限继承

> **交叉引用**：子 Agent 权限沿创建链路收窄、Deny 沿链路传播、被拒绝时静默返回。详见 [permission §F9](permission.md)（子 Agent 权限继承）。

### F13. 工作目录权限

> **交叉引用**：工作目录的强制授权机制（不受任何 Deny 规则影响）详见 [permission §F3](permission.md)（权限决策模型）。
> **交叉引用**：工作目录的解析顺序由 Session 模块确定，详见 [session §F8](session.md)（工作目录）。

### F14. 父子 Session 通信

子 Session 创建时，系统自动配置父子 Session 间的通信路由。父子 Session 之间通过 spawn 时自动生成的路由互通消息。消息送达需同时满足路由配置和权限允许两个条件。

默认仅父子之间互通。如需扩展到其他 Agent，需要额外配置。

### F15. Spawn 层级追踪

系统维护 Agent 之间的创建层级关系，记录每个 Session 对应的目标 Agent ID。

> **交叉引用**：父子 Session 关系维护、查询接口（子会话/子树/父会话）、级联清理与重启降级恢复由 Session 模块负责。详见 [session §F1](session.md)（对话持久化与恢复）、[session §F4](session.md)（子 Session 委托与协调）。

## 关联设计文档

- agent/README.md
- agent/agent-config.md
- agent/agent-permissions.md
- agent/agent-registry.md
- agent/agent-spawn.md

## 非功能需求

### 性能

- Agent 配置查询延迟不导致 Session 创建出现可感知等待
- 权限与 Agent 配置变更的生效机制详见 [config §F7](config.md)（生效机制与重启类判定）
- 子 Session 结果自动回传，父 Agent 不阻塞等待（详见 [session §F4](session.md)）

### 安全性

- 子 Agent 权限的沿链路收窄与拒绝行为详见 [permission §F9](permission.md)（子 Agent 权限继承）

### 可扩展性

- Agent 的能力通过配置字段自由组合，不受限于预定义的类型
- 新增 Agent 能力维度时无需系统升级即可生效
- 项目级与用户级两层配置的合并机制详见 [config §F1](config.md)（多文件配置结构）
