# System Prompt 需求

## 概述

System Prompt 是每次与 AI 模型通信时发送的引导前缀，承载 Agent 的身份与行为准则、工具清单、技能清单、长期记忆、运行时上下文以及 Owner 动态追加的指令。System Prompt 在固定事件触发时组装，组装结果在事件之间不变，确保前缀缓存稳定命中。Owner 通过 Bootstrap 文件定义 Agent 的行为准则，让 Agent 了解自己有哪些工具和技能，在不同类型的 Session 中加载恰当的上下文，并能随时追加临时指令。

## 功能需求

### F1. 身份与行为准则定义

Owner 在 Agent 的 Bootstrap 文件目录下通过一系列配置文件（统称 Bootstrap 文件）定义 Agent 的身份、操作规程、工具使用指南和 Owner 偏好。其中身份由角色定义和身份标识构成，Owner 偏好对应 Owner 信息文件。Bootstrap 文件属于 Agent 自身配置，同一 Agent 对全部 User 表现一致的人格。这些 Bootstrap 文件在 Agent 每次 Session 启动时自动加载，作为 System Prompt 的核心组成部分。

- 必须加载的文件：操作规程、角色定义、身份标识、Owner 信息、工具使用指南
- 可选加载的内容：自定义引导指令、长期记忆。自定义引导指令是 Bootstrap 文件；长期记忆由 memory 模块产出的规则文件承载（非 Bootstrap 文件，见 memory §F5），文件缺失时同样静默跳过。是否加载取决于 Session 类型和身份加载模式：自定义引导指令仅在主 Agent Session 且完整模式下加载；长期记忆在所有主 Agent Session 加载，与身份加载模式无关。子 Session 不加载任何可选内容（见 F8）
- 文件不存在时静默跳过，不报错
- 多文件按固定顺序注入，操作规程排在最前
- System Prompt 各组成部分按固定顺序组装。配置不变时多次组装结果逐字节相同，最大限度利用前缀缓存。具体组装顺序和格式由设计文档定义
- 边界说明：心跳配置也位于 Bootstrap 文件目录下，但**不属于 System Prompt 注入范围**——心跳由独立的周期性机制定时触发，触发时按需读取配置，不进入常规对话 Session 的 System Prompt

