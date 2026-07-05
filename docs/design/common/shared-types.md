# 共享类型

## 概述

共享类型是跨模块传递的纯数据结构，被 2 个及以上模块共同消费。每个共享类型在本文档中唯一定义，各业务模块文档通过引用指向此处，不在自身文档中重复描述字段结构。

> **本文档是 common crate 的内容边界。**
> - 本文档中定义的类型 → 代码位于 common crate（或其子 crate）
> - **不在本文档中的类型 → 代码不得出现在 common crate 中**
> - common crate 中出现本文档未收录的类型，说明代码放错了位置——应将该类型移至对应领域模块的 crate，而非将其追加到本文档

本文档不包含 trait 接口定义——核心 trait 见 [core-traits](core-traits.md)。

## 架构

### NormalizedMessage

NormalizedMessage 是平台无关的统一入站消息结构，屏蔽各 IM 平台（飞书、Discord、Telegram 等）和 terminal 渠道的差异。各渠道的 IM Adapter 入站解析产出此结构，Processor Chain 消费（读取内容做标准化和 session_key 计算）。Gateway 消费的是 Processor Chain 产出的 ProcessedMessage，不直接接触 NormalizedMessage。

| 字段 | 类型 | 说明 |
|------|------|------|
| `platform` | string | 平台标识，如 `"feishu"`、`"terminal"` |
| `sender_id` | string | 发送者的平台内 ID |
| `peer_id` | string | 会话对端（群聊 chat_id 或私聊对方 ID） |
| `thread_id` | string? | 话题 ID，可选。不参与 session key 计算，仅用于出站定向回复 |
| `account_id` | string | CloseClaw 本地账号标识，由 sender_id 通过身份映射得到。参与 session 路由 |
| `content` | string | 消息文本内容。非文本消息时可为空 |
| `message_type` | enum | 消息类型：text / image / file / audio |
| `media_refs` | list(MediaRef) | 图片/文件/音频的引用列表，每个元素为 MediaRef 结构（含 `key` 资源标识和 `url` 访问地址）。由 Adapter 负责下载到本地临时路径 |
| `timestamp` | int | 消息发送时间（毫秒级 Unix 时间戳） |

**引用/回复消息处理**：IM Adapter 在解析被引用的消息时，将其内容渲染为 markdown blockquote（`> 引用内容`），截断至 500 字符（超出追加 `...`），拼接在 `content` 字段之前。不传递独立的引用消息字段——LLM 在对话文本中直接看到 blockquote。

**消息过滤规则**：text 类型空 content 消息在解析阶段丢弃，不产 NormalizedMessage。非文本消息（image/file/audio）正常产 NormalizedMessage（message_type 标记类型，media_refs 存储引用，content 可为空），由下游 Gateway 统一处理。非文本消息 media_refs 为空列表时，消息仍正常传递——content 和 media_refs 均为空，下游 Gateway 根据 message_type 判断类型后构造错误回复

**身份映射**：`account_id` 由 IM Adapter 在解析入站消息时填入。与其他字段（platform、sender_id 等直接从消息 payload 提取）不同，account_id 需通过 sender_id 查询账户绑定表获取，非直接取值。映射规则：以 sender_id 为键查询账户绑定表，找到对应的 CloseClaw 账户 ID。一个账户可绑定多个平台的 sender_id。terminal 平台恒为 "owner"，无需查表。详见 [config 模块 accounts.json](../config/README.md)。

**字段填充职责**：各字段由 IM Adapter 入站解析时填充。Processor Chain 不修改 NormalizedMessage 字段——ContentNormalizer 仅读取 content 做文本标准化，SessionRouter 读取 platform/sender_id/peer_id/account_id 计算 session_key。session_key 写入 ProcessedMessage 的 metadata，不写入 NormalizedMessage。

**message_type 与 media_refs**：message_type 由 ContentNormalizer 消费（非 text 跳过标准化）。media_refs 为多模态支持预留，入站链路不消费。

**建模边界**：NormalizedMessage 建模用户主动发送的消息（文本、图片、文件、音频）。卡片交互事件——用户点击消息中嵌入的按钮、选择器等交互控件——属于工具调用的回执，走 tool_result 通道注入对话，不经过 NormalizedMessage 入站通路。各 IM 平台在 Adapter 解析阶段须区分消息事件和交互事件，仅将消息事件转为 NormalizedMessage。

NormalizedMessage 中引用的子结构：

**MediaRef**：图片/文件/音频的资源引用，由 IM Adapter 下载到本地临时路径后填充。

| 字段 | 类型 | 说明 |
|------|------|------|
| `key` | string | 资源标识，平台内的唯一 key |
| `url` | string | 资源访问地址，Adapter 据此下载到本地临时路径 |

### ContentBlock

ContentBlock 是跨模块传递的结构化内容单元。主要用于出站方向——入站方向仅使用 ContentBlock::Text 单个变体作为 ProcessedMessage 的内容载体。所有出站内容——LLM 回复和斜杠指令回复——均以 ContentBlock[] 数组形式传递，贯穿 Verbosity 过滤、DSL 解析、出站日志记录和平台渲染全链路。

