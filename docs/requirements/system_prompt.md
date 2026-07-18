# System Prompt 需求

## 概述

System Prompt 是每次与 AI 模型通信时发送的引导前缀，承载 Agent 的身份与行为准则、工具清单、长期记忆、运行时上下文以及 Owner 动态追加的指令。Owner 需要：通过 workspace 配置文件定义 Agent 的行为准则，让 Agent 了解自己有哪些工具和技能，在不同类型的会话中加载恰当的上下文，并能随时追加临时指令，同时确保静态内容高效缓存、文件变更时自动反映最新内容。

## 功能需求

### F1. 身份与行为准则定义

Owner 在 workspace 目录下通过一系列配置文件（统称 bootstrap 文件）定义 Agent 的身份、操作规范、工具使用指南和 Owner 偏好。其中身份由角色定义和身份标识共同构成，Owner 偏好对应 Owner 信息文件。这些定义在 Agent 每次会话启动时自动加载，作为 System Prompt 的核心组成部分。

- 必须加载的文件：操作规程、角色定义、身份标识、Owner 信息、工具使用指南
- 可选加载的文件：自定义引导指令、长期记忆（取决于会话类型和加载模式）
- 文件不存在时静默跳过，不报错
- 多文件按固定顺序注入，操作规程排在最高优先级
- 边界说明：心跳工作流配置也位于 workspace 目录下，但**不属于 System Prompt 注入范围**——心跳由独立的定时任务按需触发读取，不进入日常会话的 System Prompt

> **交叉引用**：bootstrap 文件的加载模式（Full/Minimal）和所在目录路径，由 [agent §F1](agent.md)（配置档案）、[agent §F2](agent.md)（身份与人格分离）定义。会话创建/恢复/上下文压缩完成时的重建触发，见 [session §F2](session.md)（Agent 角色与能力配置）。压缩行为本身见 [session §F3](session.md)（长对话压缩）。

### F2. 工具清单注入

Agent 需要在 System Prompt 中看到当前可用的工具清单，以便在对话中正确选择和使用。本节工具清单与 F1 中列出的工具使用指南（bootstrap 文件之一）相互独立——前者是各工具的具体功能说明（由系统生成），后者是 Owner 对工具使用规范的高层约束（由 Owner 编写）。

- **工具清单**：工具清单的字段内容、常用/延迟分组、危险度标记、长度截断与探索提示由 [tools §F1](tools.md) 定义。本模块消费 tools 模块渲染好的工具清单结果
- 当工具定义发生变更时，清单自动更新
- 技能清单不进入 System Prompt 静态层——由 session 模块作为 per-turn attachment 在每个 turn 注入 instruction block，详见 [skills §F4](skills.md)（技能清单）。技能正文内容在模型决定使用时按需注入，不预先进入 System Prompt

> **交叉引用**：当前 Agent 可用的工具范围（白名单/黑名单），由 [agent §F3](agent.md)（Agent 能力组合）定义。本模块负责将可用工具清单渲染为 System Prompt 中的分组描述文本。

### F3. 长期记忆注入

Agent 应能获取跨会话保留的长期记忆内容。长期记忆在主 Agent 会话启动时加载，作为 System Prompt 的组成部分；子 Agent 会话不加载（见 F8 与安全性小节）。当记忆文件更新时，内容自动刷新。

> **交叉引用**：记忆的存储路径和写入机制，见 memory 模块（需求文档待创建）。记忆内容的搜索策略由 memory 模块定义。

### F4. 运行时上下文注入

每次 API 调用时，Agent 需要知道当前的运行时上下文：

- **频道上下文**：当前会话所在的频道名称
- **工作目录**：当前会话的工作目录路径

这些上下文信息每次请求即时获取，不持久化存储。

> **交叉引用**：工作目录的解析顺序（spawn 参数 > Agent 配置 > 父 Agent 继承），见 [agent §F13](agent.md)（工作目录权限）。

### F5. 动态指令管理

Owner 可以在对话中通过指令管理 System Prompt 末尾的动态指令。动态指令的追加、查看、清除由 slash 模块通过 `/system` 指令提供入口；本模块定义与 System Prompt 内容状态相关的专属行为（如清除时的缓存失效）。

- 清除动态指令时，触发全部缓存失效并重建，确保下次 System Prompt 为干净状态

