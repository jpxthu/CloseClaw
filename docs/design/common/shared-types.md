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
| `thread_id` | string? | 话题/线程 ID（支持线程的平台上用于线程回复）；不参与 session_key 计算 |
| `reply_ref` | string? | 出站定向引用，可选。插件按平台语义填入、出站时原样消费的平台引用（如话题根消息标识），用于把回复投递回原会话位置。不参与 session 路由。出站传递机制：入站填入后经 Session 上下文存储，出站时由 Gateway 取出传给 IMPlugin 发送，见 [session-lifecycle 出站定向字段](../session/session-lifecycle.md) |
| `account_id` | string | CloseClaw 本地账号标识，由「平台 + 接收方机器人应用 + sender_id」经身份映射得到。参与 session 路由 |
| `chat_name` | string | 聊天/群名称（如飞书群标题），由 Adapter 从平台元数据填充，可为空 |
| `trace_id` | string | 分布式追踪标识，webhook 到达时生成，用于调试日志链路串联 |
| `message_id` | string | 平台消息标识（飞书顶层消息 ID等），由 Adapter 解析时填入，可为空 |
| `content` | string | 消息文本内容。纯媒体消息时可为空 |
| `message_type` | enum | 消息类型：text / image / file / audio / post。post 为含内嵌媒体的富文本消息——展开文本入 content，内嵌媒体入 media_refs |
| `media_refs` | list(MediaRef) | 消息携带的媒体引用列表。Adapter 在入站解析时完成媒体落盘并填充，下游一律以引用消费，不接触平台下载地址与凭证。落盘与消费机制见 [im_adapter media-store](../im_adapter/media-store.md) |
| `unavailable_media` | list(string) | 消息引用但未能获得的媒体资源标识列表（下载失败或超出大小上限）。由 Adapter 在入站解析时填充；失败媒体不进入 media_refs、仅记录于此。非空时 Gateway 按媒体不可得处理（提示用户、不进入对话），见 [im_adapter media-store](../im_adapter/media-store.md) |
| `timestamp` | int | 消息发送时间（毫秒级 Unix 时间戳） |