ContentBlock[] 的类型定义服务于出站方向——LLM 和斜杠指令以 ContentBlock[] 产出结构化内容。入站方向经 Processor Chain 处理后，标准化文本以 ContentBlock::Text 形式放入 [ProcessedMessage](#processedmessage) 的 content_blocks 字段，入站不涉及 ContentBlock 的其他变体。

ContentBlock 共 7 种变体，按语义和渲染策略分为两类：

**文本类变体**：

| 变体 | 语义 | 渲染行为 |
|------|------|------|
| Text | 文本内容，可含 markdown 格式标记和 DSL 指令行。ContentBlock 中唯一参与 DSL 解析的变体 | DSL 行由 DslParser 剥离后渲染纯文本/富文本。终端输出 ANSI 格式化文本，IM 平台按平台能力输出 markdown 元素 |
| Thinking | LLM 推理过程，终端用户可选的思考展示 | 默认折叠展示（终端 ANSI dim 样式包裹，IM 平台折叠区块）。流式模式下等待全块就绪后一次渲染。DslParser 透传 |

**非文本类变体**（DslParser 透传）：

| 变体 | 语义 | 渲染行为 |
|------|------|------|
| ToolUse | 工具调用请求，含工具名和参数 | 渲染为工具调用信息展示（终端文本，IM 平台卡片）。参数以原始结构渲染 |
| ToolResult | 工具执行结果 | 渲染为结果内容展示。终端按宽度截断，IM 平台富格式渲染 |
| Image | 图片引用，含资源标识和访问地址 | 终端渲染为占位符文本 `[image: name]`，IM 平台渲染为图片元素 |
| Audio | 音频引用，含资源标识和访问地址 | 终端渲染为占位符文本 `[audio: name]`，IM 平台渲染为音频元素 |
| File | 文件引用，含资源标识和访问地址 | 终端渲染为占位符文本 `[file: name]`，IM 平台渲染为文件元素 |

**变体处理规则**：

- **Text 是唯一可能包含 DSL 指令的变体**。DslParser 仅遍历 Text 块逐行扫描 DSL，解析后从 Text 块中移除 DSL 行。其余 6 种变体由 DslParser 透传
- **流式渲染差异化**：Text 块逐行缓冲输出（以句末标点或换行符为行边界）；Thinking/ToolUse/ToolResult 块等待全块就绪后一次交付渲染；Image/Audio/File 块不参与流式渲染，交由平台格式渲染器处理
- **输出格式决策**：各平台 Renderer 按 ContentBlock 类型组合选择输出格式——纯文本块（不含 Thinking/ToolUse/ToolResult 块）→ 纯文本消息；含 Thinking/ToolUse/ToolResult 块或多块 → 富格式/卡片消息
- **Verbosity 过滤**以单个 ContentBlock 为粒度执行——每个 ContentBlock 到达时按当前 Session 的 verbosity 等级判断其可见性，流式模式下逐块实时过滤。Verbosity 等级定义见 [slash 模块 verbose 指令](../slash/verbose.md)

### DslParseResult 和 DslInstruction

DslParseResult 是 DslParser 解析 ContentBlock::Text 中 DSL 指令行的输出结果。存储在 [ProcessedMessage](#processedmessage) 的 metadata 中，供下游 Renderer 消费。DslInstruction 是单条 DSL 指令的结构化表示。

DSL 指令是消息中的交互元素（按钮、选择器等），每条为一行，格式为 `::type[key1:value1;key2:value2;...]`。例如 `::button[label:确认;action:confirm;value:1]` 和 `::selector[label:选颜色;options:红,蓝;action:pick]`。DslParser 遍历 ContentBlock::Text 逐行扫描，匹配 DSL 格式的行解析为 DslInstruction，从 Text 块中移除 DSL 行后与其他 ContentBlock 一并传递。DslParser 仅处理 Text 变体，其余变体透传。

**DslInstruction 结构**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `instruction_type` | string | 指令类型。已知类型：`button`（按钮）、`selector`（选择器） |
| `params` | map(string→string) | 指令参数键值对，从 DSL 行中解析。例如 `::button[label:确认;action:confirm;value:1]` 解析为 `{label: "确认", action: "confirm", value: "1"}` |

**DslParseResult 结构**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `instructions` | list(DslInstruction) | 解析出的 DSL 指令列表，按原文出现顺序排列。无 DSL 指令时为空列表 |

DslParseResult 与经 DslParser 剥离 DSL 行后的 ContentBlock[] 一同传递——ContentBlock[] 承载去 DSL 后的纯文本和其他内容块，DslParseResult 承载从 ContentBlock[] 中提取的结构化指令。两者通过 [ProcessedMessage](#processedmessage) 打包交付 Renderer。

### ProcessedMessage

ProcessedMessage 是 Processor Chain 的输出结构，Gateway 的消费入口。入站和出站方向共用同一结构，content_blocks 在不同方向携带不同复杂度的内容，metadata 携带方向相关的计算结果。

| 字段 | 类型 | 说明 |
|------|------|------|
| `content_blocks` | ContentBlock[] | 处理后的内容块数组。入站方向为单个 ContentBlock::Text（ContentNormalizer 标准化后的文本），出站方向为经 DslParser 处理后的 ContentBlock[]（Text 块已剥离 DSL 行，其余块透传） |
| `metadata` | map(string→string) | 方向相关的键值对。入站含 `session_key`（SessionRouter 计算的消息级标识）和 `message_type`（来自原始 NormalizedMessage，供 Gateway 做非文本路由判断），出站含 `dsl_result`（DslParser 产出的 DslParseResult，JSON 序列化） |

入站和出站不区分类型——同一个 ProcessedMessage 结构，内容形态和 metadata 字段按方向不同而不同。

### SlashResult

SlashResult 是斜杠指令 Handler 返回的执行结果类型。每个变体封装一种指令的副作用逻辑。Handler 返回 SlashResult 后，由 Gateway 构造 SideEffectContext 并触发 SlashResult 执行，各变体自行完成对应的 session 操作和消息回复。

SlashResult 共 10 种变体：

| 变体 | 用途 | 产出 |
|------|------|------|
| SetMode | 设置会话运行模式（Normal/Plan） | ContentBlock::Text（确认信息） |
| SetReasoning | 设置推理深度 | ContentBlock::Text（确认信息） |
| SetVerbosity | 设置信息展示等级 | ContentBlock::Text（确认信息） |
| Reply | 纯文本回复，用于 /help、/status 等仅需回复文本的指令 | ContentBlock::Text（回复文本） |
| NewSession | 创建新会话 | ContentBlock::Text（确认信息） |
| Stop | 终止当前运行（含级联终止子 session） | ContentBlock::Text（确认信息） |
| Compact | 触发对话历史压缩 | ContentBlock::Text（压缩结果） |
| SystemAppend | 向 system prompt 追加内容 | ContentBlock::Text（确认信息） |
| Exec | 执行系统命令（高危操作，执行前经 Permission 模块校验） | ContentBlock[]（命令输出经出站 Processor Chain） |
| Unknown | 未知指令回退 | ContentBlock::Text（提示信息） |

**执行模型**：Gateway 不感知具体 SlashResult 变体。Handler 返回 SlashResult 后，Gateway 统一调用执行方法，由各变体自行完成副作用。新增指令只需新增 SlashResult 变体及其执行实现，Gateway 无需改动。

**SideEffectContext**：Gateway 在收到 SlashResult 后构造的执行上下文。携带当前 Session 的操作能力（用于模式切换、会话创建/停止、压缩等操作）和回复通道（用于产出回复内容）。SideEffectContext 由 Gateway 管理，SlashResult 不持有其引用。

**与 ContentBlock[] 的关系**：SlashResult 各变体在执行中通过 SideEffectContext 的回复通道产出 ContentBlock[]，进入出站 Processor Chain——与 LLM 的 UnifiedResponse 走同一条出站处理路径（VerbosityFilter → DslParser → OutboundRawLog → IM Adapter 渲染发送）。

### FragmentContext

FragmentContext 是 PromptFragmentProvider 片段生成时的输入上下文，由 System Prompt Builder 构建后传递给各 Provider。

| 字段 | 类型 | 说明 |
|------|------|------|
| `agent_id` | string | Agent 标识。Skills 按此过滤可见 skill |
| `bootstrap_mode` | enum | BootstrapMode::Minimal（精简模式）或 BootstrapMode::Full（完整模式），Bootstrap 按此选择文件集合 |
| `workdir` | string | agent 工作目录路径，Bootstrap 按此查找 bootstrap 文件 |

### PromptFragment

PromptFragment 是单个 PromptFragmentProvider 产出的静态层片段。

| 字段 | 类型 | 说明 |
|------|------|------|
| `section_title` | string | Section 标题，如 `## AGENTS.md`、`## Available Skills` |
| `section_type` | enum | Section 类型：bootstrap 文件、工具列表、skill 清单、长期记忆 |
| `content` | string | 渲染完成的文本内容 |

### RenderedOutput

RenderedOutput 是 IMPlugin 渲染方法产出的平台原生格式消息结构。渲染产出数据，发送执行副作用——Gateway 在两步之间插入中间件（审计、频率限制等）。

| 字段 | 类型 | 说明 |
|------|------|------|
| `msg_type` | string | 消息格式类型（如 `"text"`、`"interactive"`），由 Renderer 按内容特征选择 |
| `payload` | any | 平台原生格式的消息体，结构由各平台 Renderer 定义。Gateway 中间件和 Adapter 发送不解析 payload 内容 |

**输出格式决策**：各平台 Renderer 按 ContentBlock 类型组合选择 msg_type——纯文本块（不含 Thinking/ToolUse/ToolResult）→ `"text"`；含 Thinking/ToolUse/ToolResult 块或多块 → `"interactive"`。

### VerbosityLevel

VerbosityLevel 是出站信息展示等级的枚举，控制 VerbosityFilter 对 ContentBlock 的过滤策略。由 `/verbose` 指令设置，Session 存储，出站 Processor Chain 的第一道过滤（VerbosityFilter，priority 5）消费。

三个等级：

| 等级 | 值 | 过滤行为 |
|------|---|---------|
| full | `"full"` | 展示全部：思考过程、工具调用、工具结果、最终回复 |
| normal | `"normal"` | 展示工具调用和结果作为进度提示，隐藏思考过程 |
| off | `"off"` | 仅展示最终回复，隐藏所有中间过程 |

**作用范围**：Verbosity 控制展示内容，不影响 LLM 推理深度和 Agent 行为模式。切换等级不影响当前正在输出的消息——仅对后续新消息生效。非文本媒体块（Image/Audio/File）属于最终回复的一部分，不受 VerbosityLevel 过滤——在所有等级下均展示。

### PlanState

PlanState 是 Plan Mode 下的规划状态枚举，由 mode 模块管理，Session 持久化。Compaction 对此状态做隔离保护（不压缩 plan 相关消息），Session 恢复时重建 PlanState。

PlanState 描述当前规划的阶段和未完成步骤列表：

| 字段 | 类型 | 说明 |
|------|------|------|
| `phase` | enum | 当前阶段：Research / Design / Review / FinalPlan / Interview |
| `pending_steps` | list(string) | 未完成的规划步骤标识列表，用于 compaction 保护和恢复后继续 |
| `plan_file_path` | string | plan 文件的路径，Agent 写入和读取的唯一可写目标 |

### CompactConfig

CompactConfig 是会话历史压缩的配置结构，由 Config 模块加载，Gateway 和 Session 在 compaction 触发时读取。控制压缩何时自动触发、触发阈值的估算参数，以及容错机制。

| 字段 | 类型 | 说明 |
|------|------|------|
| `chars_per_token` | f64 | 每 token 对应字符数估算系数，用于根据字符数推算 token 用量。默认 0.25（即 1 token ≈ 4 字符） |
| `auto_compact_buffer_tokens` | usize | 上下文窗口边缘预留的 buffer token 数，低于该阈值时触发自动压缩。默认 13,000 |
| `max_consecutive_failures` | usize | 连续压缩失败的最大次数，超过后断路器触发停止自动压缩。默认 3 |

**触发条件**：Session 检测到当前上下文 token 用量超过 `上下文窗口 - auto_compact_buffer_tokens` 时触发自动压缩。手动 `/compact` 指令不受此阈值限制。

**保留策略**：Compaction 保留最近 N 条消息（策略由 Compaction 模块实现），对 PlanState 相关消息做隔离保护不予压缩。

### PendingMessage

PendingMessage 是待审批消息的暂存结构，存储在 Session 的消息队列中。当 Permission 模块拦截需要审批的消息时，将其包装为 PendingMessage 推入 Session 的待审批队列，等待用户审批后继续执行。

| 字段 | 类型 | 说明 |
|------|------|------|
| `message_id` | string | 消息 ID，全局唯一标识 |
| `content` | string | 消息内容，待审批的消息文本 |
| `created_at` | datetime | 创建时间（UTC），Permission 模块可据此判断审批超时 |
| `sent` | bool | 是否已发送。初始值为 false，审批通过发送后标记为 true（`mark_sent()`） |

**审批状态推断**：`sent` 字段为 true 表示消息已审批通过并发送；审批拒绝时，消息从队列中移除（delete），不保留任何痕迹。PendingMessage 不包含"拒绝"状态——拒绝即删除。

**数据流**：Session 执行消息前 → Permission 模块拦截 → 包装为 PendingMessage → 推入 Session 的 pending 队列 → 用户审批 → 标记 sent / 删除 → 继续执行。

### ReasoningLevel

ReasoningLevel 是 LLM 推理深度控制的枚举，描述 LLM 在响应中投入的推理计算量。由 `/reasoning` 指令设置，Gateway 传递给 LLM Provider，Session 持久化。推理深度越高，LLM 花费更多 token 做内部推理（thinking/reasoning），适用于复杂问题。

四个等级：

| 等级 | 值 | 行为 |
|------|----|------|
| Low | `"low"` | 低推理深度，最小推理 token 消耗。适合简单问答、常规对话 |
| Medium | `"medium"` | 中等推理深度。平衡推理质量和 token 成本 |
| High | `"high"` | 高推理深度（默认等级）。适合复杂问题分析、代码审查等 |
| Max | `"max"` | 最大推理深度，最大推理 token 消耗。适合深度研究、复杂架构设计 |

**与 VerbosityLevel 的区别**：ReasoningLevel 控制 LLM 产出前的推理计算量（影响输出的 Thinking 块内容质量和长度），VerbosityLevel 控制输出时是否展示中间推理过程。两轴正交——即使 ReasoningLevel = Max，VerbosityLevel = off 时 LLM 仍可产出完整推理，但展示时隐藏 Thinking 块。

**作用范围**：ReasoningLevel 仅对后续 LLM 调用生效。切换等级不影响当前正在运行的 LLM 调用。

### PromptOverrides

PromptOverrides 是 System Prompt 的覆盖配置，允许各 Agent 按需替换 system prompt 的特定层级。由 Gateway 在构建 system prompt 时传递给 SystemPromptBuilder。当不存在覆盖时（所有字段均为 None），按正常的三层优先级渲染各 section。

三层优先级覆盖（从高到低）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `override_prompt` | string? | 最高优先级的全量覆盖。设置后将替换整个静态层，忽略其他层级和 section 渲染 |
| `agent_prompt` | string? | 次优先级的 agent 级 prompt。替换 agent 自动生成的 prompt 段 |
| `custom_prompt` | string? | 最低优先级的用户自定义 prompt。在 agent 级 prompt 之上叠加用户定制内容 |

**解析策略**：SystemPromptBuilder 按优先级检查——先检查 `override_prompt`（非 None 则直接使用），否则检查 `agent_prompt`，再检查 `custom_prompt`，全部 None 时使用正常的 section 渲染流程。

### SessionCheckpoint

SessionCheckpoint 是 Session 持久化快照，由 Gateway 在 Session 生命周期关键节点（创建、状态变更、停止）写入 StorageProvider。Session 恢复时从存储读取此结构重建会话状态。

| 字段 | 类型 | 说明 |
|------|------|------|
| `session_id` | string | 会话唯一标识 |
| `agent_id` | string | 与会话关联的 agent 标识 |
| `channel` | string | 平台通道标识，如 `"feishu"`、`"discord"` |
| `status` | SessionStatus | 会话状态枚举（见下文） |
| `last_activity` | int | 最后活动 Unix 时间戳 |

**SessionStatus 枚举**：

| 变体 | 说明 |
|------|------|
| Active | 会话活跃，正在处理消息 |
| Idle | 会话空闲，无待处理的工作 |
| Stopped | 会话已停止 |

**数据流**：Session 状态变更 → Gateway 构造 SessionCheckpoint → 调用 StorageProvider.save_checkpoint() → 写入持久化存储 → Session 恢复时调用 StorageProvider.load_checkpoint() → 重建 Session。

### PersistResult

PersistResult 是持久化操作（save / load / delete / flush）的执行结果类型。所有 StorageProvider 实现的方法均返回此类型，允许调用方统一错误处理。

三种变体：

| 变体 | 说明 |
|------|------|
| Success | 操作成功完成 |
| PartialSuccess { warnings: Vec\<string\> } | 操作完成但存在非致命警告（如部分数据未刷新到磁盘） |
| Failure(string) | 操作失败，携带错误描述 |

**使用场景**：Gateway 在保存 checkpoint 后检查 PersistResult，Failure 时记录错误日志并尝试重试或降级。PartialSuccess 仅记录警告日志，不阻塞后续流程。

### UnifiedResponse

UnifiedResponse 是 LLM Provider 调用产出的统一响应结构。所有 LLM Provider（OpenAI、Claude 等）的原始响应经 LLM 模块的 Interpreter 统一转换为此结构，供上游 Gateway 和 Session 消费。

| 字段 | 类型 | 说明 |
|------|------|------|
| `content_blocks` | ContentBlock[] | 有序的内容块列表。LLM 产出的文本、思考过程、工具调用等均按序排列在此 |
| `usage` | UnifiedUsage | Token 用量统计（见 [UnifiedUsage](#unifiedusage)） |
| `finish_reason` | string? | 响应结束原因（如 `"stop"`、`"length"`、`"tool_use"`），由 LLM Provider 报告 |

**数据流**：LLM Provider 原始响应 → Interpreter 统一转换 → UnifiedResponse → Gateway 接收 → Session 持久化（消息维度） → ContentBlock[] 进入出站 Processor Chain。

### UnifiedUsage

UnifiedUsage 是 LLM 调用的 token 统计数据，作为 [UnifiedResponse](#unifiedresponse) 的子结构携带。由 LLM Provider 报告，Interpreter 转换。下游用于计费统计、用量监控和触发压缩决策。

| 字段 | 类型 | 说明 |
|------|------|------|
| `prompt_tokens` | u32 | 提示（输入）token 数 |
| `completion_tokens` | u32 | 补全（输出）token 数 |
| `total_tokens` | u32? | 总 token 数（可选，部分 Provider 不报告此项） |
| `reasoning_tokens` | u32? | 推理 token 数（可选，仅支持 reasoning 的 Provider 报告） |
| `cache_read_tokens` | u32? | 缓存读取 token 数（可选，仅支持 prompt caching 的 Provider 报告） |
| `cache_write_tokens` | u32? | 缓存写入 token 数（可选，仅支持 prompt caching 的 Provider 报告） |

**数据流**：LLM Provider 原始响应 → Interpreter 解析 token 字段 → 填充 UnifiedUsage → 随 UnifiedResponse 传递给 Gateway → Gateway 累计用量指标 → 超过阈值时触发压缩决策。

## 数据流

### NormalizedMessage

NormalizedMessage 的全系统流动路径：

```
IM 平台 webhook / terminal stdin
  ↓
IM Adapter 入站解析（各平台插件）
  → 平台格式转 NormalizedMessage { platform, sender_id, peer_id, thread_id?, account_id, content, message_type, media_refs, timestamp }
  ↓
Processor Chain 入站
  → RawLog（记录日志）→ SessionRouter（计算 session_key）→ ContentNormalizer（文本标准化）
  → 产出 ProcessedMessage
  ↓
Gateway 路由
  → SessionManager 查找/创建 session → LLM 对话 / SlashDispatcher
```

NormalizedMessage 仅用于入站方向。出站方向使用 ContentBlock[]（LLM 输出）和 [ProcessedMessage](#processedmessage)（经 Processor Chain 处理后的中间结构），与 NormalizedMessage 无关。

### ContentBlock

ContentBlock[] 的出站流动路径：

```
LLM UnifiedResponse / SlashResult 变体
  ↓
ContentBlock[] 进入出站处理链路
  ↓
[Processor Chain 出站: VerbosityFilter → DslParser → OutboundRawLog]
  ↓
ProcessedMessage { content_blocks, metadata[dsl_result] }
  ↓
[IM Adapter 渲染] — 按块类型选择渲染策略，输出平台原生格式
  ├─ 批量模式：一次性渲染全部 ContentBlock[]
  └─ 流式模式：增量渲染，Text 块逐行缓冲输出，非文本类块等全块就绪后一次渲染
  ↓
[中间件插入点] — Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件。中间件为 Gateway 内部的拦截链，具体中间件类型和注册机制由 Gateway 管理，不在 shared-types 范围
  ↓
IM Adapter 发送到目标平台
```

ContentBlock[] 流式与非流式走同一条预处理管线——Verbosity 过滤和 DslParser 解析同时适用于批量和流式。DslParser 对流式增量文本零开销透传。两者的差异仅在渲染阶段：批量模式一次性渲染，流式模式增量渲染。

各共享类型流动路径的详细描述见下文各类型的数据流节。

### DslParseResult / DslInstruction

DslParseResult 的流动嵌入在 ContentBlock[] 的出站路径中：

```
ContentBlock[]（来自 LLM UnifiedResponse / SlashResult）
  ↓
[Processor Chain 出站: VerbosityFilter] — 按 Session Verbosity 等级逐块过滤
  ↓
DslParser 遍历 Text 块，逐行扫描 DSL
  ├── 匹配 DSL 行 → 解析为 DslInstruction → 加入 instructions 列表 → 从 Text 块中移除该行
  └── 非 DSL 行 → 保留在 Text 块中
  ↓
DslParseResult { instructions } + 更新后的 ContentBlock[]
  ↓
[Processor Chain: OutboundRawLog] — 出站日志记录
  ↓
打包为 [ProcessedMessage](#processedmessage)
  ↓
Renderer 消费 DslParseResult：
  ├── button / selector → 渲染为平台交互元素（IM 平台卡片 button 组件、终端纯文本提示行）
  └── 其他指令类型 → Renderer 按平台能力处理或忽略
```

DslParseResult 的生命周期始于 DslParser 解析、终于 Renderer 渲染。中间经 OutboundRawLog（Processor Chain 出站日志）和 [ProcessedMessage](#processedmessage) 传递。DslParseResult 本身不被 Verbosity 过滤影响——DslParser 仅处理已通过过滤的 ContentBlock[]，因此 DslParseResult 中只包含可见块中的 DSL 指令。

### ProcessedMessage

入站方向：

```
NormalizedMessage → Processor Chain 入站（RawLog → SessionRouter → ContentNormalizer）
  ↓
ProcessedMessage {
  content_blocks: [ContentBlock::Text("标准化后文本")],
  metadata: { session_key: "{timestamp}-{hash}", message_type: "<原始 message_type>" }
}
  ↓
Gateway — 先检查 message_type：非 text（image/file/audio）构造错误回复经简化出站路径发送；text 消息从 content_blocks[0] 取 Text 内容做路由决策（/ 开头 → 斜杠指令；否则 → LLM 对话），从 metadata 取 session_key 传给 SessionManager
```

出站方向：

```
ContentBlock[]（LLM 产出 / SlashResult 变体）→ Processor Chain 出站（VerbosityFilter → DslParser → OutboundRawLog）
  ↓
ProcessedMessage {
  content_blocks: [去 DSL 后的 ContentBlock[]],
  metadata: { dsl_result: "<DslParseResult JSON>" }
}
  ↓
Gateway 出站日志 → IM Adapter 渲染（消费 content_blocks + metadata[dsl_result]）→ 发送
```

ProcessedMessage 的生命周期：Processor Chain 产出 → Gateway 消费后即完成使命，不进入 Session 持久化。

### SlashResult

SlashResult 的执行流程：

1. Gateway 将 / 开头的消息路由到 SlashDispatcher
2. SlashDispatcher 解析指令名和参数，查找对应 Handler
3. Handler 处理完成后返回 SlashResult 变体
4. Gateway 构造 SideEffectContext，触发 SlashResult 执行
5. Exec 变体：Gateway 调用 Permission 模块校验命令权限（校验通过方继续执行，拒绝则返回权限错误）
6. SlashResult 变体通过 SideEffectContext 完成副作用，分两条路径：
   - 回复路径：产出 ContentBlock[] → 出站 Processor Chain → IM Adapter 渲染发送
   - 会话路径：执行 Session 操作（模式切换、创建、停止、压缩等）

SlashResult 的生命周期：Handler 返回 → Gateway 构造 SideEffectContext 并触发执行 → 各变体通过 SideEffectContext 完成副作用后销毁。

### FragmentContext / PromptFragment

FragmentContext 和 PromptFragment 的流动嵌入在 system prompt 静态层的构建流程中：

```
SessionManager 触发构建
  ↓
System Prompt Builder 构建 FragmentContext（agent_id + bootstrap_mode + workdir）
  ↓
遍历已注册的 PromptFragmentProvider → 传入 FragmentContext → 各 Provider 产出 PromptFragment
  ↓
按优先级拼接所有 PromptFragment.content
  ↓
写入 ConversationSession 的 system prompt 字段
```

FragmentContext 由 Builder 一次性构建，所有 Provider 共享同一上下文。PromptFragment 由各 Provider 独立产出，生命周期止于 Builder 完成拼接。

### RenderedOutput

RenderedOutput 的流动嵌入在 IM Adapter 出站渲染流程中：

```
ContentBlock[] + DslParseResult（经 Processor Chain 出站处理后）
  ↓
IMPlugin.render() → RenderedOutput { msg_type, payload }
  ↓
[Gateway 中间件插入点] — 审计、频率限制等
  ↓
IMPlugin.send(payload, peer_id, thread_id) → 平台发送 API
```

RenderedOutput 的生命周期：IMPlugin 渲染产出 → Gateway 中间件 → IMPlugin 发送后销毁。

### VerbosityLevel

VerbosityLevel 的读写路径：

```
/verbose <等级> 指令
  ↓
VerboseHandler 设置等级
  ↓
Gateway 写入 Session 的 Verbosity 字段
  ↓
出站 Processor Chain 的第一道 Processor（VerbosityFilter，priority 5）读取
  ↓
按等级过滤 ContentBlock[] — 去除被隐藏的块类型
  ↓
过滤后的 ContentBlock[] 继续后续出站链路（DslParser → OutboundRawLog → Renderer）
```

### PlanState

PlanState 的管理路径：

```
/plan 指令 → mode 模块创建 PlanState
  ↓
Session 存储 PlanState（随 checkpoint 持久化）
  ↓
Compaction 时隔离保护 PlanState 相关消息（不压缩）
  ↓
Session 恢复时从 checkpoint 重建 PlanState
  ↓
Plan Mode 结束时销毁 PlanState
```

### CompactConfig

CompactConfig 的配置和消费路径：

```
Config 模块加载 CompactConfig（来自配置文件）
  ↓
Session 在 compaction 触发时读取：
  ├── 自动触发：检查上下文 token 是否超过 `窗口 - auto_compact_buffer_tokens`
  │   └── 超过阈值 → 调用 Compaction 模块执行压缩
  ├── 手动触发：/compact 指令 → 直接调用 Compaction 模块
  └── 失败处理：连续失败超过 max_consecutive_failures → 断路器切断自动压缩
  ↓
Compaction 模块根据 chars_per_token 估算 token 用量
```

### PendingMessage

PendingMessage 的审批流程：

```
Session 执行消息前
  ↓
Permission 模块检查是否需要审批
  ├── 需要审批 → 构造 PendingMessage → 推入 Session.pending 队列 → 等待用户审批
  │     ├── 审批通过 → mark_sent() → 继续执行消息
  │     └── 审批拒绝 → 从队列删除（remove）→ 丢弃消息
  └── 无需审批 → 正常执行消息
```

### ReasoningLevel

ReasoningLevel 的设置和消费路径：

```
/reasoning <等级> 指令
  ↓
ReasoningHandler 设置等级
  ↓
Gateway 写入 Session 的 reasoning_level 字段
  ↓
Gateway 在构造 LLM 请求时读取 reasoning_level
  ↓
传递给 LLM Provider（通过 provider-specific 参数，如 reasoning_effort）
  ↓
LLM 按指定推理深度产出响应
```

### PromptOverrides

PromptOverrides 在 system prompt 构建中的使用路径：

```
Agent 注册/配置时设置 PromptOverrides（各字段可选）
  ↓
Gateway 触发 system prompt 构建时传入 PromptOverrides
  ↓
SystemPromptBuilder 按优先级检查：
  1. override_prompt != None → 直接使用，跳过所有 section
  2. agent_prompt != None → 替换 agent 级 prompt
  3. custom_prompt != None → 在 agent 级 prompt 上叠加
  4. 全部 None → 正常 section 渲染流程
  ↓
产出最终 system prompt 字符串 → 写入 Session
```

### SessionCheckpoint

SessionCheckpoint 的持久化路径：

```
Session 创建 / 状态变更 / 停止
  ↓
Gateway 构造 SessionCheckpoint { session_id, agent_id, channel, status, last_activity }
  ↓
StorageProvider.save_checkpoint()
  ↓
持久化存储（SQLite / 文件系统）
  ↓
Session 恢复时：
  ↓
StorageProvider.load_checkpoint(session_id)
  ↓
重建 Session（恢复 agent_id, channel, status, last_activity）
```

### PersistResult

PersistResult 的处理路径：

```
StorageProvider 方法执行（save / load / delete / flush）
  ↓
返回 PersistResult
  ├── Success → 正常继续
  ├── PartialSuccess { warnings } → 记录警告日志，继续流程
  └── Failure(error) → Gateway 记录错误日志，尝试重试或降级
```

### UnifiedResponse / UnifiedUsage

UnifiedResponse 和 UnifiedUsage 的流动路径：

```
LLM Provider 原始响应
  ↓
Interpreter 统一转换
  → 提取 ContentBlock[]
  → 提取 token 用量填充 UnifiedUsage
  → 提取 finish_reason
  ↓
UnifiedResponse { content_blocks, usage: UnifiedUsage, finish_reason }
  ↓
Gateway 接收 → Session 将消息追加到对话历史
  ↓
ContentBlock[] 进入出站 Processor Chain
  ↓
UnifiedUsage 由 Gateway 累计 → 触发压缩决策 / 计费统计
```

## 模块关系

### NormalizedMessage

- **生产者**：IM Adapter 各平台插件（入站解析）——包括飞书、Discord、Telegram 等 IM 平台的 Adapter，以及 CLI 模块的 TerminalAdapter
- **消费者**：Processor Chain 入站（读取 NormalizedMessage 做内容标准化和 session_key 计算，产出 [ProcessedMessage](#processedmessage)）
- **无关**：LLM Provider（不接触 NormalizedMessage，只消费 ContentBlock[]）、Session（通过 Gateway 间接消费路由字段，不直接接触 NormalizedMessage）、Slash Command（斜杠指令不涉及 NormalizedMessage 结构）

### ContentBlock

- **生产者**：Session（LLM 对话产出 UnifiedResponse，含 ContentBlock[]）、SlashDispatcher（斜杠指令回复以 SlashResult 变体产出 ContentBlock[]）、Processor Chain 入站 ContentNormalizer（入站方向包装标准化文本为 ContentBlock::Text 放入 ProcessedMessage.content_blocks）
- **消费者**：Processor Chain 出站（VerbosityFilter → DslParser → OutboundRawLog）→ IM Adapter（按块类型渲染为平台原生格式并发送）
- **无关**：IM Adapter 入站链（入站方向产 NormalizedMessage，不涉及 ContentBlock[]）、Session 生命周期管理（不直接操作 ContentBlock[]，仅通过 Gateway 间接消费）、LLM Provider（LLM 调用产出 ContentBlock[]，但不参与 ContentBlock 的结构定义和处理流程）、[Gateway](../gateway/README.md)（Gateway 编排 Processor Chain 调度，不直接执行内容过滤/解析）

### DslParseResult / DslInstruction

- **DslParseResult 生产者**：Processor Chain 出站（DslParser 解析 ContentBlock::Text 中的 DSL 指令行，产出 DslParseResult）
- **DslParseResult 消费者**：IM Adapter 各平台 Renderer（读取 DslParseResult 中的 DslInstruction 列表，渲染为平台交互元素）、CLI TerminalRenderer（将 button/selector 转为纯文本提示行）
- **DslInstruction 生产者**：Processor Chain 出站（DslParser 逐行解析 DSL 指令，每条产出一个 DslInstruction）
- **DslInstruction 消费者**：IM Adapter 各平台 Renderer（按 instruction_type 选择渲染策略）
- **无关**：Processor Chain 入站（DSL 解析仅在出站方向执行）、IM Adapter 入站链（入站方向不涉及 DSL）、LLM Provider（LLM 不感知 DSL）、Session（Session 不操作 DslParseResult）

### ProcessedMessage

- **生产者**：Processor Chain 入站（ContentNormalizer 包装标准化文本为 ContentBlock::Text + SessionRouter 写 session_key 到 metadata）、Processor Chain 出站（DslParser 处理 ContentBlock[] + 写 dsl_result 到 metadata）
- **消费者**：Gateway（入站：消费 content_blocks + metadata.session_key 做路由决策；出站：消费 content_blocks + metadata.dsl_result 做出站日志后传给 IM Adapter）、IM Adapter（消费 content_blocks + metadata.dsl_result 渲染为平台格式并发送）、CLI TerminalRenderer（同 IM Adapter，渲染为 ANSI 终端文本）
- **无关**：NormalizedMessage（入站方向的上游产物，经 Processor Chain 处理后产出 ProcessedMessage，两者是不同的两个结构）、Session（Gateway 通过 ProcessedMessage 中的 session_key 找到 Session，但 Session 不直接操作 ProcessedMessage）、LLM Provider（不接触 ProcessedMessage，只产出 ContentBlock[]）

### SlashResult

- **生产者**：SlashDispatcher（各 Handler 返回 SlashResult 变体）
- **消费者**：Gateway（构造 SideEffectContext 并触发 SlashResult 执行，回复内容进入出站 Processor Chain）
- **间接消费者**：Permission 模块（Exec 变体执行前校验）、CLI（通过 Gateway 间接消费斜杠指令回复）
- **无关**：LLM Provider（不参与斜杠指令，不接触 SlashResult）、Processor Chain 入站（斜杠指令不进入站 Processor Chain）、Session（SlashResult 通过 SideEffectContext 操作 Session，但 Session 不直接消费 SlashResult 结构）

### FragmentContext

- **生产者**：system_prompt 模块（System Prompt Builder 构建）
- **消费者**：所有 PromptFragmentProvider 实现者（system_prompt / tools / skills / memory）
- **无关**：LLM Provider（不接触 FragmentContext）、Processor Chain（不参与 system prompt 构建）

### PromptFragment

- **生产者**：所有 PromptFragmentProvider 实现者（system_prompt / tools / skills / memory）
- **消费者**：system_prompt 模块（System Prompt Builder 收集所有 Fragment 并按序拼接）
- **无关**：LLM Provider（不接触 PromptFragment，消费的是拼接后的最终 system prompt 文本）、Session（Builder 写入 system prompt 字段，Session 不直接操作 PromptFragment）

### RenderedOutput

- **生产者**：IM Adapter 各平台 Renderer（IMPlugin.render() 产出）
- **消费者**：Gateway（中间件插入点，在渲染和发送之间）；IM Adapter（IMPlugin.send() 消费 payload 发送）
- **无关**：Processor Chain（RenderedOutput 在 Processor Chain 之后产出，不经过链处理）、LLM Provider（不接触 RenderedOutput）

### VerbosityLevel

- **生产者**：slash 模块（VerboseHandler 处理 `/verbose` 指令，写入 Session）
- **消费者**：Processor Chain 出站（VerbosityFilter 读取并过滤 ContentBlock[]）；Session（存储当前等级，供下次出站过滤）
- **无关**：LLM Provider（Verbosity 不影响 LLM 推理，仅控制展示）、IM Adapter 入站（入站不涉及展示过滤）

### PlanState

- **生产者**：mode 模块（Plan Mode 进入时创建）
- **消费者**：Session（持久化和 compaction 保护）；mode 模块（恢复时重建、阶段切换时更新）
- **无关**：LLM Provider（PlanState 不直接传给 LLM，通过 system prompt 的 plan 上下文间接生效）、IM Adapter（消息路由不感知 PlanState）

### CompactConfig

- **生产者**：Config 模块（从配置文件加载 CompactConfig）
- **消费者**：Compaction 模块（读取配置确定压缩触发条件和估算参数）；Session（在 compaction 触发时通过管理器间接引用）
- **无关**：LLM Provider（压缩配置不影响 LLM 调用）、IM Adapter（不参与压缩决策）

### PendingMessage

- **生产者**：Permission 模块（拦截需要审批的消息时构造 PendingMessage，推入 Session 队列）
- **消费者**：Session（存储 pending 队列，供用户审批）；Gateway（通过 SessionLookup trait 读取 pending 消息做审批处理）
- **无关**：LLM Provider（PendingMessage 在 LLM 调用之前流转，LLM 不感知审批流程）、IM Adapter（不参与审批流程）

### ReasoningLevel

- **生产者**：slash 模块（ReasoningHandler 处理 `/reasoning` 指令，写入 Session）
- **消费者**：Gateway（构造 LLM 请求时读取推理等级并传递给 LLM Provider）；Session（持久化存储当前等级）
- **无关**：IM Adapter（不感知推理深度）、Processor Chain（ReasoningLevel 在 LLM 调用前已消费，不出现在入站/出站处理链中）

### PromptOverrides

- **生产者**：Agent 注册/配置（各 Agent 设置自己的 PromptOverrides）
- **消费者**：system_prompt 模块（SystemPromptBuilder 接收 PromptOverrides，按优先级选择覆盖策略）
- **无关**：LLM Provider（不接触 PromptOverrides 结构，消费的是构建后的最终 prompt 文本）、Processor Chain（不参与 system prompt 构建）

### SessionCheckpoint

- **生产者**：Gateway（Session 创建、状态变更、停止时构造 checkpoint）
- **消费者**：StorageProvider（持久化保存和加载 checkpoint）；Gateway / SessionManager（从存储恢复时反序列化重建 Session）
- **无关**：LLM Provider（不接触 checkpoint 结构）、IM Adapter（不参与 session 持久化）

### PersistResult

- **生产者**：StorageProvider 所有方法（save_checkpoint / load_checkpoint / delete_checkpoint / list_checkpoints / flush 均返回 PersistResult）
- **消费者**：Gateway / SessionManager（消费 PersistResult 判断操作成功与否，做日志记录或重试）
- **无关**：LLM Provider（不与持久化层直接交互）、IM Adapter（不参与持久化）

### UnifiedResponse / UnifiedUsage

- **生产者（UnifiedResponse）**：LLM 模块的 Interpreter（将各 LLM Provider 原始响应统一转换为 UnifiedResponse）
- **生产者（UnifiedUsage）**：LLM 模块的 Interpreter（从 Provider 原始响应中提取 token 统计数据填充 UnifiedUsage）
- **消费者（UnifiedResponse）**：Gateway（接收 UnifiedResponse，提取 ContentBlock[] 进入出站链路，提取 UnifiedUsage 做用量统计）；Session（将 UnifiedResponse 的消息内容追加到对话历史）
- **消费者（UnifiedUsage）**：Gateway（累计 token 用量，触发压缩决策）；计费/监控系统（消费 token 统计数据）
- **无关**：IM Adapter（不接触 UnifiedResponse 和 UnifiedUsage，消费的是出站链处理后的 ContentBlock[]）