> **交叉引用**：动态指令的追加、查看、清除命令入口，见 [slash §F6](slash.md)（`/system` 指令）。持久化由 [session §F2](session.md)（Agent 角色与能力配置）管理。本节仅定义 System Prompt 内容层的专属行为。

### F6. 内容缓存与自动刷新

System Prompt 中在会话生命周期内不变的内容应走缓存机制，避免每次请求都重复读取文件和生成描述文本：

- 文件内容未变更时复用已加载的版本，不触发重复读取
- 以下事件触发全部缓存失效并重建：
  - Owner 执行清空会话指令
  - Owner 清除动态指令
  - 从归档恢复会话
  - 上下文压缩完成后
- 单个文件变更时仅失效对应部分的缓存，不影响其他部分
- 工具定义变更时，变更检测自动触发对应缓存失效，下次请求时重建

> **交叉引用**：重建触发的外部事件来源——会话恢复见 [session §F1](session.md)，重建触发见 [session §F2](session.md)（Agent 角色与能力配置），上下文压缩行为见 [session §F3](session.md)。`Owner 清除动态指令` 的事件来源见 [slash §F6](slash.md)（`/system clear`）。Agent 配置文件变更的检测机制由 [agent §F6](agent.md)（运行时配置查询）管理，本模块作为消费方响应。

### F7. API 前缀缓存利用

System Prompt 中不变的前缀部分应利用 AI 服务商的前缀缓存机制，减少重复内容的 Token 计费：

- System Prompt 中不变部分和变化部分之间有明确的分隔，使缓存层能识别可缓存的前缀范围
- 不变的前缀部分应保证字节级稳定，以持续命中服务端缓存
- 每次请求变化的部分（动态上下文、动态指令）不参与前缀缓存

> **交叉引用**：各服务商的具体缓存参数适配和 Token 统计，见 [llm §F8](llm.md)（缓存成本优化）、[llm §F9](llm.md)（用量统计）。本模块仅负责不变/变化内容的划分和前缀稳定性保证。

### F8. 会话类型适配

不同类型的会话加载不同的 System Prompt 内容：

- **主 Agent 会话**：加载全部内容（F1 必须加载的文件 + 可选加载的文件 + 工具清单）
- **子 Agent 会话**：仅加载 F1 必须加载的文件 + 工具清单，不加载 F1 中列出的可选加载文件。其中长期记忆和自定义引导指令的排除理由见安全性小节
- **无 workspace 的会话**：仅加载工具清单，跳过所有 bootstrap 文件

> **注**：技能清单由 session 模块在每个 turn 统一注入为 per-turn attachment，不区分会话类型。

> **注**：F4 运行时上下文（频道、工作目录）对所有会话类型均加载。

> **交叉引用**：三种会话类型由 session 创建流程综合判定，本模块负责按类型加载对应内容。子 Agent 的 spawn 参数（如是否精简模式），见 [agent §F7](agent.md)（子 Agent 创建）。

### F9. 动态指令持久化

Owner 追加的动态指令通过 session 检查点持久化，确保跨会话保留。持久化行为和恢复语义由 session 模块定义。

> **交叉引用**：追加区的持久化、压缩豁免和恢复行为，详见 [session §F2](session.md)（Agent 角色与能力配置）。

## 关联设计文档

- [✓] system_prompt/README.md
- [✓] system_prompt/static-layer.md
- [✓] system_prompt/dynamic-layer.md
- [✓] system_prompt/fragment-provider.md
- [✓] system_prompt/kv-cache.md
- [✓] system_prompt/appends.md

## 非功能需求

### 性能

- 不变内容的加载应在会话生命周期内缓存，文件未变更时不触发重复读取
- 各部分缓存独立失效，单一部分变更不影响其他部分的重建
- System Prompt 总体积应精简——超出上限时智能截断（如工具列表超长时截断并提示探索方式），不盲目扩大

### 可用性

- 单个部分构建失败时跳过该部分，其余部分继续，不阻断整个 System Prompt 的生成
- 所有部分构建结果均为空的极端情况下，使用默认的最简 System Prompt，确保对话不中断
- 文件变更后缓存自动失效，Owner 无需手动触发生效

### 安全性

- 子 Agent 会话不暴露长期记忆内容和自定义引导指令
- 动态指令通过会话检查点持久化，不暴露给非 Owner 用户

### 可观测性

- 文件变更后，Owner 能通过 /status 命令确认 System Prompt 已反映最新内容