**机器人身份（接收方机器人应用）**：接收方机器人应用标识（bot_app_id）不进入 NormalizedMessage 的字段表。IM Adapter 在入站解析时单独提取，用于 Agent 路由（选择处理该机器人消息的 Agent），并作为 `account_id` 身份映射键的一部分（见 [core-traits IdentityResolver](core-traits.md#identityresolver)）。

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

> **平台现状**：当前各 IM 平台（如飞书）将卡片交互事件列为暂缓——不进入消息通路、仅记录调试日志；启用时按本节设计接入（见 [im_adapter 飞书 · 事件分流](../im_adapter/platforms/feishu.md)）。

### ContentBlock

ContentBlock 是跨模块传递的结构化内容单元。所有出站内容——LLM 回复和斜杠指令回复——均以 ContentBlock[] 数组形式传递，贯穿 Verbosity 过滤、DSL 解析、出站日志记录和平台渲染全链路。入站方向经 Processor Chain 处理后，标准化文本以 ContentBlock::Text 形式放入 [ProcessedMessage](#processedmessage) 的 content_blocks 字段，入站不涉及 ContentBlock 的其他变体。流式场景下，同一份内容以 [StreamEvent](#streamevent) 增量事件形式传递，完整块由消费方按事件边界组装。

ContentBlock 共 7 种变体，按语义和渲染策略分为两类（变体来源：LLM 协议层归一化产出 Text/Thinking/ToolUse/ToolResult 四种，详见 [llm protocol-mapping](../llm/protocol-mapping.md)；Image/Audio/File 三种不由 LLM 产出，用于非流式路径的媒体交付）：

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
- **流式渲染差异化**：Text 块逐行缓冲输出（以句末标点或换行符为行边界）；Thinking/ToolUse 块等待全块就绪后一次交付渲染；Image/Audio/File 不以流式事件形式出现，在非流式路径中交由平台格式渲染器处理
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

ContentDelta 是单个 ContentBlock 内部的增量载荷，BlockDelta 事件的载体。共 9 种变体，每个变体归属唯一的块类型（一个块类型可对应多个增量变体，如 ToolUse 对应 3 个）：

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
| `metadata` | map(string→string) | 方向相关的键值对。入站含 `session_key`（SessionRouter 计算的消息级标识，用于日志追踪）、`trace_id`（IM Adapter 入站生成、随消息流转）、`platform` / `sender_id` / `peer_id`（SessionRouter 写入的稳定路由键组成部分）、`message_type`（来自原始 NormalizedMessage，由 Processor Chain 在构建 ProcessedMessage 时从 NormalizedMessage 复制，供 Gateway 做分型路由判断）和 `unavailable_media`（不可得媒体资源标识列表，JSON 序列化，同样复制，供 Gateway 做媒体可得性判断）；出站含 `dsl_result`（DslParser 产出的 DslParseResult，JSON 序列化）。metadata 字段的复制均发生在链调度构建 ProcessedMessage 时——各 Processor 不修改 NormalizedMessage 字段，复制不构成对消息的修改 |

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

**边界**：SlashResult 仅由 SlashDispatcher 分派的斜杠指令 Handler 产出。审批指令（`/approve-once`、`/approve-whitelist`、`/deny`）由 Gateway 层硬拦截、走权限审批流验证，不进 SlashDispatcher，其审批结果不属于 SlashResult（详见 [permission 审批工作流](../permission/approval-workflow.md)）。权限管理指令（如 `/perm <subcmd>` 系列（allow-file/deny-file/allow-cmd/deny-cmd））同样不产出 SlashResult，由 Gateway 权限指令处理层硬拦截执行——新用户注册的载荷结构见 [UserRegistration / UserCreationRequest / InitialPermissionSet](#userregistration--usercreationrequest--initialpermissionset)。

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
| `session_role` | enum | Session 角色：SessionRole::Main（主 Agent Session）或 SessionRole::Sub（子 Session）。长期记忆（MEMORY.md）与自定义引导指令（BOOTSTRAP.md）按此角色门控加载，与身份加载模式无关 |
| `bootstrap_mode` | enum | 身份加载模式，取值来自 agent 配置的 `bootstrapMode`：BootstrapMode::Minimal（精简）或 BootstrapMode::Full（完整），仅在主 Agent Session 决定是否注入自定义引导指令（BOOTSTRAP.md）。子 Session 不据此注入可选内容 |
| `bootstrap_dir` | string | bootstrap 文件所在目录，BootstrapFragmentProvider 按此查找 bootstrap 文件。值来源于 agent 配置的 agentDir 字段 |
| `activated_skills` | list(string) | 当前 session 已条件激活的技能名列表，由 Session 模块在 SP 重建时按值传入，SkillsFragmentProvider 据此把已激活条件技能纳入清单（仅 SP 重建路径有效） |
| `tool_registry` | ToolRegistryQuery? | 供需要工具信息的 Provider（如 ToolsFragmentProvider）直接查询的工具注册表引用；来自 InjectionParams，缺省回退 Provider 默认 |

Session 角色（主/子）在主 Session 创建 / spawn 子 Session 时确定，由 SessionManager 在触发构建时同 agent_id 一并传给 Builder 写入 FragmentContext。身份加载模式（bootstrap_mode）是 agent 配置的静态属性；两者正交——主 Agent Session 精简模式下 session_role 仍为 Main。

### PromptFragment

PromptFragment 是单个 PromptFragmentProvider 产出的静态层片段。

| 字段 | 类型 | 说明 |
|------|------|------|
| `section_title` | string | Section 标题，如 `## AGENTS.md`、`## Available Skills` |
| `section_type` | enum | Section 类型：bootstrap 文件、工具列表、skill 清单、长期记忆 |
| `content` | string | 渲染完成的文本内容 |

### RenderedOutput

RenderedOutput 是 IMPlugin 渲染方法产出的平台原生格式消息结构。渲染产出数据，发送执行副作用——Gateway 在两步之间插入中间件（审计、频率限制等）。流式场景下渲染以增量方式进行：StreamingRenderer 每处理完一批事件产出一个 StreamingOutput（见 [core-traits StreamingRenderer](core-traits.md#streamingrenderer)），平台按本批内容渲染或增量更新平台消息（不预先组装为最终整条消息），不再单独定义平台消息结构。

| 字段 | 类型 | 说明 |
|------|------|------|
| `msg_type` | string | 消息格式类型（如 `"text"`、`"interactive"`），由 Renderer 按内容特征选择 |
| `payload` | any | 平台原生格式的消息体，结构由各平台 Renderer 定义。Gateway 中间件和 Adapter 发送不解析 payload 内容 |

**输出格式决策**：由各平台 Renderer 按内容特征选择输出格式，规则详见 [IM Adapter §平台渲染选择](../im_adapter/README.md#平台渲染选择)。大致原则：纯文本、无格式标记、无 DSL → `"text"`；含 markdown 格式/换行/DSL/Thinking/ToolUse/ToolResult 块 → `"interactive"`。终端渠道例外：terminal 渠道无富格式消息形态，RenderedOutput 恒为 `"text"`——富内容（Thinking/工具块、DSL）已在 payload 内转为 ANSI 样式文本（见 [cli/Terminal Renderer](../cli/renderer.md)）。

#### StreamingOutput

StreamingOutput 是流式渲染过程中单批事件的处理产出：本批投递的文本内容列表（完整文本行或强制输出时的行内片段），加本批内累积完整的非文本块。被 common 的 [StreamingRenderer](core-traits.md#streamingrenderer) 处理单批事件产出、各平台插件持有并委托调用；其流式发送逻辑将本批内容组装为 RenderedOutput 后经 IMPlugin 的发送能力投递（不经 IMPlugin 的渲染方法），gateway 在流式出站管线中传递该结构——满足共享类型准入条件。

| 字段 | 类型 | 说明 |
|------|------|------|
| `text_messages` | list(string) | 本批输出的文本内容：行边界达成的完整文本行，或触发强制输出（缓冲超阈值/超时）时的行内片段。缓冲与阈值规则见 [im_adapter streaming-render](../im_adapter/streaming-render.md) |
| `render_blocks` | ContentBlock[] | 本批内累积完整的非文本块（Thinking/ToolUse），等待全块就绪的渲染策略在此交付 |

StreamingOutput 是渲染过程的中间产物，生命周期止于本次流式发送完成，不进入 Session 或日志持久化。行缓冲/阈值/分批的默认规则由 [StreamingRenderer 默认实现](core-traits.md#streamingrenderer)（位于 common）提供，平台差异化渲染策略见 [im_adapter streaming-render](../im_adapter/streaming-render.md)。

### ContentSegment / 内容段落解析

ContentSegment 是平台无关的内容段数据结构，把 ContentBlock::Text 的 markdown 文本按行切分为内容段，供各适配器逐段渲染（仅 Text 变体进入本原语）。共 3 种变体：

| 变体 | 语义 |
|------|------|
| Markdown | 普通 markdown 文本行（空行作为独立内容段保留） |
| Hr | 分隔线段落 |
| CodeBlock | 围栏代码块，作为整体单元 |

CodeBlock 承载代码块内容，字段定义：

| 字段 | 类型 | 说明 |
|------|------|------|
| `language` | string | 代码块语言标注，无标注时为空 |
| `code` | string | 代码块内容（不含围栏行） |

内容段由配套的内容段解析产出：无 IO 副作用，按行切分文本——围栏代码块收集为单个 CodeBlock，未闭合围栏按普通 markdown 文本行处理，分隔线识别为 Hr，其余为 Markdown；解析面向单个 ContentBlock::Text 的整块文本。流式增量路径由 [core-traits StreamingRenderer](core-traits.md#streamingrenderer) 的事件流承接，不经本原语。

**归属**：位于 common（`common/src/content_segment.rs`），被 im_adapter（飞书平台渲染路径）与 cli（TerminalRenderer）2+ 模块消费、平台无关、无单一领域归属，满足 [STANDARDS.md 共享类型准入](../STANDARDS.md)。平台无关共享渲染原语位于 common（流式渲染原语见 [core-traits StreamingRenderer](core-traits.md#streamingrenderer)），各适配器持有并委托调用，平台专属 emit（飞书卡片富文本组装、终端 ANSI 渲染）留各适配器自身；消费方经 common 直接引用，不另立二次出口（见 [STANDARDS.md 禁止二次出口](../STANDARDS.md)）。

### VerbosityLevel

VerbosityLevel 是出站信息展示等级的枚举，控制 VerbosityFilter 对 ContentBlock 的过滤策略。由 `/verbose` 指令设置，Session 存储，出站 Processor Chain 的第一道过滤（VerbosityFilter，priority 5）消费。

**过滤对象边界**：Verbosity 控制的是 Agent 工作过程中的**中间产物**，不影响交付给用户的**最终回复内容**。结合上文 [ContentBlock](#contentblock) 的分类：
- **中间产物块**：Thinking（思考过程）、ToolUse（工具调用请求）、ToolResult（工具执行结果）——记录 Agent 内部工作过程与进度，是 Verbosity 各等级按档位显隐的对象
- **最终回复内容块**：Text，以及作为交付物出现的 Image / Audio / File——是对用户的实际交付，不属于中间产物，不随展示等级被过滤，在任意等级下均展示

只有中间产物块参与等级过滤，最终回复内容块始终展示。三个等级：

| 等级 | 值 | 过滤行为 |
|------|---|---------|
| full | `"full"` | 展示全部：思考过程、工具调用与结果等中间产物与最终回复内容均展示，不过滤 |
| normal | `"normal"` | 保留工具调用与结果作为进度提示，隐藏思考过程（Thinking） |
| off | `"off"` | 移除全部中间产物（Thinking / ToolUse / ToolResult），仅展示最终回复内容（Text 与作为交付物的 Image / Audio / File） |

**作用范围**：Verbosity 控制展示内容，不影响 LLM 推理深度和 Agent 行为模式。仅有 `/verbose` 指令通过 VerboseHandler 写入 Session 的 Verbosity 字段，无其他写入者。切换等级不影响当前正在输出的消息——仅对后续新消息生效。

### PlanState

PlanState 是 Plan Mode 下的规划状态结构，由 mode 模块管理，Session 持久化。Compaction 对此状态做隔离保护（不压缩 plan 相关消息），Session 恢复时重建 PlanState。

PlanState 描述当前规划所处阶段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `phase` | enum | 当前阶段：Research / Design / Review / FinalPlan（标准路径四阶段，阶段流转语义见 [mode/plan-mode.md](../mode/plan-mode.md)） |
| `plan_file_path` | string | plan 文件路径，规划阶段 Agent 写入与读取的唯一目标 |

**边界**：PlanState 仅承载会话恢复和 compaction 隔离保护所需的最小状态。执行步骤的完成状态（未开始/进行中/已完成/失败/已跳过）由 Agent 写在 plan 文件中管理，系统不介入进度判断——PlanState 不包含执行步骤状态机（执行步骤状态定义见 [mode 执行引擎](../mode/execution.md)）。

### CompactionResult / CompactionError

compaction（对话历史压缩）操作的产出与错误。CompactionResult 描述一次压缩的结果——是否执行、压缩前后 token/字符数、承载摘要的边界消息、是否自动触发，以及供 Gateway 回发的可读描述；CompactionError 是压缩失败的错误（LLM 调用失败、会话未找到、摘要解析失败、无消息可压缩、所需 handler 不可用）。两者经 [SlashEffectExecutor](core-traits.md#slasheffectexecutor) 的压缩方法在 session（产出）与 Gateway（消费）之间传递，故收录 common。

**边界**：本题只承载跨 trait 边界的压缩产出与错误。压缩的触发条件、保留区、熔断与摘要格式等行为语义，以及压缩相关配置（阈值、保留区比例、熔断次数、摘要模型，按 Agent 配置）均归 session 模块，设计定义见 [session compact-process](../session/compact-process.md) 与 [session lifecycle](../session/session-lifecycle.md)。

### ReasoningLevel / AgentRole / SessionMode

会话行为相关的跨模块枚举。

- **ReasoningLevel**：推理深度档位——low / medium / high / max，默认 high；off 表示关闭推理输出（供应商不支持关闭时降至最低可用档位）。作为 [InternalRequest](#internalrequest--internalmessage--systemblock--tooldefinition) 字段随 LLM 请求传递，各协议映射为供应商原生参数；`/reasoning` 指令经 [SlashEffectExecutor](core-traits.md#slasheffectexecutor) 设置，由网关层解析生效档位——当所选模型不支持请求档位时自动降级到该模型支持的最高档位（需求见 [llm §F4](../../requirements/llm.md)）。
- **AgentRole**：Agent 身份枚举——MainAgent（主 Agent）/ SubAgent（分身 Agent），标识 Agent 层级（与 [FragmentContext](#fragmentcontext) 的 SessionRole（主/子 Session）相关但不同层：前者描述 Agent 身份，后者描述 Session 角色）。
- **SessionMode**：会话运行模式——Normal / Plan / Auto，控制工具可见性、权限边界与 system prompt 指令；由 `/mode` 设置，作为 [SessionModeQuery](core-traits.md#sessionmodequery) 的返回类型跨模块查询（语义见 [mode 模块](../mode/README.md)）。

### InternalRequest / InternalMessage / SystemBlock / ToolDefinition

系统内部协议无关的 LLM 请求结构族。[LlmCaller](core-traits.md#llmcaller) 的一次调用以 InternalRequest 表达，各 Provider 协议层将其映射为供应商原生请求格式（映射规则见 [llm protocol-mapping](../llm/protocol-mapping.md)）。它承载消息序列、采样参数、两段式 system prompt（静态可缓存段 + 动态段）与工具定义，是 LLM 调用契约的载荷。

**InternalRequest 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `model` | string | 本次请求使用的模型标识 |
| `messages` | list(InternalMessage) | 有序对话消息序列 |
| `temperature` | float | 采样温度，默认 0 |
| `max_tokens` | int? | 生成上限，可选 |
| `stream` | bool | 是否流式，默认否 |
| `system_static` / `system_dynamic` | string? | system prompt 静态（可缓存）段与动态段，两段契约见 [system_prompt kv-cache](../system_prompt/kv-cache.md) |
| `system_blocks` | list(SystemBlock)? | 缓存适配器产出的结构化 system 块，可选 |
| `tools` | list(ToolDefinition)? | 随请求传入的工具 schema 定义，可选 |
| `session_id` | string? | 供应商级缓存键使用的会话标识 |
| `extra_body` | map(string→any) | 供应商特定附加参数，可选 |
| `reasoning_level` | [ReasoningLevel](#reasoninglevel--agentrole--sessionmode) | 推理深度档位，各协议映射为原生参数（默认 high） |
| `turn_count` | int? | 会话内轮次计数，用于 API 元数据 |

**InternalMessage**：请求中的单条消息——`role`（角色）、`content`（文本）、`content_blocks`（多模态内容块，存在时优先于 content）、`tool_call_id`（工具结果消息标识）。

**SystemBlock**：缓存适配器产出的结构化 system 块——`text`（文本）、`cache`（是否标记可缓存）。

**ToolDefinition**：随 API `tools` 参数传入的工具定义——`name`、`description`、`input_schema`（JSON Schema）、`cache`（schema 是否标记可缓存）。

**边界**：本族是 LLM 调用契约的载荷，不进入消息出站链路的共享类型流；LLM 输出经 [UnifiedResponse](#unifiedresponse--unifiedusage) 返回。

### CommunicationConfig / CommunicationCheckResult / CommunicationError

Agent 间通信的允许列表与其校验结果。CommunicationConfig 是某个 Agent 的通信白名单——出向（允许发给哪些 Agent）与入向（允许接收哪些 Agent 的消息）两组 ID 列表，支持 `*` 通配；默认白名单仅含父 Agent（故默认仅直接父子 Session 互通，扩展需额外配置）。该配置在 spawn 子 Session 时生成（父子会话路由表；根 Session 无通信配置），在消息路由时由 Session 模块消费。消息送达需同时满足路由配置与权限允许两个条件（见 [agent §F14](../../requirements/agent.md)）。CommunicationCheckResult 是通信检查结果（允许 / 源不在目标入向 / 目标不在源出向）；CommunicationError 是通信被拒或会话/配置缺失的错误。

**CommunicationConfig 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `outbound` | list(string) | 本 Agent 允许发送消息的目标 Agent ID 列表，`*` 表示不限 |
| `inbound` | list(string) | 本 Agent 允许接收消息的来源 Agent ID 列表，`*` 表示不限 |

**CommunicationCheckResult 取值**：Allowed（允许）、SourceNotInTargetInbound（来源不在目标入向白名单）、TargetNotInSourceOutbound（目标不在来源出向白名单）。

**CommunicationError 变体**：Denied（因白名单限制被拒，携带原因）、SessionNotFound（会话未找到）、NoCommunicationConfig（会话无通信配置）。

### RiskLevel / PermissionEvalResponse / CallerInfo / PermissionDenied / SpawnPermissionError

权限评估与审批 trait 的载荷族，随 [PermissionEvaluator](core-traits.md#permissionevaluator)、[ApprovalSubmission](core-traits.md#approvalsubmission)、[PermissionChecker](core-traits.md#permissionchecker) 在 common 与消费/实现方之间传递。

- **RiskLevel**：权限请求的风险级别枚举——Low / Medium / High / Critical。
- **PermissionEvalResponse**：Agent 间权限评估结果——Allowed，或 Denied（`reason` 原因 + `risk_level` 风险级别）。
- **CallerInfo**：审批提交的调用方信息——`user_id`、`agent`、`is_sub_agent`（子 Agent 的拒绝静默丢弃、不入审批队列）。
- **PermissionDenied**：权限校验被拒的错误——携带 `reason` 原因。
- **SpawnPermissionError**：子 Agent spawn 权限校验被拒的错误——携带 `agent_id` 与 `reason`。

**边界**：本族仅承载权限评估/审批的输入输出语义；权限规则判定逻辑归 permission 模块（见 [permission 模块](../permission/README.md)）。

### HookConfig / HookParams / HookType

Session hook 审查的配置族（按 Agent 配置）。HookConfig 是单个 hook 的配置——`hook_type`（类型）、`enabled`（是否启用）、`params`（可调参数）；HookType 是 hook 类型枚举——PlanCheck（只计划未执行）、LoopCheck（重复工具调用循环）、ProgressCheck（无可验证进展）；HookParams 是 hook 可调阈值——`loop_check_repetition_threshold`（判定循环的连续相似调用数）、`progress_check_min_tool_calls`（参与审查的最小工具调用数）。由 agent 配置定义、session 的 Hook 审查器（HookReviewer）消费。hook 审查行为见 [session run-health](../session/run-health.md)。

### ShutdownState / ShutdownMode / DrainStatus

[ShutdownSignal](core-traits.md#shutdownsignal)（common DI trait）契约的载荷族。ShutdownState 是关停状态机——Running / ShuttingDown / Draining / Stopped / ForcefulShuttingDown；ShutdownMode 区分 Graceful（等待在途操作完成）与 Forceful（立即终止）；DrainStatus 是结构化 drain 快照（当前状态 + 忙计数 + 是否正在 drain + 待处理项描述）。由 daemon 的 ShutdownHandle 实现（daemon 启动时创建；gateway 侧为转发包装），llm、session 等经 ShutdownSignal 消费。关停流程见 [daemon 模块](../daemon/README.md)。

### LlmState / ToolExecState / ChildSessionState / ChildCompletionStatus / SessionActivityDimensions / SessionExecStatus

Session 的四维执行状态族（session↔gateway 契约）。ConversationSession 的运行状态由四个独立维度组合判定，整体状态供 Gateway 做消息分派决策（Busy 时排队，Idle/Waiting 时立即分发）。

- **LlmState**：LLM 交互状态——Idle / Requesting / Receiving。
- **ToolExecState**：工具执行状态——Pending / RunningForeground（阻塞会话）/ RunningBackground（不阻塞）/ Completed / Failed / Terminated / TimedOut。
- **ChildSessionState**：子 Session 状态——Running / Completed / Terminated / Errored。
- **ChildCompletionStatus**：子 Session 完成状态（announce 时对 ChildSessionState 的快照）——Completed / Errored / Terminated。
- **SessionActivityDimensions**：四维活跃快照（`llm_active` / `foreground_tool_active` / `background_tool_active` / `child_active`）。
- **SessionExecStatus**：整体执行状态——Idle / Waiting / Busy。Idle 与 Busy 由四维标志派生；Waiting 是 `llm_active` 与 `foreground_tool_active` 均为 false（后台任务 / 子 Session 仍可活跃）的特殊态，仅由 `sessions_yield` 主动让出 turn 产生。

状态模型与流转规则见 [session session-execution](../session/session-execution.md)。

### MediaStoreError

[MediaStoreAccess](core-traits.md#mediastoreaccess)（common DI trait）的错误类型——NoPath（引用无本地路径）/ FileNotFound / Io / Other。

### BackgroundTask / TaskState / RunningTaskInfo / CompletionNotification / NotificationPriority / BackgroundTaskError

[TaskManager](core-traits.md#taskmanager)（common DI trait）契约的载荷族，随后台命令任务的生成、监控与任务通知在 tasks（实现方）与 tools、gateway（消费方）、daemon（装配方）之间传递。BackgroundTask、RunningTaskInfo、CompletionNotification 是同一后台任务在生命周期不同时刻的只读视图。后台任务的完整生命周期（生成、超时转后台、卡住告警、终态通知注入、输出文件回收）见 [tools/background-tasks](../tools/background-tasks.md)，通知注入规则见 [session 消息注入](../session/session-execution.md)。

**BackgroundTask**——一个后台任务句柄，TaskManager 的生成/接管/查询方法返回：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | string | 任务唯一标识 |
| `command` | string | 原始 shell 命令 |
| `state` | TaskState | 当前生命周期状态 |
| `output_path` | string | 该任务输出文件的本地路径 |

**TaskState**——后台任务生命周期状态：Running（携带 `is_backgrounded`，标记是否经自动/手动后台化产生；运行中任务参与卡住检测）、Completed（携带 `exit_code`）、Failed（携带非零 `exit_code`）、Killed（被外部终止）。非 Running 即为终态。

**RunningTaskInfo**——运行中任务摘要，在途任务列表的元素：`task_id`（任务标识）、`command`（原始命令）、`elapsed_secs`（已运行秒数）。供消费方向下一轮对话注入运行中任务摘要。

**NotificationPriority**——后台任务通知的投递优先级（终态通知与卡住告警共用）：Later（择机稍后注入）/ Next（下一轮对话立即注入）/ Now（最高，先于用户输入立即注入）。排序 Now > Next > Later。

**CompletionNotification**——待投递的后台任务通知（终态完成/失败/被终止，或运行中检测到卡住告警）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `task_id` | string | 涉及的任务标识 |
| `command` | string | 原始命令 |
| `state` | TaskState | 通知产生时任务的状态（终态，或卡住告警时的运行中状态） |
| `output_path` | string | 输出文件本地路径 |
| `priority` | NotificationPriority | 投递优先级 |
| `summary` | string | 人类可读摘要 |
| `suggestion` | string? | 基于任务结果/告警的可选建议 |

由任务管理接口清空并返回，进入会话统一消息队列按优先级注入（见 [session 消息注入](../session/session-execution.md)）。

**BackgroundTaskError**——后台任务操作错误：SpawnFailed（启动失败）、NotFound（任务不存在）、NotRunning（任务非运行态）、Io（IO 错误）。

### SpawnValidationResult / SpawnError

[SpawnValidator](core-traits.md#spawnvalidator)（common DI trait）契约的载荷族，随子会话生成校验在 session（实现方 + 消费方）、tools 与 daemon（消费方）之间传递。

**SpawnValidationResult**——一次成功的前置校验产出：

| 字段 | 类型 | 说明 |
|------|------|------|
| `agent_id` | string | 目标 agent 标识 |
| `effective_max_spawn_depth` | int | 子会话可用的最大生成深度 |
| `spawn_timeout` | int? | 子 Agent 执行时长上限（秒），按目标 agent 配置回退全局默认解析（spawn 显式参数由消费方覆盖） |
| `timeout_warning_secs` | int? | 超时告警时长（秒），同上 |
| `timeout_notify_interval_ratio` | float? | 循环告警间隔比例（相对 timeout_warning），取值 [0.1, 2.0]，默认 0.5 |

目标 agent 的完整配置档案不进入本共享结构（含模型），仅其派生参数（上方字段）进入；创建子会话所需的完整目标配置由提供方（session）内部获取。`agent_id` 为目标 agent 标识——输入可空时由前置校验解析，无法解析则返回 [SpawnError](#spawnvalidationresult--spawnerror)。本结构的超时/告警字段为 spawn 生效值，与 [AgentConfigInfo](#agentconfiginfo) 的同源配置字段（agent 配置原始值）对应。

**SpawnError**——子会话生成校验的错误（SpawnValidator 两步的统一错误载体）：DepthExceeded（超出生成深度上限）、MaxChildrenReached（达到最大并发子会话数）、AgentNotAllowed（目标 agent 不在 allowlist）、AgentIdRequired（配置要求 agentId 但未提供）、ConfigNotFound（目标 agent 配置缺失）、Permission（权限被拒，载荷复用 [SpawnPermissionError](#risklevel--permissionevalresponse--callerinfo--permissiondenied--spawnpermissionerror)，不重复定义拒绝载荷）。

### AuditLogEntry / AuditDisposition / AuditLogFilter

[AuditLogger](core-traits.md#auditlogger)（common DI trait）契约的载荷族，随权限审计的写入与查询在 permission（实现方 + 消费方）与 daemon、tools（消费方）之间传递。

**AuditLogEntry**——单条审计日志：

| 字段 | 类型 | 说明 |
|------|------|------|
| `timestamp` | string | 事件时间（ISO 8601） |
| `agent_id` | string | 涉及操作的 agent |
| `tool_name` | string | 请求类型名（操作维度，如 file / exec / network；非 ToolRegistry 工具名） |
| `operation` | string | 操作描述（如 `write <path>`、命令文本） |
| `reason` | string | 处置的人类可读原因 |
| `risk_level` | [RiskLevel](#risklevel--permissionevalresponse--callerinfo--permissiondenied--spawnpermissionerror) | 操作风险级别 |
| `session_mode` | [SessionMode](#reasoninglevel--agentrole--sessionmode)? | 事件发生时的会话模式，可选 |
| `disposition` | AuditDisposition | 最终处置（批准/拒绝） |

**AuditDisposition**——审计处置枚举：Approved（被批准）、Rejected（被拒绝）。

**AuditLogFilter**——审计条目查询过滤条件（各字段均可选，为空表示不过滤）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `agent_id` | string? | 仅返回该 agent 的条目 |
| `disposition` | AuditDisposition? | 仅返回该处置（批准/拒绝）的条目 |
| `since` | string? | 仅返回 timestamp ≥ 此值（ISO 8601）的条目 |
| `until` | string? | 仅返回 timestamp ≤ 此值（ISO 8601）的条目 |

由审计查看工具（tools）经 [AuditLogger](core-traits.md#auditlogger) 的查询能力消费。

### AgentConfigInfo

[AgentConfigLookup](core-traits.md#agentconfiglookup)（common DI trait）的返回类型——按 agent_id 查得的该 agent 配置子集（子 Agent 生成与子会话超时告警相关的最小集合，非完整 agent 配置档案）。

| 字段 | 类型 | 说明 |
|------|------|------|
| `subagents_model` | [ModelSpec](#modelspec)? | 该 agent 配置的子 Agent 模型覆盖，未配置为空 |
| `timeout_warning` | int? | 子 Agent 执行时长告警阈值（秒），空表示回退全局默认 |
| `timeout_notify_interval_ratio` | float? | 循环告警间隔比例（相对 timeout_warning），取值 [0.1, 2.0]，默认 0.5 |

### ModelSpec

agent 模型规格——主模型 + 回退模型列表。纯值数据，无单一领域归属，被 config（作为 agent 配置的模型字段产出）与 cli（agent info 管理协议）等 2+ 模块消费（也作为 [AgentLookup](core-traits.md#agentlookup) 的模型查询返回类型、[AgentConfigInfo](#agentconfiginfo) 的子 Agent 模型覆盖字段）。

| 字段 | 类型 | 说明 |
|------|------|------|
| `primary` | string | 主模型标识，始终最先尝试 |
| `fallback` | list(string) | 回退模型标识列表，主模型不可用时按序尝试（实际回退选择逻辑在 LLM 层） |

### ResolvedAgentConfig / SubagentsConfig / MemoryConfig

Agent 配置档案是 Agent 模块的核心数据对象（语义与需求见 [agent §F1/F6](../../requirements/agent.md)）：Config 模块按注册清单加载各 Agent 的 `config.json`、补齐默认值后产出 **ResolvedAgentConfig**，交由 Daemon 填充 AgentRegistry 作为运行时只读查询数据源，被 Session / Permission / System Prompt / Tools / Skills / Gateway / Daemon 等多个下游模块消费。因被 2+ 模块消费、无单一领域归属，其类型在此唯一定义；字段与 `config.json` 原始字段的对应关系、加载流程见 [agent/agent-config.md](../agent/agent-config.md)。

**ResolvedAgentConfig 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | string | Agent 唯一标识 |
| `name` | string | 显示名称，未配置时回退 `id` |
| `parent_id` | string? | 父 Agent ID（创建时写入、运行时不变；`null` 表示无父 Agent） |
| `model` | [ModelSpec](#modelspec)? | 默认模型与回退列表 |
| `workspace` | string? | 工作目录 |
| `agent_dir` | string? | Bootstrap 文件所在目录 |
| `bootstrap_mode` | [BootstrapMode](#会话注入辅助类型) | 身份加载模式 |
| `no_bootstrap` | bool | 是否声明不使用任何 Bootstrap 文件（见 [agent §F1](../../requirements/agent.md)、[system_prompt §F8](../../requirements/system_prompt.md)） |
| `skills` | list(string) | 技能白名单，`["*"]` 或空表示不限制 |
| `tools` | list(string) | 工具白名单，`["*"]` 或空表示不限制 |
| `disallowed_tools` | list(string) | 工具黑名单，与白名单交集时黑名单优先 |
| `subagents` | SubagentsConfig | 子 Session 创建控制参数 |
| `memory` | MemoryConfig? | 记忆子系统 per-agent 覆盖，未声明为 `null`（回退全局默认） |
| `hooks` | list([HookConfig](#hookconfig--hookparams--hooktype)) | run-health hook 审查配置 |
| `parallel_tool_calls` | bool | 是否允许并行工具调用 |

ResolvedAgentConfig 是**静态配置视图**——所有字段已完成解析（默认值补齐），但不含运行时派生能力（权限过滤后的工具清单、运行模式决定的工具范围等由下游消费模块按需派生，见 [agent-registry §架构](../agent/agent-registry.md)）。`tools` / `skills` 保留静态字面值（`["*"]` 即字面 `["*"]`，不展开为具体工具名）。

**SubagentsConfig 字段**（子 Session 创建控制，由 agent 配置定义、session 的 spawn 流程消费）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `allow_agents` | list(string) | 允许 spawn 的目标 Agent ID 白名单；`["*"]` 表示不限制，空数组表示禁止 spawn 任何子 Session（见 [agent §F9](../../requirements/agent.md)） |
| `require_agent_id` | bool? | spawn 时是否必须显式指定 agentId，默认 `false` |
| `max_spawn_depth` | int? | 本 Agent 允许的最大子孙层级数（0 表示禁止 spawn） |
| `max_children` | int? | 最大并发活跃子 Session 数 |
| `timeout` | int? | 子 Session 硬超时（秒），未指定回退全局默认 |
| `timeout_warning` | int? | 子 Session 超时预警时长（秒），未指定回退全局默认 |
| `timeout_notify_interval_ratio` | float? | 预警后循环通知间隔比例（相对 timeout_warning），取值 [0.1, 2.0]，默认 0.5 |
| `model` | [ModelSpec](#modelspec)? | 子 Session 默认模型覆盖 |

**MemoryConfig**：记忆子系统的 per-agent 覆盖结构（字段级覆盖全局 `memory.json`）。作为 ResolvedAgentConfig 的 `memory` 字段类型在此收录；其字段结构与默认值以 [memory/config](../memory/config.md) 为权威定义，本文档不重复。

### 消息/内容辅助类型

- **ContentBlockType**：ContentBlock 的块类型分类枚举（Text / Thinking / ToolUse / ToolResult / Image / Audio / File），用作 [StreamEvent](#streamevent) 的 `block_type` 与流式渲染的分类。
- **MessageType**：入站消息类型枚举（text / image / file / audio / post），为 [NormalizedMessage](#normalizedmessage) 字段。
- **MediaType**：媒体类型枚举（image / file / audio），为 [MediaRef](#normalizedmessage) 字段。
- **ProcessError**：Processor Chain 入站/出站处理的错误类型。
- **AdapterError**：IM Adapter 操作的错误类型，为 [IMPlugin](core-traits.md#implugin) 契约的错误载荷。

### 会话/注入辅助类型

- **SessionRole**：会话角色枚举（Main 主 Agent Session / Sub 子 Session），为 [FragmentContext](#fragmentcontext) 字段。
- **SectionType**：[PromptFragment](#promptfragment) 的片段类型枚举（bootstrap 文件 / 工具列表 / skill 清单 / 长期记忆）。
- **BootstrapMode**：身份加载模式枚举（Minimal 精简 / Full 完整），取自 agent 配置的 `bootstrapMode`，为 [FragmentContext](#fragmentcontext) 字段。
- **PlanPhase**：[PlanState](#planstate) 的阶段枚举（Research / Design / Review / FinalPlan）。
- **RequestContext**：当前入站消息的元数据（发送者、渠道、时间戳、聊天名），由 Gateway 在每次 LLM 调用前写入 session，供动态层构建使用；置于 common 以避免 session→gateway 反向依赖。
- **InjectionParams**：system prompt 注入链的参数契约。设计文档要求的必选输入为 agent_id、ToolRegistry 引用、Session 角色、身份加载模式四项；另含既有参数 session_id、overrides、activated_skills。由 SessionManager → session → System Prompt Builder 传递。
- **TurnCounter**：会话轮次计数器（工具结果计入轮次）。
- **PendingMessage**：待处理消息（未最终确认、等待注入对话的消息），为 [SessionLookup](core-traits.md#sessionlookup) / [SlashSessionQuery](core-traits.md#slashsessionquery) 「向统一消息队列推送」的载荷。

### Slash 执行辅助类型

- **SlashContext**：斜杠指令执行上下文——`command` / `sender_id` / `session_id` / `channel`，为 [SlashHandler](core-traits.md#slashhandler) 的入参。
- **SystemAppendAction**：`/system` 追加动作——Add（追加指令）/ Clear（清空追加），为 [SlashResult](#slashresult) 变体载荷。
- **ReplyAction**：[SlashResultExecutor](core-traits.md#slashresultexecutor) 产出、Gateway 分派的回复动作——Reply（内容块回复）/ TriggerCompact（触发压缩）/ Nothing。

### LLM/流式/中间件辅助类型

- **LLMError**：LLM 操作的错误类型，为 [LlmCaller](core-traits.md#llmcaller) 的错误载荷（[StreamEvent](#streamevent) 的 Error 事件载荷为错误消息文本）。
- **ErrorKind**：LLM 错误的分类（用于重试策略判定）。
- **StreamDone**：流式完成通知载荷（model + usage），为 [StreamingSink](core-traits.md#streamingsink) 的完成通知参数。
- **MiddlewareContext** / **MiddlewareError**：[OutboundMiddleware](core-traits.md#outboundmiddleware) 契约的上下文与错误载荷。

### 工具契约载荷族

围绕 [Tool](core-traits.md#tool-trait)、[ToolRegistry](core-traits.md#toolregistry)、[ToolRegistrar](core-traits.md#toolregistrar)、[ToolRegistryQuery](core-traits.md#toolregistryquery)、[ToolSession](core-traits.md#toolsession)、[ToolExecutor](core-traits.md#toolexecutor) 的载荷类型：

- **ToolFlags**：工具运行时标记（是否只读 / 破坏性 / 昂贵 / 默认延迟加载 / 并发安全）。
- **ToolDescriptor**：工具摘要信息（name / group / summary / detail / input_schema / flags），用于 system prompt 生成。
- **ToolBox**：桥接 `Tool` 与类型擦除注册的包装（`Arc<dyn Tool>`）。
- **RegistryError**：工具注册表操作错误（区别于 ToolRegistrarError）。
- **ToolRegistrarError**：Registrar 级注册错误（冲突报告、内部失败）。
- **ToolContext**：工具调用时的运行时上下文（含 session、workdir、session_mode、media_store 等句柄）。
- **ToolResult**：工具调用链路的结果结构（data / new_messages / context_modifier）。与 [ContentBlock](#contentblock) 的 ToolResult **变体**（出站内容块）同名但不同物。
- **ToolMessage**：注入 agent 上下文的消息。
- **ContextModifier**：工具结果中携带的、用于在工具执行后修改会话上下文的动作（[ToolResult](#工具契约载荷族) 字段）。
- **ToolCallError**：工具执行错误。
- **PromptGenerationContext** / **WorkdirContext**：`generate_prompt` / workdir 上下文构建的输入。
- **ReadRange** / **ToolProgress**：文件读取范围与工具进度快照，为 [ToolSession](core-traits.md#toolsession) 契约载荷。
- **PendingToolCall** / **ToolCallDispatcher**：多工具并行调度的调用描述与调度器（gateway 与 tools 共用）；**DispatchGroup** 为调度分组枚举。其执行抽象 ToolExecutor 为 DI trait，见 [core-traits](core-traits.md#toolexecutor)。
- **FileMutexMap**：同文件并发写的按路径互斥映射（gateway 与 tools 共用）；**TryAcquireResult** 为其非阻塞获取结果。

### 其他契约辅助类型

- **PromptOverrides** / **ModeTransition** / **DynamicPromptContext**：[SystemPromptBuilder](core-traits.md#systempromptbuilder) 与 [DynamicPromptBuilder](core-traits.md#dynamicpromptbuilder) 契约的覆盖项、模式切换信号与动态构建上下文。
- **AgentToolsConfig**：Agent 工具白/黑名单配置，为 [AgentToolsConfigQuery](core-traits.md#agenttoolsconfigquery) 的返回类型。
- **ConditionalSkillMatch**：条件技能匹配结果，为 [SkillListingProvider](core-traits.md#skilllistingprovider) 的载荷。

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
  → SessionManager 以稳定路由键（platform + sender_id + peer_id + account_id）查找/创建 session → LLM 对话 / SlashDispatcher（session_key 仅作日志追踪，不参与路由查找）
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
[IM Adapter 渲染] — 按块类型选择渲染策略，输出平台原生格式：
    - 批量模式：一次性渲染全部 ContentBlock[]
    - 流式模式：消费 [StreamEvent](#streamevent) 增量事件，Text 块逐行缓冲输出，非文本类块等 BlockEnd 全块就绪后一次渲染
  ↓
[中间件插入点] — Gateway 可在渲染完成后、发送前插入审计、频率限制等中间件。中间件为 Gateway 内部的拦截链，具体中间件类型和注册机制由 Gateway 管理，不在 shared-types 范围
  ↓
IM Adapter 发送到目标平台
  ↓
[Gateway 出站日志] — 发送成功后记录完整 ProcessedMessage 的出站历史
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
Gateway — 先检查 message_type：含媒体消息做媒体可得性校验（不可得 → 提示「该消息内容无法获取」经简化出站路径发送、流程结束；可得 → 按类型构造上下文形态后与文本同链路继续，形态规则见 [im_adapter media-store](../im_adapter/media-store.md)）；对话消息从 content_blocks[0] 取 Text 内容做路由决策（/ 开头 → 斜杠指令；否则 → LLM 对话），从 metadata 取 session_key 传入 SessionManager（仅作日志追踪；SessionManager 以稳定路由键 platform+sender_id+peer_id+account_id 做查找）
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
IM Adapter 渲染（消费 content_blocks + metadata[dsl_result]）→ 发送；发送成功后 Gateway 写出站历史
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
SessionManager 触发构建（新 Session / 恢复 archive / compaction），传 Session 角色（主/子）
  ↓
System Prompt Builder 构建 FragmentContext（agent_id + session_role + bootstrap_mode + bootstrap_dir）
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

### ContentSegment / 内容段落解析

ContentSegment 的解析与消费嵌入在批量渲染路径中，由各适配器在渲染 ContentBlock::Text 时触发：

```
ContentBlock::Text 变体文本（其他 ContentBlock 变体不经本原语）
  ↓
内容段解析 → ContentSegment[]（Markdown / Hr / CodeBlock）
  ↓
各适配器按变体 emit 平台格式 — 飞书卡片富文本组装 / 终端 ANSI 文本
```

ContentSegment 的生命周期：common 解析产出 → 各适配器消费并按内容段渲染 → 随 RenderedOutput 产出后销毁，不进入 Session 或日志持久化。

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

### CompactionResult / CompactionError

```
session 模块执行压缩 → 经 SlashEffectExecutor.execute_compact → Result<CompactionResult, CompactionError>
  ↓
Gateway 按结果回发压缩摘要或错误提示
```

### InternalRequest / InternalMessage / SystemBlock / ToolDefinition

```
Session 构造 InternalRequest（messages + system 两段 + tools + reasoning_level）
  ↓
LlmCaller 抽象接口 → LLM 协议层映射为供应商原生请求
  ↓
（流式）StreamEvent 流 /（非流式）UnifiedResponse
```

### CommunicationConfig / CommunicationCheckResult / CommunicationError

```
agent 配置层产出 CommunicationConfig（按 Agent）
  ↓
session spawn 流程取子/父 Agent 的 CommunicationConfig 做通信权限校验
  ↓
CommunicationCheckResult（允许）/ CommunicationError（拒绝或缺失）
```

### RiskLevel / PermissionEvalResponse / CallerInfo / PermissionDenied / SpawnPermissionError

```
消费方（session tools / gateway）发起权限校验或评估请求
  ↓
经 common 权限 trait（PermissionEvaluator / PermissionChecker / ApprovalSubmission）
  ↓
返回对应载荷（PermissionEvalResponse / PermissionDenied / SpawnPermissionError）
  ↓
被拒且需审批 → 经 ApprovalSubmission 提交，携带 CallerInfo + RiskLevel
```

### HookConfig / HookParams / HookType

```
agent 配置定义 hook 列表（config.json）
  ↓
session 的 Hook 审查器（HookReviewer）按 HookType 并行调用、按 HookParams 阈值判定
  ↓
任一 hook 标记异常 → session 判 unhealthy
```

### ShutdownState / ShutdownMode / DrainStatus

```
daemon 的 ShutdownHandle 维护 ShutdownState（含 graceful→forceful 升级）
  ↓
ShutdownSignal 消费方（llm 等）查询状态/忙计数/drain 快照（DrainStatus）
```

### LlmState / ToolExecState / ChildSessionState / ChildCompletionStatus / SessionActivityDimensions / SessionExecStatus

```
LLM / 工具 / 子 Session 维度状态变化
  ↓
Session 汇总为四维快照 SessionActivityDimensions
  ↓
派生整体状态 SessionExecStatus（Idle / Waiting / Busy）
  ↓
Gateway 据此做消息分派决策（Busy 排队、Idle/Waiting 立即分发）
```

### MediaStoreError

```
Gateway 调用 MediaStoreAccess 解析 MediaRef
  ↓
成功 → 绝对路径 / 失败 → MediaStoreError
```

### BackgroundTask / TaskState / RunningTaskInfo / CompletionNotification / NotificationPriority / BackgroundTaskError

```
Agent 经 bash 工具显式后台化，或命令超时自动转后台
  ↓
TaskManager 生成 / 接管后台任务 → BackgroundTask（Running）
  ↓
任务运行（输出写入 output_path）；下一轮对话开始时取在途任务摘要（RunningTaskInfo）注入
  ↓
运行中检测到卡住（交互式提示）→ 生成任务通知（state=Running，priority=Next，带建议）
  │
任务到达终态（Completed / Failed / Killed）→ 生成任务通知（带 NotificationPriority）
  ↓
TaskManager 取出待处理通知 → 会话统一消息队列按优先级注入（见 session 消息注入）
  ↓
Session 销毁 → TaskManager 回收该 session 全部终态任务的输出文件与句柄
```

### SpawnValidationResult / SpawnError

```
子会话管理工具（tools）向 SpawnValidator 发起前置校验（父 session_id + 目标 agent_id）
  ↓
通过前置校验（深度 / 并发 / allowlist / agent 解析）→ SpawnValidationResult（目标 agent 标识 + 派生参数）
  │  失败 → SpawnError（前置错误）
  ↓
SpawnValidator 权限校验（前置校验产物）→ 经 PermissionChecker 完成权限判定
  ↓
权限通过 → 子会话工具据此创建子会话
  │  权限被拒 → SpawnError 的权限变体（复用 SpawnPermissionError）
```

### AuditLogEntry / AuditDisposition / AuditLogFilter

```
权限引擎（或审批流）对危险操作作出批准/拒绝处置
  ↓
构造 AuditLogEntry（操作内容 + RiskLevel + 处置 + 会话模式）
  ↓
AuditLogger 记录 → permission 的文件日志实现追加落盘
  ↓
审计查看工具按 AuditLogFilter 查询、返回匹配条目
```

### AgentConfigInfo

```
子会话工具（sessions_spawn / sessions_yield）需要所属 agent 的最小配置
  ↓
AgentConfigLookup 按 agent_id 查询 → AgentConfigInfo（子 Agent 模型规格 + 超时告警参数）
```

### ModelSpec

```
agent 配置解析（config）产出 ModelSpec（主模型 + 回退列表）
  ↓
随 agent 配置聚合传递：
  ├── AgentLookup / AgentRegistryQuery 按 agent_id 返回该模型规格（system_prompt、gateway、daemon 等消费）
  └── 经 AgentConfigInfo.subagents_model 提供子 Agent 模型覆盖（消费方：子会话管理工具等）
  ↓
消费方读取主模型与回退列表
```

### 会话/工具/斜杠/LLM 等辅助契约类型

这些辅助类型不构成独立的跨模块流动，而作为其宿主契约的载荷随调用传递：

- **会话/注入辅助**（SessionRole / SectionType / BootstrapMode / PlanPhase / RequestContext / InjectionParams / TurnCounter / PendingMessage）：随 session 构建、system prompt 注入、消息排队等流程传递。
- **工具契约载荷族**：随 Tool / ToolRegistry / ToolRegistrar / ToolSession / ToolExecutor 的注册与调用传递。
- **Slash 执行辅助**（SlashContext / SystemAppendAction / ReplyAction）：随斜杠指令分派与 SideEffectContext 执行传递。
- **LLM/流式/中间件辅助**（LLMError / ErrorKind / StreamDone / MiddlewareContext / MiddlewareError）：随 LlmCaller / StreamingSink / OutboundMiddleware 调用传递。
- **system prompt 辅助**（PromptOverrides / ModeTransition / DynamicPromptContext）：随 SystemPromptBuilder / DynamicPromptBuilder 调用传递。
- **其他**（AgentToolsConfig / ConditionalSkillMatch）：随 AgentToolsConfigQuery / SkillListingProvider 调用传递。

## 模块关系

### NormalizedMessage

- **生产者**：IM Adapter 各平台插件（入站解析）——包括飞书、Discord、Telegram 等 IM 平台的 Adapter，以及 CLI 模块的 TerminalAdapter
- **消费者**：Processor Chain 入站（读取 NormalizedMessage 做内容标准化和 session_key 计算，产出 [ProcessedMessage](#processedmessage)）
- **无关**：LLM Provider（不接触 NormalizedMessage，只消费 ContentBlock[]）、Session（通过 Gateway 间接消费路由字段，不直接接触 NormalizedMessage）、Slash Command（斜杠指令消息本身经入站链归一化为 NormalizedMessage，但 SlashDispatcher 消费的是处理后的消息文本，不直接消费 NormalizedMessage 结构）

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

### ContentSegment / 内容段落解析

- **生产者**：common 自身（配套纯解析函数把文本切分为内容段序列）
- **消费者**：im_adapter（飞书平台渲染路径——按内容段类型组装卡片元素）、cli（TerminalRenderer——按内容段类型输出 ANSI 文本）
- **无关**：LLM Provider（不接触渲染原语）、Processor Chain 出站（渲染原语在出站链之后）、StreamingRenderer 系流式渲染原语（与本原语分属批/流两条渲染路径，代码块边界识别各自独立）、Session 持久化（中间产物，不进 checkpoint）

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
- **消费者**：Gateway（入站：消费 content_blocks 做路由决策（session_key 用于日志追踪，路由查找用稳定路由键）+ metadata.message_type 做分型路由判断；出站：消费 content_blocks + metadata.dsl_result 做出站日志后传给 IM Adapter）、IM Adapter（消费 content_blocks + metadata.dsl_result 渲染为平台格式并发送）、CLI TerminalRenderer（同 IM Adapter，渲染为 ANSI 终端文本）
- **无关**：NormalizedMessage（入站方向的上游产物，经 Processor Chain 处理后产出 ProcessedMessage，两者是不同的两个结构）、Session（Gateway 通过 ProcessedMessage 中的 session_key 找到 Session，但 Session 不直接操作 ProcessedMessage）、LLM Provider（不接触 ProcessedMessage，只产出 ContentBlock[]）

### SlashResult

- **生产者**：SlashDispatcher（各 Handler 返回 SlashResult 变体）
- **消费者**：Gateway（构造 SideEffectContext 并触发 SlashResult 执行，回复内容进入出站 Processor Chain）
- **间接消费者**：Permission 模块（Exec 变体执行前校验）、CLI（通过 Gateway 间接消费斜杠指令回复）
- **无关**：LLM Provider（不参与斜杠指令，不接触 SlashResult）、Session（SlashResult 通过 SideEffectContext 操作 Session，但 Session 不直接消费 SlashResult 结构）

**与入站链的关系**：斜杠指令消息与其他入站消息走同一条入站通路——经 IM Adapter 归一化为 [NormalizedMessage](#normalizedmessage)、Processor Chain 入站计算 session_key 并清洗内容，再由 Gateway 按 `/` 前缀路由到 SlashDispatcher。SlashResult 本身由指令 Handler 产出，不由入站链产生。

### FragmentContext

- **生产者**：system_prompt 模块（System Prompt Builder 构建，Session 角色由 SessionManager 在触发构建时传入）
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

### CompactionResult / CompactionError

- **生产者**：session 模块（经 SlashEffectExecutor）→ **消费者**：Gateway（回发结果/错误）
- **无关**：LLM Provider、IM Adapter；压缩配置（阈值/保留区/熔断/摘要模型）语义归 session 模块，本文档不定义

### ReasoningLevel / AgentRole / SessionMode

- **ReasoningLevel 生产者**：slash 模块（`/reasoning` 指令经 SlashEffectExecutor 设置）、config 的 `llm.reasoning_level` 全局默认档位；**消费者**：session / LLM 协议层（映射为原生参数）
- **AgentRole**：随 Agent 配置 / 会话创建确定；**消费者**：session、gateway（会话角色门控）
- **SessionMode 生产者**：slash 模块（`/mode`）、mode 模块；**消费者**：session、permission、gateway、system_prompt（经 SessionModeQuery 与共享类型）
- **无关**：IM Adapter、Processor Chain

### InternalRequest / InternalMessage / SystemBlock / ToolDefinition

- **生产者**：Session（构造请求）、LLM Client 的缓存适配器（产出 system_blocks 与工具缓存标记）
- **消费者**：LLM 协议层（映射为供应商原生请求）
- **无关**：消息出站链路的共享类型（本族是 LLM 调用载荷，不进入出站 Processor Chain）

### CommunicationConfig / CommunicationCheckResult / CommunicationError

- **生产者**：agent 配置层（CommunicationConfig）
- **消费者**：session 的 spawn 流程（通信权限校验）
- **无关**：LLM Provider、IM Adapter、Processor Chain

### RiskLevel / PermissionEvalResponse / CallerInfo / PermissionDenied / SpawnPermissionError

- **生产者**：各权限 trait 实现方（permission / gateway / daemon 包装）
- **消费者**：各权限 trait 消费方（session tools / gateway）
- **无关**：LLM Provider、IM Adapter、Processor Chain

### HookConfig / HookParams / HookType

- **生产者**：agent 配置（config.json）
- **消费者**：session 的 Hook 审查器（HookReviewer，run-health 质量门禁）
- **无关**：LLM Provider（hook 调用隔离于主对话）、IM Adapter

### ShutdownState / ShutdownMode / DrainStatus

- **生产者/维护方**：daemon 的 ShutdownHandle
- **消费者**：llm 等经 ShutdownSignal 查询关停状态与 drain 快照
- **无关**：IM Adapter、Processor Chain、permission

### LlmState / ToolExecState / ChildSessionState / ChildCompletionStatus / SessionActivityDimensions / SessionExecStatus

- **生产者**：session（维护四维状态并派生 SessionExecStatus；ChildCompletionStatus 在子 Session announce 时产出）
- **消费者**：gateway（消息分派决策）、session 自身（分派/归档判定）；workflow（验收闸门看四维全 false）
- **无关**：LLM Provider、IM Adapter 入站链

### MediaStoreError

- **生产者**：MediaStoreAccess 实现方（im_adapter 的 MediaStore）
- **消费者**：MediaStoreAccess 消费方（gateway 等）
- **无关**：Processor Chain、LLM Provider

### BackgroundTask / TaskState / RunningTaskInfo / CompletionNotification / NotificationPriority / BackgroundTaskError

- **生产者**：tasks（TaskManager 实现 BackgroundTaskManager；生成/接管任务时产出 BackgroundTask，卡住告警/任务终态时产出 CompletionNotification）
- **消费者**：tools、gateway（经 [TaskManager](core-traits.md#taskmanager) 生成/查询/终止任务、取出任务通知）、daemon（装配注入）；任务通知经会话统一消息队列注入
- **无关**：LLM Provider、IM Adapter

### SpawnValidationResult / SpawnError

- **生产者**：session（SpawnController 前置校验产出 SpawnValidationResult / SpawnError）
- **消费者**：session 的子会话管理工具（据此创建子会话并处理错误）、daemon（消费/装配）
- **无关**：LLM Provider、IM Adapter、Processor Chain

### AuditLogEntry / AuditDisposition / AuditLogFilter

- **生产者**：permission（权限引擎/审批流构造 AuditLogEntry）
- **消费者**：permission（文件日志实现落盘与查询）、tools（审计查看工具按 AuditLogFilter 查询条目）、daemon（装配注入）
- **无关**：LLM Provider、IM Adapter、Processor Chain

### AgentConfigInfo

- **生产者**：agent（AgentRegistry 实现 AgentConfigLookup 产出）
- **消费者**：session 的子会话工具（sessions_spawn / sessions_yield）、daemon（装配注入）
- **无关**：LLM Provider、IM Adapter

### ModelSpec

- **生产者**：config（agent 配置解析产出）
- **消费者**：system_prompt、gateway、daemon（经 AgentLookup / AgentRegistryQuery 查询模型规格）、cli（agent info 管理协议）、以及经 [AgentConfigInfo](#agentconfiginfo) 读取子 Agent 模型覆盖（子会话管理工具）
- **无关**：IM Adapter、Processor Chain、LLM Provider（回退链由 daemon 装配，不经 common 传递）

### ResolvedAgentConfig / SubagentsConfig / MemoryConfig

- **生产者**：config（按注册清单加载 `config.json`、补齐默认值产出 ResolvedAgentConfig）
- **消费者**：agent（AgentRegistry 以 `agent_id` 为键存储并提供只读查询）、daemon（装配填充注册表）、gateway（经 AgentRegistry 查询）、session / permission / system_prompt / tools / skills（经 AgentRegistry 查询消费配置字段）
- **无关**：LLM Provider、IM Adapter、Processor Chain

### 会话/工具/斜杠/LLM 等辅助契约类型

- **生产者/消费者**：随各自宿主契约的实现方与消费方（见 [core-traits](core-traits.md) 模块关系）；本身无独立流动。
- **无关**：LLM Provider、IM Adapter 入站链（MessageType/MediaType/AdapterError 归属 IM 契约，ContentBlockType/ProcessError 归属 Processor 契约）