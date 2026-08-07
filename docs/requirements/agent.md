# Agent 需求

## 概述

Agent 模块定义每个 AI Agent 的身份和能力边界，通过配置文件定义差异化的 Agent 实例，并通过 Agent 间的层级协作完成复杂任务。

## 功能需求

> 交叉引用：
>
> - Agent 的配置加载与热重载依赖 [config 需求](config.md)
> - 子 Session 委托与协调依赖 [session 需求：子 Session 委托与协调](session.md#f4-子-session-委托与协调)
> - 权限继承依赖 [permission 需求：子 Agent 权限继承](permission.md#f9-子-agent-权限继承)
>
> 各相应章节中已标注引用。

### F1. Agent 配置档案

每个 Agent 对应一份独立的配置档案，包含以下能力维度：

- **身份标识**：Agent 的唯一 ID 和显示名称
- **模型选择**：Agent 使用的默认 LLM 模型及备用模型列表
- **工作目录**：Agent 的默认工作目录
- **身份加载模式（bootstrapMode）**：完整模式或精简模式，控制上下文注入文件的数量
- **身份人格文件目录（bootstrap 文件目录）**：Bootstrap 文件所在目录
- **工具白名单/黑名单**：Agent 可以使用的工具范围
- **技能白名单**：Agent 可以使用的技能范围。技能的发现、目录结构和多 Agent 隔离详见 [skills §F1](skills.md)（技能即插即用）、[skills §F8](skills.md)（多 Agent 隔离）
- **子 Agent 控制**：Agent 创建子 Agent 的限制规则（目标白名单、层级深度、并发数等）
- **记忆配置**：Agent 的记忆模块参数（可选覆盖默认值）

Agent 的配置档案为纯静态定义，不包含运行时可变状态。Agent 的运行时行为由 Session 模块驱动。

### F2. 身份与人格分离

Agent 的能力边界（配置档案）和身份人格（Bootstrap 文件）是两层独立定义：

- **配置档案**定义 Agent 的能力边界——模型、工具、权限、spawn 控制
- **Bootstrap 文件**定义 Agent 的身份人格——操作规程、角色定义、用户偏好等

Agent 的身份人格文件通过配置指定，包括身份加载模式和身份人格文件目录。

> **交叉引用**：完整模式加载哪些文件、精简模式只加载哪些文件，详见 [system_prompt §F1](system_prompt.md)（身份与行为准则定义）。

### F3. Agent 能力组合

Agent 的能力（行为边界）由其配置字段的组合决定，不依赖预定义的类型标签：

- 工具白名单/黑名单控制 Agent 可以执行的操作范围
- 身份加载模式控制 Agent 的上下文体积
- 子 Agent 控制参数决定 Agent 的繁衍能力
- 权限配置决定 Agent 的安全边界

框架提供一组预置的行为模板（如"只读研究"、"校验审计"），在创建子 Agent 时可选择注入对应的行为约束，但不影响 Agent 本身的配置定义。

### F4. 配置层级与优先级

> **交叉引用**：Agent 配置的目录布局、注册清单加载和项目级/用户级字段合并机制详见 [config §F1](config.md)（多文件配置结构）。

### F5. 初始 Agent 创建

首次使用系统时，CLI 配置向导引导创建一个初始 Agent。初始 Agent 的 ID 默认为 `master`，具备全部工具和技能的访问权限，作为系统的基础入口。

### F6. 运行时配置查询

系统运行时，各模块通过 Agent ID 查询 Agent 的完整配置。查询是只读的，返回该 Agent 的完整能力定义（模型、工具集、技能列表、子 Agent 控制参数等）。

配置文件变更检测与重载通知由 Config 模块负责（详见 [config §F4](config.md)（配置重载））。已运行的会话对配置变更的感知时机由各消费模块自行决定。修改权限配置不影响 Agent 核心配置，反之亦然。

### F7. 子 Agent 创建（Spawn）

Agent 可以创建子 Agent 来执行子任务。默认创建的子 Agent 为一次性执行（完成后自动结束并回传结果），也可选择持久存活（等待后续控制，见「持久子 Agent 控制」）。创建时调用方指定：

- **目标 Agent**：使用哪个 Agent 配置（未指定时默认使用当前 Agent 的 ID，即 spawn 一个自己的分身）
- **任务描述**：子 Agent 要完成的任务
- **上下文模式**：子 Agent 仅接收任务描述，还是继承父 Agent 的对话历史（详见「子 Agent 上下文继承」）
- **上下文精简**：子 Agent 是否以精简模式启动
- **模型覆盖**：可选覆盖目标 Agent 的默认模型
- **行为模板**：可选注入预置的行为模板（如"只读研究"、"校验审计"）
- **工作目录覆盖**：可选为子 Agent 指定独立的工作目录
- **超时预警**（timeout_warning）：子 Agent 的预期执行时长。未指定时按以下优先级确定：spawn 显式参数 > 目标 Agent 配置 > 全局默认值
- **硬超时**（timeout）：子 Agent 的绝对最大执行时长。未指定时按以下优先级确定：spawn 显式参数 > 目标 Agent 配置 > 全局默认值（默认 48 小时）

> **交叉引用**：超时预警和硬超时的执行行为（预警通知、终止与级联终止）详见 [session §F4](session.md)（子 Session 委托与协调）。

子 Agent 的最终模型按以下优先级确定：显式指定的模型 > 父 Agent 配置中的子 Agent 默认模型 > 目标 Agent 配置的模型 > 系统默认模型。

### F8. 子 Agent 上下文继承（Fork）

Fork 是子 Agent 创建中「上下文模式」的具体实现：子 Agent 在创建时继承父 Agent 的完整对话历史，使子 Agent 理解已发生的上下文后再执行新任务。普通 Spawn 的子 Agent 只看到任务描述，Fork 模式的子 Agent 先看到父 Agent 的对话历史，再看到任务描述。

### F9. Spawn 创建控制

父 Agent 的配置控制子 Agent 的创建行为：

- **目标白名单**：限制可以 spawn 的目标 Agent 范围（通配符 `*` 表示不限制，空列表表示禁止 spawn）
- **层级深度**：限制最多可以嵌套多少层子 Agent（0 表示禁止 spawn 任何子 Agent）
- **并发数量**：限制同时存活的子 Agent 数量上限
- **必选 Agent ID**：目标 Agent 必须可解析（显式指定或使用 F7 默认值——当前 Agent 的 ID）
- **子 Agent 默认模型**：子 Agent 的默认模型覆盖（优先级低于 spawn 时显式指定的模型）

层级深度受父 Agent 配置和目标 Agent 配置的双重约束：取两者中更严格的值生效。即使父 Agent 允许更多层级，目标 Agent 可以主动收窄自己的子树深度。

### F10. 子 Session 结果回传

> **交叉引用**：一次性执行的子 Session 完成后，执行结果自动回传给父 Session，带去重保护。详见 [session §F4](session.md)（子 Session 委托与协调）。

### F11. 持久子 Session 控制

持久存活的子 Session 允许父 Session 在运行期间下发新任务或终止子 Session 树。

> **交叉引用**：steer/kill 的完整操作语义、级联清理、生命周期联动详见 [session §F4](session.md)（子 Session 委托与协调）。系统重启恢复时的降级处理详见 [session §F1](session.md)（对话持久化与恢复）。

### F12. Agent 权限继承

子 Agent 的权限沿创建链路收窄，不超出任何父 Agent 的权限范围。子 Agent 被拒绝时不进入用户审批流程（子 Agent 不是面向用户的入口）。

> **交叉引用**：权限继承的交集计算规则、Deny 沿链路传播、最高权限身份的 User 维度豁免、权限评估的实时性详见 [permission §F9](permission.md)（子 Agent 权限继承）。

### F13. 工作目录权限

每个 Agent 在 spawn 时获得其专属工作目录的读写权限。

> **交叉引用**：工作目录的解析顺序由 Session 模块确定（详见 [session §F8](session.md)（工作目录））。Workspace 路径强制授权机制详见 [permission §F3](permission.md)（权限决策模型）。父 Agent 可通过显式的拒绝规则覆盖工作目录权限。

### F14. Agent 间通信

子 Agent 创建时，系统自动配置父子之间的消息路由。父子之间可以双向收发消息。Agent 间的消息送达需要同时满足路由配置和权限允许两个条件。

通信路由在 spawn 时自动生成，默认仅父子之间互通。如需扩展到其他 Agent，需要额外配置。

### F15. Spawn 层级追踪

系统维护 Agent 之间的创建层级关系，记录每个会话对应的目标 Agent ID。

> **交叉引用**：父子 session 关系维护、查询接口（子会话/子树/父会话）、级联清理与重启降级恢复由 Session 模块负责（详见 [session §F1](session.md)（对话持久化与恢复）、[session §F4](session.md)（子 Session 委托与协调））。

## 关联设计文档

- [✓] agent/README.md
- [✓] agent/agent-config.md
- [✓] agent/agent-permissions.md
- [✓] agent/agent-registry.md
- [✓] agent/agent-spawn.md

## 非功能需求

### 性能

- Agent 配置查询延迟不影响会话创建
- 权限评估在每次操作前重新判定，配置变更即时生效
- 子 Agent 结果自动回传，父 Agent 不阻塞等待（详见 [session §F4](session.md)）

### 安全性

- 子 Agent 的权限沿创建链路只收窄不放宽
- 子 Agent 被拒绝时不进入用户审批流程，避免非用户入口的权限提升
- 权限配置和 Agent 配置互不干扰——修改任一配置不影响另一方，热更新互不触发

### 可用性

- Agent 配置文件缺失时，系统启动时跳过缺失的 Agent，记录警告日志
- 权限文件缺失时不阻塞 Agent 加载，使用系统默认权限
- 系统重启后自动恢复 Agent 层级关系，重启前已丢失父会话的子会话降级为独立会话继续服务

### 可扩展性

- Agent 的能力通过配置字段自由组合，不受限于预定义的类型
- 新增 Agent 能力维度时无需系统升级，新增维度基于文件存放标准路径即可生效
- 项目级和用户级两层配置支持团队协作和个人定制的场景
