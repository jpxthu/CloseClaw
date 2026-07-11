# System Prompt 需求

## 概述

System Prompt 是每次与 AI 模型通信时发送的引导前缀，承载 Agent 的身份定义、能力范围、运行上下文以及 Owner 临时注入的指令。Owner 需要：通过 workspace 配置文件定义 Agent 的行为准则，让 Agent 了解自己有哪些工具和技能，在不同类型的会话中加载恰当的上下文，并能随时追加临时指令——同时确保静态内容高效缓存、来源变更自动反映。

## 功能需求

### F1. 身份与行为准则定义

Owner 在 workspace 目录下通过一系列配置文件（统称 bootstrap 文件）定义 Agent 的身份、操作规范、工具使用指南和 Owner 偏好。Agent 每次会话启动时自动加载这些定义，作为 System Prompt 的核心组成部分。

- 必须加载的文件：操作规程、角色定义、身份标识、Owner 信息、工具使用指南
- 可选加载的文件：自定义引导指令、长期记忆（取决于会话类型和加载模式）
- 文件不存在时静默跳过，不报错
- 多文件按固定顺序注入，操作规程排在最高优先级
- 心跳工作流配置不属于 System Prompt 注入范围——心跳由定时任务按需触发读取，不进入日常会话的 System Prompt

> **交叉引用**：bootstrap 文件的加载模式（Full/Minimal）和所在目录路径，由 [agent §F1](../requirements/agent.md)（配置档案）、[agent §F2](../requirements/agent.md)（身份与人格分离）定义。会话创建/恢复/压缩完成时的重建触发，见 [session §F1](../requirements/session.md)（对话持久化与恢复）、[session §F3](../requirements/session.md)（长对话压缩）。

### F2. 工具与技能清单注入

Agent 需要在 System Prompt 中看到当前可用的工具清单和技能清单，以便在对话中正确选择和使用。

- **工具清单**：列出所有可用工具的名称、分组归属、功能描述和危险度标记（只读/破坏性）。常用工具注入完整使用说明，低频工具至少展示名称和危险度标记。工具总数超限时需截断并给出探索提示
- **技能清单**：列出所有可用技能的摘要（名称 + 说明 + 触发条件），按固定优先级排序（内置 > 全局 > Agent 专属 > 项目专属）。无可用技能时该清单不出现
- 当工具定义或技能文件发生变更时，清单自动更新
- Skill 正文内容在模型决定使用时按需注入，不预先进入 System Prompt

> **交叉引用**：当前 Agent 可用的工具/技能范围（白名单/黑名单），由 [agent §F3](../requirements/agent.md)（Agent 能力组合）定义。本模块负责将可用清单渲染为 System Prompt 中的分组描述文本。F1 bootstrap 中的"工具使用指南"是 Owner 的高层使用规范，与本节的工具功能说明相互独立。

### F3. 长期记忆注入

Agent 应能获取跨会话保留的长期记忆内容。长期记忆在会话启动时加载，作为 System Prompt 的组成部分。当记忆文件更新时，内容自动刷新。

> **交叉引用**：记忆的存储路径和写入机制，见 [memory 模块](../requirements/)（待创建）。记忆内容的搜索策略由 memory 模块定义。

### F4. 运行时上下文注入

每次 API 调用时，Agent 需要知道当前的运行时上下文：

- **频道上下文**：当前会话所在的聊天名称/类型
- **工作目录**：当前会话的工作目录路径
- **Git 状态**：工作目录的 Git 分支和变更状态（默认关闭，Owner 可通过配置开启；非 Git 仓库时不注入）

这些上下文信息每次请求即时获取，不持久化存储。

> **交叉引用**：工作目录的解析顺序（spawn 参数 > Agent 配置 > 父 Agent 继承），见 [agent §F13](../requirements/agent.md)（工作目录权限）。

### F5. 动态指令管理

Owner 可在对话中通过指令向 System Prompt 末尾追加或清除临时指令：

- 追加指令：多次叠加，每条独立保留
- 查看指令：列出当前所有追加的指令条目
- 清除指令：一键清空所有追加条目
- 追加内容不区分类型（行为约束、临时规则、上下文提示均可）
- 清除指令时，静态内容缓存同步刷新，确保下次重建为干净状态

> **交叉引用**：本条定义 /system 指令的增删查行为。指令的路由和消息解析，由 [session §F2](../requirements/session.md)（Agent 角色与能力配置）处理。

### F6. 内容缓存与自动刷新

System Prompt 中在会话生命周期内不变的内容应走缓存机制，避免每次请求都重复读取文件和生成描述文本：

- 文件内容未变更时复用已加载的版本，不触发重复读取
- 以下事件触发全部缓存失效并重建：
  - Owner 执行清空会话指令
  - Owner 清除追加指令
  - 从归档恢复会话
  - 上下文压缩完成后
- 单个文件变更时仅失效对应部分的缓存，不影响其他部分
- 工具定义或技能文件变更时，变更检测自动触发对应缓存失效，下次请求时重建

> **交叉引用**：重建触发的外部事件来源——会话恢复见 [session §F1](../requirements/session.md)，压缩完成见 [session §F3](../requirements/session.md)。Agent 配置文件变更的检测机制由 [agent §F6](../requirements/agent.md)（运行时配置查询）管理，本模块作为消费方响应。

### F7. API 前缀缓存利用

System Prompt 中不变的前缀部分应利用 AI 服务商的前缀缓存机制，减少重复内容的 Token 计费：

- System Prompt 中不变部分和变化部分之间有明确的分隔，使缓存层能识别可缓存的前缀范围
- 不变的前缀部分应保证字节级稳定，以持续命中服务端缓存
- 每次请求变化的部分（动态上下文、追加指令）不参与前缀缓存

> **交叉引用**：各服务商的具体缓存参数映射和 Token 统计，见 [llm §F8](../requirements/llm.md)（缓存成本优化）、[llm §F9](../requirements/llm.md)（用量统计）。本模块仅负责不变/变化内容的划分和前缀稳定性保证。

### F8. 会话类型适配

不同类型的会话加载不同的 System Prompt 内容：

- **主 Agent 会话**：加载全部内容（F1 必须加载的文件 + 可选加载的文件 + 工具清单 + 技能清单）
- **子 Agent 会话**：仅加载 F1 必须加载的文件 + 工具清单 + 技能清单，不加载可选加载的文件
- **无 workspace 的会话**：仅加载工具清单和技能清单，跳过所有 bootstrap 文件

> **交叉引用**：会话类型的判定（主/子/无 workspace）和子 Agent 的 spawn 参数，由 [agent §F7](../requirements/agent.md)（子 Agent 创建）定义。各类型对应的 System Prompt 加载范围由本模块定义。

### F9. 跨会话持久化

Owner 追加的动态指令应跨会话保留：

- 追加指令随会话检查点持久化，会话恢复后完整保留
- 上下文压缩不删除追加的指令内容
- 清空指令后持久化更新，下次恢复不会复现

> **交叉引用**：检查点的存储格式和写入机制，见 [session §F1](../requirements/session.md)（对话持久化与恢复）。本模块仅声明动态指令的持久化约束。

## 关联设计文档

- [✓] docs/design/system_prompt/README.md
- [✓] docs/design/system_prompt/static-layer.md
- [✓] docs/design/system_prompt/dynamic-layer.md
- [✓] docs/design/system_prompt/fragment-provider.md
- [✓] docs/design/system_prompt/kv-cache.md
- [✓] docs/design/system_prompt/appends.md

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

- 文件变更后，Owner 能观察到 System Prompt 已反映最新内容（例如通过 /status 或类似机制查看重建状态）