> **交叉引用**：Bootstrap 文件的身份加载模式（完整/精简）和所在目录路径，由 [agent §F1](agent.md)（Agent 配置档案）、[agent §F2](agent.md)（身份与人格分离）定义。
> **交叉引用**：Session 创建/恢复/上下文压缩完成时的重建触发。详见 [F6](#f6-内容缓存与自动刷新)（内容缓存与自动刷新）。
> **交叉引用**：压缩行为本身详见 [session §F3](session.md)（长对话压缩）。

### F2. 工具清单注入

Agent 需要在 System Prompt 中看到当前可用的工具清单，以便在对话中正确选择和使用。本节工具清单与 F1 中列出的工具使用指南（Bootstrap 文件之一）相互独立——前者是各工具的具体功能说明（由系统生成），后者是 Owner 编写的工具使用指南（高层约束）。

- **工具清单内容**：字段内容、常用/延迟分组、危险度标记、长度截断与探索提示由 [tools §F1](tools.md)（工具注册与发现）定义。本模块消费 tools 模块产出的工具清单结果
- 技能清单的注入位置和组装时机由 [skills §F4](skills.md)（技能清单）定义。技能正文内容在模型决定使用时按需注入，不预先进入 System Prompt

> **交叉引用**：当前 Agent 可用的工具范围（白名单/黑名单），由 [agent §F3](agent.md)（Agent 能力组合）定义。本模块负责将可用工具清单产出为 System Prompt 中的分组描述文本。

### F3. 长期记忆注入

Agent 应能获取跨 Session 保留的长期记忆内容。长期记忆在主 Agent Session 启动时加载，作为 System Prompt 的组成部分；子 Session 不加载（见 F8 与安全性小节）。

> **交叉引用**：记忆的挖掘机制。详见 [memory §F1](memory.md)（Session 结束后自动挖掘记忆）。
> **交叉引用**：对话中自动注入相关记忆的另一机制详见 [memory §F4](memory.md)（对话中自动注入相关记忆）。

### F4. 运行时上下文注入

每次 API 调用时，Agent 需要知道当前的运行时上下文：

- **频道上下文**：当前 Session 所在的频道名称
- **工作目录**：当前 Session 的工作目录路径

这些上下文信息每次 API 调用即时获取，不持久化存储。

> **交叉引用**：工作目录的解析顺序详见 [session §F8](session.md)（工作目录）。

### F5. 追加指令管理

Owner 可以在对话中通过命令管理 System Prompt 末尾的追加指令。追加指令的写入、查看、清除由 slash 模块通过 `/system` 指令提供入口；本模块定义与 System Prompt 内容状态相关的专属行为（如清除时的缓存失效）。

- Owner 清除追加指令时，触发 System Prompt 重新组装，确保被清除的追加指令不再出现在后续 System Prompt 中

System Prompt 末尾的追加区除 Owner 追加指令外，还承载系统注入的上下文（如 workflow 上下文）。系统注入内容的写入与移除由对应功能模块触发，写入与移除不触发 System Prompt 重新组装，也不清除 Owner 追加指令。

> **交叉引用**：系统注入内容的一个实例——workflow 上下文的注入与移除。详见 [workflow §F2](workflow.md)（workflow 启动）、[workflow §F8](workflow.md)（流程生命周期）。
> **交叉引用**：追加指令的追加、查看、清除命令入口。详见 [slash §F6](slash.md)（System Prompt 追加）。
> **交叉引用**：追加指令的持久化由 [session §F2](session.md)（恢复时的 System Prompt 重建）管理。本节仅定义 System Prompt 内容层的专属行为。

### F6. 内容缓存与自动刷新

System Prompt 的组装触发时机是固定的，两次组装之间内容不变，利用此不变性做缓存：

- 组装结果按 F1 固定顺序产出，配置不变时多次组装结果逐字节相同
- 以下事件触发 System Prompt 重新组装：
  - 新 Session 创建
  - 从归档恢复 Session
  - 上下文压缩完成后
  - Owner 清除追加指令
- 重新组装时从各数据源（Bootstrap 文件、工具注册中心、技能注册中心、长期记忆）读取最新内容
- 两次组装之间不响应数据源变更——文件修改、工具或技能注册中心变更均在下次组装时反映
- 组装结果缓存于 Session 运行时，每次 API 调用直接取出，不重复组装

> **交叉引用**：Bootstrap 文件等数据源变更的生效机制。详见 [config §F4](config.md)（配置重载）；本节仅约定两次组装之间不响应此类变更。

> **交叉引用**：重建触发的外部事件来源——新 Session 创建详见 [session §F1](session.md)（对话持久化与恢复），归档恢复详见 [session §F2](session.md)（恢复时的 System Prompt 重建），上下文压缩行为详见 [session §F3](session.md)（长对话压缩），Owner 清除追加指令详见 [slash §F6](slash.md)（System Prompt 追加）。

### F7. API 前缀缓存利用

System Prompt 中不变的前缀部分应利用 AI 服务商的前缀缓存机制，减少重复内容的 token 计费：

- System Prompt 中不变部分和变化部分之间有明确的分隔，使缓存层能识别可缓存的前缀范围
- 每次 API 调用时变化的部分（运行时上下文、追加指令、系统注入的上下文）不参与前缀缓存

> **交叉引用**：各服务商的具体缓存参数适配和 token 统计。详见 [llm §F8](llm.md)（缓存成本优化）、[llm §F9](llm.md)（用量统计）。本模块仅负责不变/变化内容的划分，并维持前缀稳定。

### F8. Session 类型适配

不同类型的 Session 加载不同的 System Prompt 内容：

- **主 Agent Session**：加载全部内容（F1 必须加载的文件 + 可选加载的内容 + 工具清单）。其中长期记忆在精简模式下仍加载——长期记忆的加载只取决于 Session 类型，不取决于身份加载模式
- **子 Session**：仅加载 F1 必须加载的文件 + 工具清单，不加载 F1 中列出的可选加载内容（无论 Agent 配置的身份加载模式为何）。其中长期记忆和自定义引导指令的排除要求见安全性小节
- **无 Bootstrap 文件的 Session**：仅加载工具清单，跳过所有 Bootstrap 文件

> **注**：F4 运行时上下文（频道、工作目录）对所有 Session 类型均加载。

> **交叉引用**：三种 Session 类型由 Session 创建流程综合判定，本模块负责按类型加载对应内容。子 Session 的创建控制与 spawn 参数详见 [agent §F7](agent.md)（子 Session 创建（Spawn））。

## 非功能需求

### 性能

- 组装结果在事件之间复用，不触发重复读取和组装
- System Prompt 总体积应保持精简——超出上限时智能截断（如工具列表超长时精简并提示探索方式），不盲目扩大

### 可用性

- 单个部分组装失败时跳过该部分，其余部分继续，不阻断整个 System Prompt 的生成
- 所有部分组装结果均为空的极端情况下，使用默认的最简 System Prompt，确保对话不中断
- 配置变更无需 Owner 手动触发，下次重新组装自动生效

### 安全性

- 子 Session 不暴露长期记忆内容和自定义引导指令
- 追加指令不暴露给非 Owner 用户
