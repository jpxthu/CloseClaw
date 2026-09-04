# 共享类型

## 概述

共享类型是跨模块传递的纯数据结构，被 2 个及以上模块共同消费。每个共享类型在本文档中唯一定义，各业务模块文档通过引用指向此处，不在自身文档中重复描述字段结构。

> **本文档是 common crate 中共享类型的权威清单。** 判定规则与 [STANDARDS.md「common 文档内容准入标准」](../STANDARDS.md)一致：被 2 个及以上模块消费的类型，在此完整定义、代码留 common crate；仅单模块消费的类型不在本清单——代码中如出现在 common crate，应移至对应领域模块的 crate。反之，本文档定义的所有类型，代码中均位于 common crate（或其子 crate）。

本文档不包含 trait 接口定义——核心 trait 见 [core-traits](core-traits.md)。

## 架构

### NormalizedMessage

NormalizedMessage 是平台无关的统一入站消息结构，屏蔽各 IM 平台（飞书、Discord、Telegram 等）和 terminal 渠道的差异。各渠道的 IM Adapter 入站解析产出此结构，Processor Chain 消费（读取内容做标准化和 session_key 计算）。Gateway 消费的是 Processor Chain 产出的 ProcessedMessage，不直接接触 NormalizedMessage。

| 字段 | 类型 | 说明 |
|------|------|------|
| `platform` | string | 平台标识，如 `"feishu"`、`"terminal"` |
| `sender_id` | string | 发送者的平台内 ID |
| `peer_id` | string | 会话对端——会话上下文锚点，由插件按平台语义构造，同一会话内取值稳定、不同会话间互不相同（如私聊的「对方用户 + 话题」组合、群聊的群 ID） |
| `reply_ref` | string? | 出站定向引用，可选。插件按平台语义填入、出站时原样消费的平台引用（如话题根消息标识），用于把回复投递回原会话位置。不参与 session 路由。出站传递机制：入站填入后经 Session 上下文存储，出站时由 Gateway 取出传给 IMPlugin 发送，见 [session-lifecycle 出站定向字段](../session/session-lifecycle.md) |
| `account_id` | string | CloseClaw 本地账号标识，由「平台 + 接收方机器人应用 + sender_id」经身份映射得到。参与 session 路由 |
| `content` | string | 消息文本内容。纯媒体消息时可为空 |
| `message_type` | enum | 消息类型：text / image / file / audio / post。post 为含内嵌媒体的富文本消息——展开文本入 content，内嵌媒体入 media_refs |
| `media_refs` | list(MediaRef) | 消息携带的媒体引用列表。Adapter 在入站解析时完成媒体落盘并填充，下游一律以引用消费，不接触平台下载地址与凭证。落盘与消费机制见 [im_adapter media-store](../im_adapter/media-store.md) |
| `unavailable_media` | list(string) | 消息引用但未能获得的媒体资源标识列表（下载失败或超出大小上限）。由 Adapter 在入站解析时填充；失败媒体不进入 media_refs、仅记录于此。非空时 Gateway 按媒体不可得处理（提示用户、不进入对话），见 [im_adapter media-store](../im_adapter/media-store.md) |
| `timestamp` | int | 消息发送时间（毫秒级 Unix 时间戳） |

**机器人身份（app_id）**：机器人自身标识（app_id）不属于 NormalizedMessage 字段。IM Adapter 在入站解析时单独提取 app_id，不经归一化结构传递，直接交给 Gateway 用于 Agent 路由（选择处理该机器人消息的 Agent）。

**引用/回复消息处理**：IM Adapter 在解析被引用的消息时，将其内容渲染为 markdown blockquote（`> 引用内容`），截断至 500 字符（超出追加 `...`），拼接在 `content` 字段之前（对 text 与 post 消息均适用）。不传递独立的引用消息字段——LLM 在对话文本中直接看到 blockquote。

**消息过滤规则**：text 类型空 content 消息在解析阶段丢弃，不产生 NormalizedMessage；post 类型 content 与 media_refs 均为空时同样丢弃。其余消息正常产 NormalizedMessage（message_type 标记类型，media_refs 承载已落盘的媒体引用，纯媒体消息 content 可为空），由 Gateway 分型处理——媒体可得时按上下文形态进入对话，不可得时提示用户（详见 [im_adapter media-store](../im_adapter/media-store.md) 与 [gateway 入站流程](../gateway/inbound-flow.md)）。

**身份映射**：`account_id` 由 IM Adapter 在解析入站消息时填入。与其他字段（platform、sender_id 等直接从消息 payload 提取）不同，account_id 需查询账户绑定表获取，非直接取值。映射规则：以「平台 + 接收方机器人应用 + sender_id」为键查询账户绑定表，找到对应的 CloseClaw 账户 ID——IM 平台的发送者标识按「应用 × 发送者」隔离，跨应用标识不可直接互换，故映射键必须包含接收方机器人应用。一个账户可绑定多个平台的多个发送者标识。terminal 平台恒为 "owner"，无需查表。详见 [config 模块 accounts.json](../config/README.md)。

**字段填充职责**：各字段由 IM Adapter 入站解析时填充。Processor Chain 不修改 NormalizedMessage 字段——ContentNormalizer 读取 message_type 判断消息类型，仅对 text 类型做 content 文本标准化；SessionRouter 读取 platform/sender_id/peer_id/account_id 计算 session_key。Processor Chain 各 Processor 通过共享的可变 ProcessedMessage 上下文传递数据：SessionRouter 计算 session_key 后直接写入 ProcessedMessage.metadata，ContentNormalizer 随后从同一 NormalizedMessage 读取 content 做标准化后写入 ProcessedMessage.content_blocks。session_key 不写入 NormalizedMessage。

**message_type 与 media_refs**：message_type 由 ContentNormalizer 消费（非 text 跳过标准化）。media_refs 在入站链路仅透传——媒体已由 Adapter 落盘，上下文形态决策由 Gateway 在路由阶段完成（见 [gateway 入站流程](../gateway/inbound-flow.md)）。

NormalizedMessage 引用的子结构：

**MediaRef**：媒体资源的本地存储引用，由 IM Adapter 在入站解析落盘后填充，是下游消费媒体的唯一形态。

| 字段 | 类型 | 说明 |
|------|------|------|
| `key` | string | 平台内资源标识（如飞书 image_key / file_key），用于关联与幂等 |
| `path` | string | 媒体存储中的本地文件路径（相对媒体存储根目录），文件名经安全净化并附加唯一后缀 |
| `media_type` | enum | image / file / audio |
| `size` | int | 文件大小（字节） |
| `mime` | string | MIME 类型 |

`key` 与出站 [ContentBlock](#contentblock) 非文本变体的 `name` 均表示资源标识，语义等价——命名差异源于入站（MediaRef）与出站（ContentBlock）两套独立结构。落盘、上下文形态与生命周期机制见 [im_adapter media-store](../im_adapter/media-store.md)。

**建模边界**：NormalizedMessage 建模用户主动发送的消息（文本、图片、文件、音频）。卡片交互事件——用户点击消息中嵌入的按钮、选择器等交互控件——属于工具调用的回执，走 tool_result 通道注入对话，不经过 NormalizedMessage 入站通路。各 IM 平台在 Adapter 解析阶段须区分消息事件和交互事件，仅将消息事件转为 NormalizedMessage。卡片交互事件的载荷结构为 [CardActionEvent](#cardactionevent)，平台解析阶段的识别规则见 [im_adapter feishu](../im_adapter/platforms/feishu.md)（事件区分段落）。

#### CardActionEvent

CardActionEvent 是用户与消息内嵌交互控件（按钮、选择器等）交互产生的事件载荷。Adapter 识别后将动作值作为工具调用回执经 tool_result 通道注入对话，不进入 NormalizedMessage 入站链路，也不经过入站 Processor Chain。

| 字段 | 类型 | 说明 |
|------|------|------|
| `platform` | string | 平台标识，如 `"feishu"` |
| `sender_id` | string | 触发交互的用户在平台内的 ID |
| `action_value` | string | 交互控件的回传值，即被触发动作的内容 |
| `metadata` | map(string→string) | 平台附加信息（如卡片 ID、动作标签），可为空 |
| `timestamp` | int | 事件发生时间（毫秒级 Unix 时间戳） |
| `account_id` | string | CloseClaw 本地账号标识，用于多租户会话隔离，可为空 |

`account_id` 为可选的原因：部分平台交互事件不携带租户/账号上下文，此时留空；填值时的解析方式与会话隔离语义同 [NormalizedMessage §身份映射](#normalizedmessage)。

### ContentBlock

ContentBlock 是跨模块传递的结构化内容单元。所有出站内容——LLM 回复和斜杠指令回复——均以 ContentBlock[] 数组形式传递，贯穿 Verbosity 过滤、DSL 解析、出站日志记录和平台渲染全链路。入站方向经 Processor Chain 处理后，标准化文本以 ContentBlock::Text 形式放入 [ProcessedMessage](#processedmessage) 的 content_blocks 字段，入站不涉及 ContentBlock 的其他变体。流式场景下，同一份内容以 [StreamEvent](#streamevent) 增量事件形式传递，完整块由消费方按事件边界组装。

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

Image/Audio/File 三个变体结构相同，字段定义：

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | 资源标识，终端占位符 `[image: name]` 等引用此字段 |
| `url` | string | 资源访问地址，IM 平台渲染时使用 |

**变体处理规则**：

- **Text 是唯一可能包含 DSL 指令的变体**。DslParser 仅遍历 Text 块逐行扫描 DSL，解析后从 Text 块中移除 DSL 行。其余 6 种变体由 DslParser 透传
- **流式渲染差异化**：Text 块逐行缓冲输出（以句末标点或换行符为行边界）；Thinking/ToolUse/ToolResult 块等待全块就绪后一次交付渲染；Image/Audio/File 不以流式事件形式出现，在非流式路径中交由平台格式渲染器处理
- **输出格式决策**：各平台 Renderer 按内容特征选择输出格式（纯文本 vs 富格式），完整规则见 [RenderedOutput §输出格式决策](#renderedoutput)
- **Verbosity 过滤**以单个 ContentBlock 为粒度执行——每个 ContentBlock 到达时按当前 Session 的 verbosity 等级判断其可见性，流式模式下逐块实时过滤。Verbosity 等级定义见 [slash 模块 verbose 指令](../slash/verbose.md)

DslParseResult 是 DslParser 解析 ContentBlock::Text 中 DSL 指令行的输出结果。存储在 [ProcessedMessage](#processedmessage) 的 metadata 中。批量模式下供下游 Renderer 消费（渲染为平台交互元素）；流式模式下仅用于日志记录和出站历史写入，不产生渲染输出。DslInstruction 是单条 DSL 指令的结构化表示。

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

### StreamEvent

StreamEvent 是流式输出的统一增量事件，ContentBlock 的流式形态——描述一条消息在生成过程中的边界变化。LLM 模块将各协议 SSE 事件归一化为 StreamEvent 后逐事件对外交付；流式链路上的消费方（VerbosityFilter、DslParser 透传、流式渲染器）以事件流为输入，实现「块未结束即逐行输出」的增量行为。

StreamEvent 共 5 种事件：

| 事件 | 载荷 | 语义 |
|------|------|------|
| BlockStart | `index`、`block_type` | 内容块开始。开启一个 ContentBlock 边界，携带块序号和块类型（正常出现的变体：Text/Thinking/ToolUse；ToolResult 属预留不出现，原因见 [ContentDelta §无生产者的预留变体](#contentdelta)；Image/Audio/File 不以流式事件形式出现） |
| BlockDelta | `index`、[ContentDelta](#contentdelta) | 内容增量。携带当前块的增量载荷 [ContentDelta](#contentdelta)，一个块内可有任意多个增量 |
| BlockEnd | `index`、`block_type` | 内容块结束。该块内容已完整，块级消费方据此判定全块就绪 |
| MessageEnd | `usage`（Optional [UnifiedUsage](#unifiedresponse--unifiedusage)）、`finish_reason` | 消息结束。携带结束原因（如 stop / length / 工具调用）与最终用量（见 [UnifiedResponse §UnifiedUsage](#unifiedresponse--unifiedusage)），此后不再有事件 |
| Error | `message` | 错误。流式调用失败，流终止 |

**事件与块的关系**：一个 `BlockStart → 若干 BlockDelta → BlockEnd` 序列重组出一个完整的 ContentBlock；消息级完整 ContentBlock[] 由消费方按 BlockEnd 边界累积组装。`index` 标识块在本次响应内的序号，供增量消费方把 BlockDelta 归属到正确的事件序列。典型事件顺序（文本 + 工具调用混合响应）：Thinking 块序列 → Text 块序列 → ToolUse 块序列 → MessageEnd。

**媒体增量约束**：BlockDelta 的 ContentDelta 含 ImageRef/AudioRef/FileRef 变体是为结构完备预留——当前 LLM 协议不对媒体内容产生流式增量，正常链路不会出现这三种增量事件；媒体块仅在非流式路径按完整 ContentBlock 处理。

**消费契约**：
- 增量消费方按事件流逐事件处理，不等待完整块——Text 块的逐行渲染依赖 BlockDelta 携带的文本片段
- 以完整块为处理粒度的消费方（Verbosity 过滤、Thinking/Tool 整块渲染）按块边界（BlockStart/BlockEnd）判定作用对象，等待 BlockEnd 后一次处理
- 事件流的协议归一化规则（OpenAI/Anthropic SSE → StreamEvent）由 LLM 模块定义，见 [llm protocol-mapping](../llm/protocol-mapping.md)

#### ContentDelta

ContentDelta 是单个 ContentBlock 内部的增量载荷，BlockDelta 事件的载体。9 种变体与所归属的块类型一一对应：

| 变体 | 字段 | 归属块类型 |
|------|------|-----------|
| Text | `text`（文本片段） | Text |
| Thinking | `thinking`（思考片段）、`signature`（签名，可选） | Thinking |
| ToolUseId | `id`（工具调用标识） | ToolUse |
| ToolUseName | `name`（工具名） | ToolUse |
| ToolUseInputChunk | `input`（参数 JSON 片段） | ToolUse |
| ToolResultText | `text`（结果文本片段） | ToolResult（预留变体：工具结果是下一轮请求输入而非响应流产物，正常链路不出现此增量，见下方约束说明） |
| ImageRef / AudioRef / FileRef | `name`（资源标识）、`url`（资源访问地址） | Image / Audio / File（当前协议不产出，见媒体增量约束） |

逐个增量的归属由同一事件的块类型和 `index` 确定；完整块的组装由消费方按上述合并规则执行（LLM 侧归 Session 的事件组装；渲染侧的行缓冲与交付节奏见 [im_adapter streaming-render](../im_adapter/streaming-render.md)）。

**同块多增量的合并规则**：消费方将多个增量的载荷按变体拼接重组——Text/ToolResult 依次追加文本片段；ToolUse 按 id → name → input 分字段填充；Thinking 追加思考片段，签名只在首个携带签名的增量处设置一次，后续空签名增量不覆盖已有值。

**无生产者的预留变体**：LLM 响应流只包含模型生成的内容增量（文本、思考、工具请求）；工具结果由系统执行后进入下一轮请求，媒体引用走非流式路径。因此 ToolResultText 与三个媒体增量变体在正常事件流中不会出现，保留它们是为结构完备与协议扩展预留位。

### UnifiedResponse / UnifiedUsage

UnifiedResponse 是各 LLM Provider 非流式调用的统一响应结构。Session 在每次 LLM 对话后收到 UnifiedResponse，其中的 ContentBlock[] 进入出站处理链路（与 SlashResult 回复共用出站路径）。各供应商协议的响应映射为 UnifiedResponse 的规则由 LLM 模块定义，见 [llm protocol-mapping](../llm/protocol-mapping.md)；LlmCaller trait 以 UnifiedResponse 为非流式调用的返回类型，trait 定义见 [core-traits LlmCaller](core-traits.md#llmcaller)。

| 字段 | 类型 | 说明 |
|------|------|------|
| `content_blocks` | ContentBlock[] | 按序排列的回复内容块，变体沿用 [ContentBlock](#contentblock) 定义 |
| `usage` | [UnifiedUsage](#unifiedresponse--unifiedusage) | 本次请求的 token 用量统计 |
| `finish_reason` | string? | 结束原因，如 `"stop"`、`"length"`。可选，协议未给出时为空 |
| `retry_attempts` | int | 本响应成功前的重试次数，默认 0 |

**UnifiedUsage** 是 UnifiedResponse 与 StreamEvent::MessageEnd 共用的 token 用量统计结构：

| 字段 | 类型 | 说明 |
|------|------|------|
| `prompt_tokens` | int | 输入 token 数 |
| `completion_tokens` | int | 输出 token 数 |
| `total_tokens` | int? | 总 token 数，可选。协议未给出时由消费方按需自行合计 |
| `reasoning_tokens` | int? | 推理过程消耗的 token 数，协议提供时才有值 |
| `cache_read_tokens` | int? | 缓存命中的输入 token 数，协议提供时才有值 |
| `cache_write_tokens` | int? | 写入缓存的 token 数，协议提供时才有值 |

注意 UnifiedUsage 只承载单次调用的原始计数。用户可见的派生指标——缓存命中率百分比、跨轮次累计等——由 [RunningStats](#runningstats--cachebreakinfo--cachebreakthresholds) 累计计算；预估费用等需要模型定价信息的指标不属于本结构的职责（定价知识在 LLM 模块）。

### RunningStats / CacheBreakInfo / CacheBreakThresholds

跨轮次 LLM 用量的统计结构族。Session 持有 RunningStats，每次 API 调用完成后将当次 UnifiedUsage 累加进去；派生的缓存命中率供 `/status` 展示与缓存异常提醒，统计快照参与 compaction 阈值判断。行为细节（流式 MessageEnd 时更新、会话结束清零）定义于 [llm-session-enhancements](../session/llm-session-enhancements.md)，压缩对统计的读取时机见 [compact-process](../session/compact-process.md)，Session 概览见 [session](../session/README.md)；对 slash 的呈现见 [slash status](../slash/status.md)。

**RunningStats** 字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `total_prompt_tokens` | int | 所有调用累计的输入 token |
| `total_completion_tokens` | int | 累计输出 token |
| `total_tokens` | int | 累计总 token |
| `total_cache_read_tokens` | int | 累计缓存命中 token |
| `total_cache_write_tokens` | int | 累计缓存写入 token |
| `request_count` | int | 已累加的 API 调用次数 |
| `total_reasoning_tokens` | int | 累计推理 token |
| `cache_break_thresholds` | CacheBreakThresholds? | 自定义命中率下降判定阈值，空则用默认值 |
| `last_cache_read_tokens` | int? | 最近一次调用的缓存命中数，尚无调用时为空 |
| `last_cache_hit_rate` | float? | 最近一次调用的单次命中率，尚无调用时为空 |
| `last_cache_break` | CacheBreakInfo? | 最近一次命中率下降事件，未发生时为空 |

除原始累计外，RunningStats 还提供累计缓存命中率、累计节省 token 等派生指标的查询。累计口径：单次用量缺省 total_tokens 时按 prompt + completion 求和后累加；可空的缓存/推理 token 缺省按 0 计入累计。缓存命中率下降事件的判定基于相邻两次调用的缓存命中数对比（单次命中率 = 该次 cache_read_tokens ÷ prompt_tokens），自有前值的第二次调用起参与判定（首次调用无前值不触发）；仅当绝对降幅超过下限且相对降幅超过阈值比例时触发，产生一个 CacheBreakInfo（当前值不低于前值时不触发，天然规避除零）。「缺省按 0 计入」仅是累加口径不等于展示或告警依据——协议始终不携带缓存字段的供应商不参与命中率下降检测与告警（需求见 [llm §F9](../../requirements/llm.md)，行为细节见 [llm-session-enhancements](../session/llm-session-enhancements.md)）。

**CacheBreakThresholds** 字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `drop_ratio_threshold` | float | 触发下降事件的最小命中率降幅比例，默认 0.05 |
| `min_drop_tokens` | int | 启动比例比较所需的最小绝对 token 降幅，默认 2000 |

**CacheBreakInfo** 字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `previous_cache_read` | int | 上一次调用的缓存命中 token 数 |
| `current_cache_read` | int | 本次调用的缓存命中 token 数 |
| `drop_tokens` | int | 两次之间的绝对降幅 |
| `drop_ratio` | float | 相对上次命中数的降幅比例 |
| `previous_hit_rate` | float | 上一次调用的单次命中率 |
| `current_hit_rate` | float | 本次调用的单次命中率 |

CacheBreakInfo 可格式化为用户可读的命中率下降提示文本（含前后命中率对比、token 降幅与常见原因说明）。

### ProcessedMessage

ProcessedMessage 是 Processor Chain 的输出结构，Gateway 的消费入口。入站和出站方向共用同一结构，content_blocks 在不同方向携带不同复杂度的内容，metadata 携带方向相关的计算结果。

| 字段 | 类型 | 说明 |
|------|------|------|
| `content_blocks` | ContentBlock[] | 处理后的内容块数组。入站方向为单个 ContentBlock::Text（ContentNormalizer 标准化后的文本；非 text 消息跳过标准化，此格为原样内容的 Text 包装，后续由 Gateway 按媒体可得性分型路由，见下方数据流），出站方向为经 DslParser 处理后的 ContentBlock[]（Text 块已剥离 DSL 行，其余块透传） |
| `metadata` | map(string→string) | 方向相关的键值对。入站含 `session_key`（SessionRouter 计算的消息级标识）、`message_type`（来自原始 NormalizedMessage，由 Processor Chain 在构建 ProcessedMessage 时从 NormalizedMessage 复制，供 Gateway 做分型路由判断）和 `unavailable_media`（不可得媒体资源标识列表，JSON 序列化，同样复制，供 Gateway 做媒体可得性判断）；出站含 `dsl_result`（DslParser 产出的 DslParseResult，JSON 序列化）。metadata 字段的复制均发生在链调度构建 ProcessedMessage 时——各 Processor 不修改 NormalizedMessage 字段，复制不构成对消息的修改 |

入站和出站不区分类型——同一个 ProcessedMessage 结构，内容形态和 metadata 字段按方向不同而不同。

### SlashResult

SlashResult 是斜杠指令 Handler 返回的执行结果类型。每个变体封装一种指令的副作用逻辑。Handler 返回 SlashResult 后，由 Gateway 构造 SideEffectContext 并触发 SlashResult 执行，各变体自行完成对应的 session 操作和消息回复。

SlashResult 共 10 种变体：

| 变体 | 用途 | 产出 |
|------|------|------|
| SetMode | 设置会话运行模式（Normal/Plan/Auto） | ContentBlock::Text（确认信息） |
| SetReasoning | 设置推理深度 | ContentBlock::Text（确认信息） |
| SetVerbosity | 设置信息展示等级 | ContentBlock::Text（确认信息） |
| Reply | 纯文本回复，用于 /help、/status 等仅需回复文本的指令 | ContentBlock::Text（回复文本） |
| NewSession | 创建新会话 | ContentBlock::Text（确认信息） |
| Stop | 终止当前运行（含级联终止子 session） | ContentBlock::Text（确认信息） |
| Compact | 触发对话历史压缩 | ContentBlock::Text（压缩结果） |
| SystemAppend | 向 system prompt 追加内容 | ContentBlock::Text（确认信息） |
| Exec | 执行系统命令（高危操作，执行前经 Permission 模块校验） | ContentBlock[]（命令输出经出站 Processor Chain） |
| Unknown | 未知指令回退 | ContentBlock::Text（提示信息） |

**执行模型**：Handler 返回 SlashResult 后，Gateway 统一调用执行方法，由各变体自行完成副作用。高危指令（Exec、Git 写操作）的权限校验由 Gateway 在触发执行前经 Permission 引擎完成（校验通过方继续，拒绝则返回权限错误），不属于变体自身副作用。新增指令只需新增 SlashResult 变体及其执行实现，Gateway 无需改动。

**边界**：SlashResult 仅由 SlashDispatcher 分派的斜杠指令 Handler 产出。审批指令（`/approve-once`、`/approve-whitelist`、`/deny`）由 Gateway 层硬拦截、走权限审批流验证，不进 SlashDispatcher，其审批结果不属于 SlashResult（详见 [permission 审批工作流](../permission/approval-workflow.md)）。权限管理指令（如 `/perm register`）同样不产出 SlashResult，由 Gateway 权限指令处理层硬拦截执行——新用户注册的载荷结构见 [UserRegistration / UserCreationRequest / InitialPermissionSet](#userregistration--usercreationrequest--initialpermissionset)。

**SideEffectContext**：Gateway 在收到 SlashResult 后构造的执行上下文。携带当前 Session 的操作能力（用于模式切换、会话创建/停止、压缩等操作）和回复通道（用于产出回复内容）。SideEffectContext 由 Gateway 管理，SlashResult 不持有其引用。

| 字段 | 类型 | 说明 |
|------|------|------|
| `session_id` | string | 斜杠指令所在的会话 ID |
| `channel` | string | 渠道标识（如 `"feishu"`、`"terminal"`） |
| `session_lookup` | SessionLookup | 会话状态查询接口（见 [core-traits SessionLookup](core-traits.md#sessionlookup)） |
| `reply_tx` | 回复通道 | ReplyAction 通道，SlashResult 执行时回发回复内容 |
| `executor` | SlashEffectExecutor | 斜杠指令副作用执行接口（见 [core-traits SlashEffectExecutor](core-traits.md#slasheffectexecutor)） |

**与 ContentBlock[] 的关系**：SlashResult 各变体在执行中通过 SideEffectContext 的回复通道产出 ContentBlock[]，进入出站 Processor Chain——与 LLM 的 UnifiedResponse 走同一条出站处理路径（VerbosityFilter → DslParser → OutboundRawLog → IM Adapter 渲染发送）。

#### UserRegistration / UserCreationRequest / InitialPermissionSet

新用户注册工作流的三个数据结构：注册结果记录、待审批请求、预置权限集。三者由 Gateway 权限指令处理层构造，由 permission 侧消费落为权限规则与用户记录；slash 指令层仅是参数入口，不经 SlashDispatcher 分派。用户注册的需求背景（新建 User 默认无任何权限，收发消息也需 Owner 显式授予）见 [permission 需求 §F1](../../requirements/permission.md)。结构流转路径见下文数据流节；审批队列的去重与请求 ID 回调机制以工具调用审批为背景定义于 [permission 审批工作流](../permission/approval-workflow.md#审批队列)。

**UserRegistration**——已通过审批的注册用户的记录：

| 字段 | 类型 | 说明 |
|------|------|------|
| `user_id` | string | 用户唯一标识（如飞书 open_id） |
| `im_channel` | string | 用户使用的 IM 渠道，如 `"feishu"` |
| `initial_permissions` | list(InitialPermissionSet) | 注册时授予的预置权限集 |
| `created_at` | string | 注册时间（ISO-8601 时间戳） |

**UserCreationRequest**——需 Owner 审批的新用户注册请求（经审批队列流转）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `user_id` | string | 发起注册的用户标识 |
| `im_channel` | string | 将使用的 IM 渠道 |
| `request_id` | string | 审批队列中的唯一请求 ID，用于关联审批回调 |
| `initial_permissions` | list(InitialPermissionSet) | 请求携带的预置权限集候选：入队时由 Gateway 按指令参数构造，Owner 审批时确认或调整；为空表示注册后不授予任何规则（与零权限默认一致） |

**InitialPermissionSet**——Owner 可授予新注册用户的预置权限集枚举。每个变体映射为一组具体权限规则（如 BasicMessaging 对应收发消息 + workspace 读）。当前仅有 `BasicMessaging` 一个变体；新增预设按同样方式扩展映射规则。预设集是 Owner 在审批时**显式选择**的选项而非注册默认——需求要求新建 User 默认无任何权限（含收发消息），无预置权限集时注册后的 User 不获得任何规则，与 [permission 需求 §F1](../../requirements/permission.md) 的零权限默认一致。

### FragmentContext

FragmentContext 是 PromptFragmentProvider 片段生成时的输入上下文，由 System Prompt Builder 构建后传递给各 Provider。

| 字段 | 类型 | 说明 |
|------|------|------|
| `agent_id` | string | Agent 标识。Skills 按此过滤可见 skill |
| `bootstrap_mode` | enum | BootstrapMode::Minimal（精简模式）或 BootstrapMode::Full（完整模式），Bootstrap 按此选择文件集合 |
| `bootstrap_dir` | string | bootstrap 文件所在目录，BootstrapFragmentProvider 按此查找 bootstrap 文件。值来源于 agent 配置的 agentDir 字段 |

### PromptFragment

PromptFragment 是单个 PromptFragmentProvider 产出的静态层片段。

| 字段 | 类型 | 说明 |
|------|------|------|
| `section_title` | string | Section 标题，如 `## AGENTS.md`、`## Available Skills` |
| `section_type` | enum | Section 类型：bootstrap 文件、工具列表、skill 清单、长期记忆 |
| `content` | string | 渲染完成的文本内容 |

### RenderedOutput

RenderedOutput 是 IMPlugin 渲染方法产出的平台原生格式消息结构。渲染产出数据，发送执行副作用——Gateway 在两步之间插入中间件（审计、频率限制等）。流式场景下渲染以增量方式进行：StreamingRenderer 每处理完一批事件产出一个 StreamingOutput（见 [core-traits StreamingRenderer](core-traits.md#streamingrenderer)），平台将其组装为整条 RenderedOutput 后发送，不再单独定义平台消息结构。

| 字段 | 类型 | 说明 |
|------|------|------|
| `msg_type` | string | 消息格式类型（如 `"text"`、`"interactive"`），由 Renderer 按内容特征选择 |
| `payload` | any | 平台原生格式的消息体，结构由各平台 Renderer 定义。Gateway 中间件和 Adapter 发送不解析 payload 内容 |

**输出格式决策**：由各平台 Renderer 按内容特征选择输出格式，规则详见 [IM Adapter §平台渲染选择](../im_adapter/README.md#平台渲染选择)。大致原则：纯文本、无格式标记、无 DSL → `"text"`；含 markdown 格式/换行/DSL/Thinking/ToolUse/ToolResult 块 → `"interactive"`。终端渠道例外：terminal 渠道无富格式消息形态，RenderedOutput 恒为 `"text"`——富内容（Thinking/工具块、DSL）已在 payload 内转为 ANSI 样式文本（见 [cli/Terminal Renderer](../cli/renderer.md)）。

#### StreamingOutput

StreamingOutput 是流式渲染过程中单批事件的处理产出：本批投递的文本内容列表（完整文本行或强制输出时的行内片段），加本批内累积完整的非文本块。被 common 内 IMPlugin trait 的流式方法签名直接引用，gateway 在流式出站管线中消费——满足共享类型准入条件。

| 字段 | 类型 | 说明 |
|------|------|------|
| `text_messages` | list(string) | 本批输出的文本内容：行边界达成的完整文本行，或触发强制输出（缓冲超阈值/超时）时的行内片段。缓冲与阈值规则见 [im_adapter streaming-render](../im_adapter/streaming-render.md) |
| `render_blocks` | ContentBlock[] | 本批内累积完整的非文本块（Thinking/ToolUse/ToolResult），等待全块就绪的渲染策略在此交付 |

StreamingOutput 是渲染过程的中间产物，生命周期止于本次流式发送完成，不进入 Session 或日志持久化。行缓冲和分批规则见 [im_adapter streaming-render](../im_adapter/streaming-render.md)。

### VerbosityLevel

VerbosityLevel 是出站信息展示等级的枚举，控制 VerbosityFilter 对 ContentBlock 的过滤策略。由 `/verbose` 指令设置，Session 存储，出站 Processor Chain 的第一道过滤（VerbosityFilter，priority 5）消费。

三个等级：

| 等级 | 值 | 过滤行为 |
|------|---|---------|
| full | `"full"` | 展示全部：思考过程、工具调用、工具结果、最终回复 |
| normal | `"normal"` | 展示工具调用和结果作为进度提示，隐藏思考过程 |
| off | `"off"` | 仅展示最终回复，隐藏所有中间过程 |

**作用范围**：Verbosity 控制展示内容，不影响 LLM 推理深度和 Agent 行为模式。仅有 `/verbose` 指令通过 VerboseHandler 写入 Session 的 Verbosity 字段，无其他写入者。切换等级不影响当前正在输出的消息——仅对后续新消息生效。非文本媒体块（Image/Audio/File）属于最终回复的一部分，不受 VerbosityLevel 过滤——在所有等级下均展示。

### PlanState

PlanState 是 Plan Mode 下的规划状态结构，由 mode 模块管理，Session 持久化。Compaction 对此状态做隔离保护（不压缩 plan 相关消息），Session 恢复时重建 PlanState。

PlanState 描述当前规划的阶段和未完成步骤列表：

| 字段 | 类型 | 说明 |
|------|------|------|
| `phase` | enum | 当前阶段：Research / Design / Review / FinalPlan / Interview |
| `pending_steps` | list(string) | 未完成的规划步骤标识列表，用于 compaction 保护和恢复后继续 |
| `plan_file_path` | string | plan 文件的路径，Agent 写入和读取的唯一可写目标 |

**边界**：PlanState 仅承载会话恢复和 compaction 隔离保护所需的最小状态。执行步骤的完成状态（未开始/进行中/已完成/失败/已跳过）由 Agent 写在 plan 文件中管理，系统不介入进度判断——PlanState 不包含执行步骤状态机（执行步骤状态定义见 [mode 执行引擎](../mode/execution.md)）。

## 数据流

NormalizedMessage 的全系统流动路径：

```
IM 平台事件 / terminal stdin
  ↓
IM Adapter 入站解析（各平台插件）
  → 平台格式转 NormalizedMessage { platform, sender_id, peer_id, reply_ref?, account_id, content, message_type, media_refs, unavailable_media, timestamp }
  ↓
Processor Chain 入站
  → RawLog（记录日志）→ SessionRouter（计算 session_key）→ ContentNormalizer（文本标准化）
  → 产出 ProcessedMessage
  ↓
Gateway 路由
  → SessionManager 查找/创建 session → LLM 对话 / SlashDispatcher
```

NormalizedMessage 仅用于入站方向。出站方向使用 ContentBlock[]（LLM 输出）和 [ProcessedMessage](#processedmessage)（经 Processor Chain 处理后的中间结构），与 NormalizedMessage 无关。

卡片交互事件不进入上述入站流动：Adapter 解析阶段识别后，[CardActionEvent](#cardactionevent) 的动作值经 tool_result 通道注入对话（见建模边界）。

流式场景下的增量流动以 [StreamEvent](#streamevent) 事件流表达，流动路径即下文「ContentBlock[] 的出站流动路径」的流式分支（事件的产生与消费分工见模块关系节）；渲染过程中的单批产出为 [StreamingOutput](#streamingoutput)（完整文本行或强制输出的行内片段 + 本批完成的非文本块），生命周期止于本次流式发送完成。

LLM 非流式调用的响应流动：LlmCaller 返回 [UnifiedResponse](#unifiedresponse--unifiedusage) → Session 封装后其 ContentBlock[] 进入上述出站路径；用量 UnifiedUsage 由 [RunningStats](#runningstats--cachebreakinfo--cachebreakthresholds) 累加，供用量统计与缓存异常提醒。

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
[Gateway 出站日志] — 记录完整 ProcessedMessage
  ↓
[IM Adapter 渲染] — 按块类型选择渲染策略，输出平台原生格式：
    - 批量模式：一次性渲染全部 ContentBlock[]
    - 流式模式：消费 [StreamEvent](#streamevent) 增量事件，Text 块逐行缓冲输出，非文本类块等 BlockEnd 全块就绪后一次渲染
  ↓
[中间件插入点] — Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件。中间件为 Gateway 内部的拦截链，具体中间件类型和注册机制由 Gateway 管理，不在 shared-types 范围
  ↓
IM Adapter 发送到目标平台
```

来源说明：卡片交互事件经 [CardActionEvent](#cardactionevent) 的 tool_result 通道注入对话后触发的模型回复，仍以 UnifiedResponse 形态进入上述同一条出站路径——卡片交互场景的出站闭环复用本图，不另设通路。

图中出现的两处日志是不同层次的两份记录：链内的 OutboundRawLog 是 Processor Chain 出站的调试日志（按 Verbosity 过滤后的内容）；[Gateway 出站日志] 指发送成功后 Gateway 写入 session checkpoint 的出站历史记录（含 timestamp、session_id、platform、ContentBlock[]、dsl_result），规则详见 [gateway outbound-flow](../gateway/outbound-flow.md)。

ContentBlock[] 流式与非流式走同一条预处理管线——Verbosity 过滤和 DslParser 解析同时适用于批量和流式。流式模式下增量内容以 [StreamEvent](#streamevent) 事件流形式在链上传递：VerbosityFilter 按块边界逐事件过滤；DslParser 零开销透传（不解析 DSL），DSL 完整解析推迟到收尾阶段对完整 ContentBlock[] 执行。非 DSL 内容不引入额外缓冲或拷贝。两者的差异在渲染阶段：批量模式一次性渲染，流式模式增量渲染；流式模式下 DSL 指令仅用于日志记录和出站历史写入，不产生渲染输出。

各共享类型流动路径的详细描述见下文各类型的数据流节。

### DslParseResult / DslInstruction

DslParseResult 的流动嵌入在 ContentBlock[] 的出站路径中：

```
ContentBlock[]（来自 LLM UnifiedResponse / SlashResult）
  ↓
[Processor Chain 出站: VerbosityFilter] — 按 Session Verbosity 等级逐块过滤
  ↓
DslParser 遍历 Text 块，逐行扫描 DSL 指令：
  - 匹配 DSL 格式的行 → 解析为 DslInstruction，加入 instructions 列表，从 Text 块中移除该行
  - 非 DSL 行 → 保留在 Text 块中
  两种情况的输出汇合为 DslParseResult { instructions } + 更新后的 ContentBlock[]
  ↓
[Processor Chain: OutboundRawLog] — 出站日志记录
  ↓
打包为 [ProcessedMessage](#processedmessage)
  ↓
Renderer 消费 DslParseResult：
  ├── button / selector → 渲染为平台交互元素（IM 平台卡片 button 组件、终端纯文本提示行）
  └── 其他指令类型 → Renderer 按平台能力处理或忽略
```

DslParseResult 的生命周期始于 DslParser 解析、终于 Renderer 渲染（批量模式）或出站历史写入（流式模式，仅日志不渲染）。中间经 OutboundRawLog（Processor Chain 出站日志）和 [ProcessedMessage](#processedmessage) 传递。DslParseResult 本身不被 Verbosity 过滤影响——DslParser 仅处理已通过过滤的 ContentBlock[]，因此 DslParseResult 中只包含可见块中的 DSL 指令。

### ProcessedMessage

入站方向：

```
NormalizedMessage → Processor Chain 入站（RawLog → SessionRouter → ContentNormalizer）
  ↓
ProcessedMessage {
  content_blocks: [ContentBlock::Text("标准化后文本")],
  metadata: { session_key: "{timestamp}-{hash}", message_type: "<原始 message_type>", unavailable_media: "<不可得媒体资源标识列表 JSON>" }
}
  ↓
Gateway — 先检查 message_type：含媒体消息做媒体可得性校验（不可得 → 提示「该消息内容无法获取」经简化出站路径发送、流程结束；可得 → 按类型构造上下文形态后与文本同链路继续，形态规则见 [im_adapter media-store](../im_adapter/media-store.md)）；对话消息从 content_blocks[0] 取 Text 内容做路由决策（/ 开头 → 斜杠指令；否则 → LLM 对话），从 metadata 取 session_key 传给 SessionManager
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
4. Gateway 构造 SideEffectContext
5. 高危指令（Exec、Git 写操作）：Gateway 调用 Permission 引擎校验权限（校验通过方继续执行，拒绝则返回权限错误）
6. 权限校验通过后，SlashResult 变体通过 SideEffectContext 触发执行，完成副作用，分两条路径：
   - 回复路径：产出 ContentBlock[] → 出站 Processor Chain → IM Adapter 渲染发送
   - 会话路径：执行 Session 操作（模式切换、创建、停止、压缩等）

SlashResult 的生命周期：Handler 返回 → Gateway 构造 SideEffectContext 并触发执行 → 各变体通过 SideEffectContext 完成副作用后销毁。

### FragmentContext / PromptFragment

FragmentContext 和 PromptFragment 的流动嵌入在 system prompt 静态层的构建流程中：

```
SessionManager 触发构建
  ↓
System Prompt Builder 构建 FragmentContext（agent_id + bootstrap_mode + bootstrap_dir）
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
IMPlugin.send(rendered_output, peer_id, reply_ref) → 平台发送 API
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

### CardActionEvent

CardActionEvent 的管理路径：

```
用户点击消息内嵌交互控件（按钮、选择器等）
  ↓
平台推送交互事件 → IM Adapter 解析识别（区分于消息事件，不产 NormalizedMessage）
  ↓
提取 action_value 等字段构造 CardActionEvent
  ↓
经 tool_result 通道注入对话（不进入入站 Processor Chain）
```

### UserRegistration / UserCreationRequest / InitialPermissionSet

新用户注册工作流的载荷流转：

```
User 发起注册指令（如 /perm register）
  ↓
Gateway 层硬拦截（同审批类指令，不进 SlashDispatcher，不产出 SlashResult）
  ↓
构造 UserCreationRequest { request_id, initial_permissions } 入审批队列
  ↓
Owner 审批通过（快照与去重机制见 permission 审批工作流）
  ↓
生成 UserRegistration 记录 + InitialPermissionSet 映射为具体权限规则落盘
```

### UnifiedResponse / UnifiedUsage

UnifiedResponse 的流动路径：

```
Session 发起非流式 LLM 调用 → LlmCaller 返回 UnifiedResponse { content_blocks, usage, ... }
  ↓
content_blocks → 出站处理链路（路径同上文 ContentBlock[]）
  ↓
usage (UnifiedUsage) → RunningStats 累加 → 用量统计与调试日志
```

流式调用不走 UnifiedResponse：增量以 StreamEvent 交付，收尾时 MessageEnd 携带 Optional UnifiedUsage 提供同一套用量口径（见 StreamEvent 节），同样汇入 RunningStats。

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

## 模块关系

### NormalizedMessage

- **生产者**：IM Adapter 各平台插件（入站解析）——包括飞书、Discord、Telegram 等 IM 平台的 Adapter，以及 CLI 模块的 TerminalAdapter
- **消费者**：Processor Chain 入站（读取 NormalizedMessage 做内容标准化和 session_key 计算，产出 [ProcessedMessage](#processedmessage)）
- **无关**：LLM Provider（不接触 NormalizedMessage，只消费 ContentBlock[]）、Session（通过 Gateway 间接消费路由字段，不直接接触 NormalizedMessage）、Slash Command（斜杠指令不涉及 NormalizedMessage 结构）

### ContentBlock

- **生产者**：Session（LLM 对话产出 UnifiedResponse，含 ContentBlock[]）、SlashDispatcher（斜杠指令回复以 SlashResult 变体产出 ContentBlock[]）、Processor Chain 入站 ContentNormalizer（入站方向包装标准化文本为 ContentBlock::Text 放入 ProcessedMessage.content_blocks）
- **消费者**：Processor Chain 出站（VerbosityFilter → DslParser → OutboundRawLog）→ IM Adapter（按块类型渲染为平台原生格式并发送）
- **无关**：IM Adapter 入站链（入站方向产 NormalizedMessage，不涉及 ContentBlock[]）、Session 生命周期管理（不直接操作 ContentBlock[]，仅通过 Gateway 间接消费）、LLM Provider（LLM 调用返回原始 ContentBlock[]，由 Session 统一封装为 UnifiedResponse 后进入共享类型流；LLM Provider 不参与跨模块 ContentBlock 结构定义和传递流程）、[Gateway](../gateway/README.md)（Gateway 编排 Processor Chain 调度，不直接执行内容过滤/解析）

### DslParseResult / DslInstruction

- **DslParseResult 生产者**：Processor Chain 出站（DslParser 解析 ContentBlock::Text 中的 DSL 指令行，产出 DslParseResult）
- **DslParseResult 消费者**：IM Adapter 各平台 Renderer（读取 DslParseResult 中的 DslInstruction 列表，渲染为平台交互元素）、CLI TerminalRenderer（将 button/selector 转为纯文本提示行）
- **DslInstruction 生产者**：Processor Chain 出站（DslParser 逐行解析 DSL 指令，每条产出一个 DslInstruction）
- **DslInstruction 消费者**：IM Adapter 各平台 Renderer（按 instruction_type 选择渲染策略）
- **无关**：Processor Chain 入站（DSL 解析仅在出站方向执行）、IM Adapter 入站链（入站方向不涉及 DSL）、LLM Provider（LLM 不感知 DSL）、Session（Session 不操作 DslParseResult）

### StreamEvent

- **生产者**：LLM 模块（Protocol 层解析各协议 SSE 原生事件，ModelInterpreter 归一化为 StreamEvent，映射规则见 [llm protocol-mapping](../llm/protocol-mapping.md)）
- **消费者**：流式出站链路——Session（接收事件流并转发 Gateway）、Gateway（增量阶段调度 Processor Chain 与 IM Adapter 流式渲染）、Processor Chain 出站（VerbosityFilter 按块边界逐事件过滤、DslParser 透传）、IM Adapter 流式渲染器（逐事件消费，Text 块依赖 BlockDelta 携带的 [ContentDelta](#contentdelta) 逐行输出）
- **无关**：入站链路（入站不产生流式事件）、SlashDispatcher（斜杠指令回复为完整 ContentBlock[]，走批量模式）

### ContentDelta

- **生产者**：LLM 模块（协议 SSE 归一化为 StreamEvent 时随 BlockDelta 产出）
- **消费者**：流式渲染组件（StreamingRenderer 按 delta 变体累积行缓冲和块状态）、Session（重组完整 ContentBlock 写入对话历史）
- **无关**：批量路径（非流式响应直接返回完整 ContentBlock[]，无增量）、入站链路

### UnifiedResponse / UnifiedUsage

- **生产者**：LLM 模块（各供应商协议响应归一化产出 UnifiedResponse；LlmCaller 实现方 gateway 返回给 Session）
- **消费者**：Session（content_blocks 进入出站链路、写入对话历史；usage 记录统计）；UnifiedUsage 另被 StreamEvent::MessageEnd 作为收尾用量携带（消费者同 StreamEvent 流式链路）
- **无关**：IM Adapter 入站链、Permission、斜杠指令分派

### CardActionEvent

- **生产者**：IM Adapter 各平台插件（入站解析阶段从交互事件 payload 构造）
- **消费者**：Gateway/tool_result 通道（将 action_value 作为工具调用回执注入对话）
- **无关**：入站 Processor Chain（交互事件不经消息链路）、LLM Provider（不感知事件来源结构）

### StreamingOutput

- **生产者**：IM Adapter 流式渲染组件（每次批量处理事件、刷新或超时检查后产出一批）
- **消费者**：平台插件的流式发送逻辑（将本批文本行与内容块组装为 RenderedOutput 后经发送能力投递）、gateway（调度流式出站管线时传递该结构）
- **无关**：Session 持久化（中间产物，不进 checkpoint）、批量渲染路径

### UserRegistration / UserCreationRequest / InitialPermissionSet

- **生产者**：Gateway 权限指令处理层（同审批指令的硬拦截路径：注册类权限指令不进 SlashDispatcher，解析参数后直接构造；审批通过后生成 UserRegistration）
- **消费者**：permission 模块（InitialPermissionSet 映射为具体权限规则；UserRegistration 落盘用户记录）、审批队列（UserCreationRequest 流转与回调）
- **无关**：SlashDispatcher（注册指令硬拦截，不产出 SlashResult）、LLM Provider、Processor Chain、IM Adapter 入站链

### RunningStats / CacheBreakInfo / CacheBreakThresholds

- **生产者**：Session（持有并随每次 LLM 调用累加；压缩流程按需读取快照）
- **消费者**：session（compaction 阈值判断）、gateway（checkpoint 恢复时传递统计快照）、slash（/status 呈现命中率与累计用量）、llm（re-export 供模块内使用）
- **无关**：Processor Chain 出站过滤、IM Adapter 渲染

### ProcessedMessage

- **生产者**：Processor Chain 入站（ContentNormalizer 包装标准化文本为 ContentBlock::Text + SessionRouter 写 session_key 到 metadata）、Processor Chain 出站（DslParser 处理 ContentBlock[] + 写 dsl_result 到 metadata）
- **消费者**：Gateway（入站：消费 content_blocks + metadata.session_key 做路由决策 + metadata.message_type 做分型路由判断；出站：消费 content_blocks + metadata.dsl_result 做出站日志后传给 IM Adapter）、IM Adapter（消费 content_blocks + metadata.dsl_result 渲染为平台格式并发送）、CLI TerminalRenderer（同 IM Adapter，渲染为 ANSI 终端文本）
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
- **消费者**：Gateway（中间件——在渲染与发送之间插入审计、频率限制等中间件，不改变 RenderedOutput 内容）；IM Adapter（IMPlugin.send() 消费 RenderedOutput 发送）
- **无关**：Processor Chain（RenderedOutput 在 Processor Chain 之后产出，不经过链处理）、LLM Provider（不接触 RenderedOutput）

### VerbosityLevel

- **生产者**：slash 模块（VerboseHandler 处理 `/verbose` 指令，写入 Session）
- **消费者**：Processor Chain 出站（VerbosityFilter 读取并过滤 ContentBlock[]）；Session（存储当前等级，供下次出站过滤）
- **无关**：LLM Provider（Verbosity 不影响 LLM 推理，仅控制展示）、IM Adapter 入站（入站不涉及展示过滤）

### PlanState

- **生产者**：mode 模块（Plan Mode 进入时创建）
- **消费者**：Session（持久化和 compaction 保护）；mode 模块（恢复时重建、阶段切换时更新）
- **无关**：LLM Provider（PlanState 不直接传给 LLM，通过 system prompt 的 plan 上下文间接生效）、IM Adapter（消息路由不感知 PlanState）