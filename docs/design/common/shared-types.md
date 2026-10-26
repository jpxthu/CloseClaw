# 共享类型

## 概述

共享类型是跨模块传递的纯数据结构，被 2 个及以上模块共同消费。每个共享类型在本文档中唯一定义，各业务模块文档通过引用指向此处，不在自身文档中重复描述字段结构。

> **本文档是 common crate 中共享类型的权威清单。** 判定规则与 [STANDARDS.md「common 文档内容准入标准」](../STANDARDS.md)一致：被 2 个及以上模块消费的类型，在此完整定义、代码留 common crate；仅单模块消费的类型不在本清单——代码中如出现在 common crate，应移至对应领域模块的 crate。反之，本文档定义的所有类型，代码中均位于 common crate（或其子 crate）。

本文档不包含 trait 接口定义——核心 trait 见 [core-traits](core-traits.md)。

## 架构

### NormalizedMessage

NormalizedMessage 是平台无关的统一入站消息结构，屏蔽各 IM 平台（飞书、Discord、Telegram 等）和 terminal 渠道的差异。各渠道的 IM Adapter 入站解析产出此结构，Processor Chain 消费（读取内容做标准化和 session_key 计算）。Gateway 消费的是 Processor Chain 产出的 ProcessedMessage；链后补充路由元数据时会读取本结构的少量字段（见下方元数据键表），不参与消息内容处理。

| 字段 | 类型 | 说明 |
|------|------|------|
| `platform` | string | 平台标识，如 `"feishu"`、`"terminal"` |
| `sender_id` | string | 发送者的平台内 ID。terminal 渠道无平台账号体系，恒为 `"owner"`（CLI 渠道的 User 上下文为 Owner，需求 [cli §F1](../../requirements/cli.md)） |
| `peer_id` | string | 会话对端——会话上下文锚点，由插件按平台语义构造，同一会话内取值稳定、不同会话间互不相同；取值粒度随平台形态而定、不设跨平台统一规则（各平台取值语义见平台插件文档）；terminal 渠道取目标 Agent 标识（由 `--agent-id` 指定，未指定时取初始 Agent），使不同 Agent 的对话落到不同会话路由键 |
| `thread_id` | string? | 话题/线程 ID（支持线程的平台上用于线程回复）；不参与 session_key 计算 |
| `reply_ref` | string? | 出站定向引用，可选。插件按平台语义填入、出站时原样消费的平台引用（如话题根消息标识），用于把回复投递回原会话位置。不参与 session 路由。出站传递机制：入站填入后经 Session 上下文存储，出站时由 Gateway 取出传给 IMPlugin 发送，见 [session-lifecycle 出站定向字段](../session/session-lifecycle.md) |
| `account_id` | string | CloseClaw 本地账号标识。IM 渠道由「平台 + 接收方机器人应用 + sender_id」经身份映射得到——同一用户在不同机器人应用下的多个发送者标识归并为同一账号标识（先按应用定位发送者，再跨应用归并）；terminal 渠道恒为字面量 `"owner"`（不经映射，见下方身份映射）。参与 session 路由 |
| `chat_name` | string | 聊天/群名称，由 Adapter 从平台元数据取值填充，可为空 |
| `trace_id` | string | 分布式追踪标识，webhook 到达时生成，用于调试日志链路串联 |
| `message_id` | string | 平台消息标识，由 Adapter 解析时填入，可为空 |
| `content` | string | 消息文本内容。纯媒体消息时可为空 |
| `message_type` | [MessageType](#消息内容辅助类型) | 消息类型；`post` 为含内嵌媒体的富文本消息——展开文本入 content，内嵌媒体入 media_refs |
| `media_refs` | list(MediaRef) | 消息携带的媒体引用列表。Adapter 在入站解析时完成媒体落盘并填充，下游一律以引用消费（媒体能力暂缓，当前不填充，见下方「媒体相关字段的当前状态」） |
| `unavailable_media` | list(string) | 消息引用但未能获得的媒体资源标识列表（下载失败或超出大小上限）。由 Adapter 在入站解析时填充；失败媒体不进入 media_refs、仅记录于此（媒体能力暂缓，当前不填充） |
| `timestamp` | int | 消息发送时间（毫秒级 Unix 时间戳） |

**机器人身份（接收方机器人应用）**：接收方机器人应用标识（bot_app_id）不进入 NormalizedMessage 的字段表，也不写入 metadata。IM Adapter 在入站解析时单独提取、随入站消息旁路提供给 Gateway，供 Gateway 按 [config accounts.json](../config/README.md) 的机器人→Agent 绑定解析目标 Agent；它同时是 `account_id` 身份映射键的一部分（见 [core-traits IdentityResolver](core-traits.md#identityresolver)）。

**引用/回复消息**：引用关系不在当前范围——Adapter 不识别引用、不回查被引用消息正文，被引用内容不进入 Agent 上下文（需求 [im_adapter §F2](../../requirements/im_adapter.md)）。

**消息过滤规则**：text 类型空 content 消息在解析阶段丢弃，不产生 NormalizedMessage；post 类型 content 与 media_refs 均为空时同样丢弃。其余消息正常产 NormalizedMessage（message_type 标记类型，纯媒体消息 content 可为空），由 Gateway 按 message_type 分型：非文本消息在当前范围下记调试日志并回复「暂不支持该类型消息」，不进入对话（需求 [gateway §F3](../../requirements/gateway.md)）；媒体可得性校验与上下文形态属媒体能力启用后的目标形态（见 [im_adapter media-store](../im_adapter/media-store.md)）。

**身份映射**：`account_id` 由 IM Adapter 在解析入站消息时填入。与其他字段（platform、sender_id 等直接从消息 payload 提取）不同，account_id 需查询账户绑定表获取，非直接取值。映射规则：以「平台 + 接收方机器人应用 + sender_id」为键查询账户绑定表，找到对应的 CloseClaw 账户 ID——IM 平台的发送者标识按「应用 × 发送者」隔离，跨应用标识不可直接互换，故映射键必须包含接收方机器人应用。一个账户可绑定多个平台的多个发送者标识。terminal 渠道无需查表，恒为 "owner"——它承载的是系统内置 Owner 账号，与 IM 渠道承载身份映射结果语义一致（都是「本消息归属的 CloseClaw 账号」），差别仅在于 terminal 无平台账号体系、无需求值。详见 [config 模块 accounts.json](../config/README.md)。

**字段填充职责**：各字段由 IM Adapter 入站解析时填充。Processor Chain 不修改 NormalizedMessage 字段——ContentNormalizer 读取 message_type 判断消息类型，仅对 text 类型做 content 文本标准化；SessionRouter 读取 platform/sender_id/peer_id/account_id 计算 session_key。Processor Chain 各 Processor 通过共享的可变 ProcessedMessage 上下文传递数据：SessionRouter 计算 session_key 后直接写入 ProcessedMessage.metadata，ContentNormalizer 随后从同一 NormalizedMessage 读取 content 做标准化后写入 ProcessedMessage.content_blocks。session_key 不写入 NormalizedMessage。

**message_type 与 media_refs**：message_type 由 ContentNormalizer 消费（非 text 跳过标准化）。media_refs 在入站链路仅透传——媒体已由 Adapter 落盘，上下文形态决策由 Gateway 在路由阶段完成（见 [gateway 入站流程](../gateway/inbound-flow.md)）。

**媒体相关字段的当前状态**：多媒体能力按需求暂不设计（见 [im_adapter media-store](../im_adapter/media-store.md)）——message_type 的媒体取值（image / file / audio）以及 media_refs、unavailable_media 当前不产生、不填充；非文本消息当前一律记录调试日志并回复「暂不支持该类型消息」（需求 [gateway §F3](../../requirements/gateway.md)）。下方 MediaRef 与 Post 变体为媒体能力启用时的目标形态，启用前需解冻需求并重新确认字段结构。

NormalizedMessage 引用的子结构：

**MediaRef**：媒体资源的本地存储引用，由 IM Adapter 在入站解析落盘后填充，是下游消费媒体的唯一形态。

| 字段 | 类型 | 说明 |
|------|------|------|
| `key` | string | 平台内资源标识（如飞书 image_key / file_key），用于关联与幂等 |
| `path` | string | 媒体存储中的本地文件路径（相对媒体存储根目录），文件名经安全净化并附加唯一后缀 |
| `media_type` | [MediaType](#消息内容辅助类型) | 媒体类型 |
| `size` | int | 文件大小（字节） |
| `mime` | string | MIME 类型 |

`key` 与出站 [ContentBlock](#contentblock) 非文本变体的 `name` 均表示资源标识，语义等价——命名差异源于入站（MediaRef）与出站（ContentBlock）两套独立结构。落盘、上下文形态与生命周期机制见 [im_adapter media-store](../im_adapter/media-store.md)。

**建模边界**：NormalizedMessage 建模用户主动发送的消息（文本、图片、文件、音频）。卡片交互事件（用户点击消息内嵌按钮/选择器）不在当前范围——Adapter 识别后忽略并记录调试日志，不回复、不进入对话上下文（需求 [im_adapter 飞书 §F2](../im_adapter/platforms/feishu.md)）。

### ContentBlock

ContentBlock 是跨模块传递的结构化内容单元。所有出站内容——LLM 回复和斜杠指令回复——均以 ContentBlock[] 数组形式传递，贯穿 Verbosity 过滤、出站日志记录和平台渲染全链路。入站方向经 Processor Chain 处理后，标准化文本以 ContentBlock::Text 形式放入 [ProcessedMessage](#processedmessage) 的 content_blocks 字段，入站不涉及 ContentBlock 的其他变体。流式场景下，同一份内容以 [StreamEvent](#streamevent) 增量事件形式传递，完整块由消费方按事件边界组装。

ContentBlock 共 7 种变体，按语义和渲染策略分为两类（变体来源：LLM 协议层归一化产出 Text/Thinking/ToolUse 三种为响应体内容，ToolResult 为历史消息中工具回执的归一化形态，详见 [llm protocol-mapping](../llm/protocol-mapping.md)；Image/Audio/File 三种不由 LLM 产出，用于非流式路径的媒体交付）：

**文本类变体**：

| 变体 | 语义 | 渲染行为 |
|------|------|------|
| Text | 文本内容，可含 markdown 格式标记 | 终端输出 ANSI 格式化文本，IM 平台按平台能力输出 markdown 元素 |
| Thinking | LLM 推理过程，用户可选的思考展示 | IM 平台折叠区块；终端不支持折叠交互，默认仅展示思考进行中的状态指示，展示等级 full 时输出思考正文（需求见 [cli §F2](../../requirements/cli.md)）。流式模式下等待全块就绪后一次渲染 |

**非文本类变体**：

| 变体 | 语义 | 渲染行为 |
|------|------|------|
| ToolUse | 工具调用请求，含工具名和参数 | 渲染为工具调用信息展示（终端文本，IM 平台卡片），含工具名与参数概要 |
| ToolResult | 工具执行结果 | 渲染为结果内容展示。终端按行截断（默认保留前 20 行，超出以省略号标注；展示等级 full 时不按行截断），超终端宽度的行再按宽度截断；IM 平台富格式渲染。需求见 [cli §F2](../../requirements/cli.md) |
| Image | 图片引用，含资源标识和访问地址 | IM 平台渲染为图片元素；终端不渲染（媒体能力暂缓，见 [im_adapter media-store](../im_adapter/media-store.md)） |
| Audio | 音频引用，含资源标识和访问地址 | IM 平台渲染为音频元素；终端不渲染（同上） |
| File | 文件引用，含资源标识和访问地址 | IM 平台渲染为文件元素；终端不渲染（同上） |

Image/Audio/File 三个变体结构相同，字段定义：

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | 资源标识，IM 平台渲染媒体元素时使用 |
| `url` | string | 资源访问地址，IM 平台渲染时使用 |

**变体处理规则**：

- **流式渲染差异化**：Text 块逐行缓冲输出（以句末标点或换行符为行边界）；Thinking/ToolUse 块等待全块就绪后一次交付渲染；Image/Audio/File 不以流式事件形式出现，在非流式路径中交由平台格式渲染器处理
- **输出格式决策**：各平台 Renderer 按内容特征选择输出格式（纯文本 vs 富格式），完整规则见 [RenderedOutput §输出格式决策](#renderedoutput)
- **Verbosity 过滤**以单个 ContentBlock 为粒度执行——每个 ContentBlock 到达时按当前 Session 的 verbosity 等级判断其可见性，流式模式下逐块实时过滤。Verbosity 等级定义见 [§VerbosityLevel](#verbositylevel)


### StreamEvent

StreamEvent 是流式输出的统一增量事件，ContentBlock 的流式形态——描述一条消息在生成过程中的边界变化。LLM 模块将各协议 SSE 事件归一化为 StreamEvent 后逐事件对外交付；流式链路上的消费方（VerbosityFilter、流式渲染器）以事件流为输入，实现「块未结束即逐行输出」的增量行为。

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

ContentDelta 是单个 ContentBlock 内部的增量载荷，BlockDelta 事件的载体。共 9 种变体（其中 ToolResultText 与 ImageRef/AudioRef/FileRef 四者为结构预留位，正常链路不出现，见下文约束），每个变体归属唯一的块类型（一个块类型可对应多个增量变体，如 ToolUse 对应 3 个）：

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
| `prompt_tokens` | int | 输入 token 总数（含缓存命中与缓存写入的输入 token） |
| `completion_tokens` | int | 输出 token 数 |
| `total_tokens` | int? | 总 token 数，可选。协议未给出时由消费方按需自行合计 |
| `reasoning_tokens` | int? | 推理过程消耗的 token 数，协议提供时才有值 |
| `cache_read_tokens` | int? | 缓存命中的输入 token 数，协议提供时才有值 |
| `cache_write_tokens` | int? | 写入缓存的 token 数，协议提供时才有值 |

注意 UnifiedUsage 只承载单次调用的原始计数。用户可见的派生指标——缓存命中率百分比、跨轮次累计等——由 [RunningStats](#runningstats--cacheanomalyinfo) 累计计算；预估费用等需要模型定价信息的指标不属于本结构的职责（定价知识在 LLM 模块）。

### RunningStats / CacheAnomalyInfo

跨轮次 LLM 用量的统计结构族。Session 持有 RunningStats，每次 API 调用完成后将当次 UnifiedUsage 累加进去；RunningStats 为会话运行时状态，不随 checkpoint 持久化，会话恢复后重新累计。派生的缓存命中率供 `/status` 展示，统计快照参与 compaction 阈值判断，保留状态供缓存命中一致性校验。行为细节（流式 MessageEnd 时更新、会话结束清零、缓存命中一致性校验）定义于 [llm-session-enhancements](../session/llm-session-enhancements.md)，压缩对统计的读取时机见 [compact-process](../session/compact-process.md)，Session 概览见 [session](../session/README.md)；对 slash 的呈现见 [slash status](../slash/status.md)。

**RunningStats** 字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `total_prompt_tokens` | int | 所有调用累计的输入 token 总数 |
| `total_completion_tokens` | int | 累计输出 token |
| `total_tokens` | int | 累计总 token |
| `total_cache_read_tokens` | int | 累计缓存命中 token |
| `total_cache_write_tokens` | int | 累计缓存写入 token |
| `request_count` | int | 已累加的 API 调用次数 |
| `total_reasoning_tokens` | int | 累计推理 token |
| `last_prompt_tokens` | int? | 最近一次调用的输入 token 总数（作为下一轮校验的参考值），尚无调用时为空 |
| `last_completion_tokens` | int? | 最近一次调用的输出 token 数，供上下文占用估算 |
| `last_call_at` | timestamp? | 最近一次调用的毫秒级时间戳，供缓存自然过期判定 |

除原始累计外，RunningStats 还提供累计缓存命中率等派生指标的查询。累计口径：单次用量缺省 total_tokens 时按 prompt + completion 求和后累加；可空的缓存/推理 token 缺省按 0 计入累计。RunningStats 保留最近一次调用的输入 token 总数，作为下一轮缓存命中一致性校验的参考值；校验仅在「追加扩展轮」参与，其前置条件、容忍余量固定值与异常上报行为（需求见 [llm §F9](../../requirements/llm.md)）定义于 [llm-session-enhancements](../session/llm-session-enhancements.md)。

**CacheAnomalyInfo** 字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `previous_prompt_tokens` | int | 上一轮调用的输入 token 总数（参考值） |
| `current_cache_read_tokens` | int | 本次调用的缓存命中 token 数 |
| `deviation_tokens` | int | 本次命中数与参考值的偏差（绝对值） |
| `tolerance_tokens` | int | 本次判定采用的容忍余量 |

CacheAnomalyInfo 可格式化为用户可读的缓存命中异常提示文本（含参考值、本次命中数与偏差、容忍余量与常见原因说明）。

### CallTarget

CallTarget 是一次 LLM 调用实际由哪个供应方/模型服务的标识，由生产调用方（gateway 与 llm 回退链）在成功调用后上报，session 与 gateway 消费，用于记录本次调用实际服务的供应方与模型（如用量归属与调试展示）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `provider_id` | string | 实际服务该次调用的供应商标识 |
| `model_id` | string | 实际服务该次调用的模型标识 |

未主动上报的调用（如测试替身）留空，消费方退化为按会话配置语义处理。缓存命中一致性校验的判定状态（前缀快照、上次调用态、资格判定）为 session 模块内部状态，不在本清单。

### ProcessedMessage

ProcessedMessage 是 Processor Chain 的输出结构，Gateway 的消费入口。入站和出站方向共用同一结构，content_blocks 在不同方向携带不同复杂度的内容，metadata 携带方向相关的计算结果。

| 字段 | 类型 | 说明 |
|------|------|------|
| `content_blocks` | ContentBlock[] | 处理后的内容块数组。入站方向为单个 ContentBlock::Text（ContentNormalizer 标准化后的文本；非 text 消息跳过标准化，此格为原样内容的 Text 包装，后续由 Gateway 按媒体可得性分型路由，见下方数据流），出站方向为经 Verbosity 过滤后的 ContentBlock[] |
| `metadata` | map(string→string) | 方向相关的键值对，值均为字符串（复杂结构以 JSON 序列化）。入站的键由链调度在构建 ProcessedMessage 时从 NormalizedMessage 复制，并由 SessionRouter 与 Gateway 补充；出站的键由出站链与 Gateway 补充。完整键集见下方元数据键表 |

**元数据键**（标「条件」的键仅在满足条件时写入）：

| 键 | 方向 | 写入方 | 取值来源 |
|----|------|--------|----------|
| `message_type` | 入站 | 链调度（构建时从 NormalizedMessage 复制） | 原始 message_type，供 Gateway 做分型路由判断 |
| `unavailable_media` | 入站 | 链调度（同上复制） | 不可得媒体资源标识列表（JSON），供 Gateway 做媒体可得性判断 |
| `media_refs` | 入站（条件） | Gateway | NormalizedMessage.media_refs 序列化（媒体能力暂缓，当前不写入） |
| `thread_id` | 入站（条件） | Gateway | NormalizedMessage.thread_id |
| `reply_ref` | 入站（条件） | Gateway | NormalizedMessage.reply_ref |
| `account_id` | 入站（条件） | Gateway | NormalizedMessage.account_id |
| `chat_name` | 入站（条件） | Gateway | NormalizedMessage.chat_name |
| `trace_id` | 入站（条件） | Gateway | NormalizedMessage.trace_id（webhook 到达时生成） |
| `session_key` | 入站（条件） | SessionRouter | 计算所得的消息级标识，用于日志追踪 |
| `platform` / `sender_id` / `peer_id` | 入站 | SessionRouter | NormalizedMessage 对应字段（稳定路由键组成部分） |
| `span_id` | 入站（条件） | Gateway（链后） | 追踪 span 标识，供子 span 派生 |
| `agent_id` | 入站（条件） | Gateway（机器人→Agent 绑定）/ daemon chat RPC（CLI） | 本会话的目标 Agent |
| `channel` | 出站 | 出站链 / Gateway | 出站目标渠道 |
| `session_id` | 出站 | 出站链 / Gateway | 会话标识（出站链本地路由用途） |
| `verbosity_level` | 出站 | Gateway | 当前展示等级 |

**入站键的写入时序**：①链调度构建 ProcessedMessage 时从 NormalizedMessage 复制 `message_type`、`unavailable_media`；②SessionRouter 写入 `session_key` 与路由字段 `platform` / `sender_id` / `peer_id`；③Gateway 在链后补充其余入站键（`media_refs`、`thread_id`、`reply_ref`、`account_id`、`chat_name`、`trace_id`、`span_id`、`agent_id`，其中 `account_id` 与路由字段共同构成稳定路由键）。本表是入站键集的唯一权威定义，其它文档只描述写入时机与用途、不另行枚举。

元数据的复制与写入均不构成对消息的修改——各 Processor 不修改 NormalizedMessage 字段。`metadata` 不包含 `message_id`（无生产写入方）；NormalizedMessage 上的 `message_id` 是 Adapter 解析出的平台消息标识字段，二者不是同一键。

入站和出站不区分类型——同一个 ProcessedMessage 结构，内容形态和 metadata 字段按方向不同而不同。

### SlashResult

SlashResult 是斜杠指令 Handler 返回的执行结果类型。每个变体封装一种指令的副作用逻辑。Handler 返回 SlashResult 后，由 Gateway 构造 SideEffectContext 并触发 SlashResult 执行，各变体自行完成对应的 session 操作和消息回复。

SlashResult 共 10 种变体：

| 变体 | 载荷 | 用途 | 产出 |
|------|------|------|------|
| SetReasoning | `level`（[ReasoningLevel](#reasoninglevel--agentrole)） | 设置推理深度 | ContentBlock::Text（确认信息） |
| SetVerbosity | `level`（[VerbosityLevel](#verbositylevel)） | 设置信息展示等级 | ContentBlock::Text（确认信息） |
| Reply | 回复文本（string） | 纯文本回复，用于 /help、/status 等仅需回复文本的指令 | ContentBlock::Text（回复文本） |
| NewSession | 无 | 创建新会话（强制新建，即使当前标识下已有未归档 Session，见 [slash §F3](../../requirements/slash.md)） | ContentBlock::Text（确认信息） |
| Stop | 无 | 终止当前运行（含级联终止子 session） | ContentBlock::Text（确认信息） |
| WorkflowStart | `name`（string） | 启动指定 workflow（仅 Owner） | ContentBlock::Text（确认信息） |
| Compact | `instruction`（可选） | 触发对话历史压缩 | ContentBlock::Text（压缩结果） |
| SystemAppend | `action`（[SystemAppendAction](#slash-执行辅助类型)） | 向 system prompt 追加内容 | ContentBlock::Text（确认信息） |
| Exec | `command` | 执行系统命令（需权限评估，见下方执行模型） | ContentBlock[]（命令输出经出站 Processor Chain） |
| Unknown | 未匹配的指令名（string） | 未知指令回退 | ContentBlock::Text（提示信息） |

**执行模型**：Handler 返回 SlashResult 后，Gateway 统一调用执行方法，由各变体自行完成副作用。需权限评估的指令由 Handler 在 [SlashHandler 权限标记](core-traits.md#slashhandler)中声明，Gateway 在触发执行前统一经 [PermissionGate](core-traits.md#permissiongate) 判定（不穷举指令清单，变体不自带标记）；判定为 Deny 则返回权限错误，不进入副作用。新增指令只需新增 SlashResult 变体及其执行实现，Gateway 无需改动。

**边界**：SlashResult 仅由 SlashDispatcher 分派的斜杠指令 Handler 产出。审批指令（`/approve-once`、`/approve-whitelist`、`/deny`）由 Gateway 层硬拦截、走权限审批流验证，不进 SlashDispatcher，其审批结果不属于 SlashResult（详见 [permission 审批工作流](../permission/approval-workflow.md)）。权限管理指令（`/perm` 系列）同样不产出 SlashResult，由 Gateway 权限指令处理层硬拦截执行——具体子命令与参数以 [permission 权限配置管理](../permission/permission-configuration.md) 为权威；新用户注册的载荷结构见 [UserRegistration / UserCreationRequest / InitialPermissionSet](#userregistration--usercreationrequest--initialpermissionset)。

**SideEffectContext**：Gateway 在收到 SlashResult 后构造的执行上下文。携带当前 Session 的操作能力（用于会话创建/停止、压缩等操作）和回复通道（用于产出回复内容）。SideEffectContext 由 Gateway 管理，SlashResult 不持有其引用。

| 字段 | 类型 | 说明 |
|------|------|------|
| `session_id` | string | 斜杠指令所在的会话 ID |
| `sender_id` | string? | 调用方发送者标识；供 Owner 作用域副作用判定（如 Owner 的 `/stop` 需终止该会话活跃 workflow） |
| `channel` | string | 渠道标识（如 `"feishu"`、`"terminal"`） |
| `session_lookup` | SessionLookup | 会话状态查询接口（见 [core-traits SessionLookup](core-traits.md#sessionlookup)） |
| `reply_tx` | 回复通道 | ReplyAction 通道，SlashResult 执行时回发回复内容 |
| `executor` | SlashEffectExecutor | 斜杠指令副作用执行接口（见 [core-traits SlashEffectExecutor](core-traits.md#slasheffectexecutor)） |

**与 ContentBlock[] 的关系**：SlashResult 各变体在执行中通过 SideEffectContext 的回复通道产出 ContentBlock[]，进入出站 Processor Chain——与 LLM 的 UnifiedResponse 走同一条出站处理路径（VerbosityFilter → OutboundRawLog → IM Adapter 渲染发送）。

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
| `request_id` | string | 审批队列中的唯一请求 ID，用于关联审批结果与回调定位 |
| `initial_permissions` | list(InitialPermissionSet) | 请求携带的预置权限集候选：入队时由 Gateway 按指令参数构造，Owner 审批时确认或调整；为空表示注册后不授予任何规则（与零权限默认一致） |

**InitialPermissionSet**——Owner 可授予新注册用户的预置权限集枚举。每个变体映射为一组具体权限规则（如 BasicMessaging 对应收发消息 + workspace 读）。当前仅有 `BasicMessaging` 一个变体；新增预设按同样方式扩展映射规则。预设集是 Owner 在审批时**显式选择**的选项而非注册默认——需求要求新建 User 默认无任何权限（含收发消息），无预置权限集时注册后的 User 不获得任何规则，与 [permission 需求 §F1](../../requirements/permission.md) 的零权限默认一致。

### FragmentContext

FragmentContext 是 PromptFragmentProvider 片段生成时的输入上下文，由 System Prompt Builder 构建后传递给各 Provider。

| 字段 | 类型 | 说明 |
|------|------|------|
| `agent_id` | string | Agent 标识。Skills / Tools / Memory 等 Provider 按此决定可见范围 |
| `session_role` | enum | Session 角色：SessionRole::Main（主 Agent Session）或 SessionRole::Sub（子 Session）。长期记忆（MEMORY.md）按此角色门控（仅主 Session 加载，与身份加载模式无关）；自定义引导指令（BOOTSTRAP.md）另行按 `bootstrap_mode` 门控（仅主 Session 且完整模式加载，见 [system_prompt §F1/F8](../../requirements/system_prompt.md)） |
| `bootstrap_mode` | enum | 身份加载模式，取值来自 agent 配置的 `bootstrapMode`：BootstrapMode::Minimal（精简）或 BootstrapMode::Full（完整），仅主 Agent Session 下决定是否注入自定义引导指令（BOOTSTRAP.md）。子 Session 不据此注入可选内容 |
| `no_bootstrap` | bool | Agent 是否显式声明无 Bootstrap 文件。为真时该 Session 按「无 Bootstrap 文件的 Session」处理——仅加载工具/技能清单，不加载任何 Bootstrap 文件与长期记忆（见 [system_prompt §F8](../../requirements/system_prompt.md)） |
| `bootstrap_dir_present` | bool | Bootstrap 文件目录（`bootstrap_dir`）是否存在，由 Builder 在构建时检查。与 `no_bootstrap` 共同决定「无 Bootstrap 文件的 Session」：`no_bootstrap` 为真或本字段为假时，Provider 不加载任何 Bootstrap 文件与长期记忆；未显式声明且目录缺失时同时记录降级告警（规则见 [system_prompt/static-layer](../system_prompt/static-layer.md)） |
| `bootstrap_dir` | string | bootstrap 文件所在目录，BootstrapFragmentProvider 按此查找 bootstrap 文件。值来源于 agent 配置的 agentDir 字段 |
| `activated_skills` | list(string) | 当前 session 已条件激活的技能名列表，由 Session 模块在 SP 重建时按值传入，SkillsFragmentProvider 据此把已激活条件技能纳入清单（仅 SP 重建路径有效） |
| `tool_registry` | ToolRegistryQuery? | 供需要工具信息的 Provider（如 ToolsFragmentProvider）直接查询的工具注册表引用；来自 InjectionParams，缺省回退 Provider 默认 |

Session 角色（主/子）在主 Session 创建 / spawn 子 Session 时确定，由 SessionManager 在触发构建时同 agent_id 一并传给 Builder 写入 FragmentContext。身份加载模式（bootstrap_mode）是 agent 配置的静态属性；两者正交——主 Agent Session 精简模式下 session_role 仍为 Main。Builder 另检查 Bootstrap 文件目录是否存在（`bootstrap_dir_present`），与 `no_bootstrap` 一起解析出三种 Session 类型（主 Agent Session / 子 Session / 无 Bootstrap 文件的 Session），判定规则见 [system_prompt/static-layer](../system_prompt/static-layer.md) §Session 类型与 Bootstrap 文件加载。

### PromptFragment

PromptFragment 是单个 PromptFragmentProvider 产出的静态层片段。

| 字段 | 类型 | 说明 |
|------|------|------|
| `section_title` | string | Section 标题，如 `## AGENTS.md`、`## Available Skills` |
| `section_type` | [SectionType](#会话注入辅助类型) | Section 类型 |
| `content` | string | 渲染完成的文本内容 |

### RenderedOutput

RenderedOutput 是 IMPlugin 渲染方法产出的平台原生格式消息结构。渲染产出数据，发送执行副作用——Gateway 在两步之间插入中间件（审计、频率限制等）。流式场景下渲染以增量方式进行：StreamingRenderer 每处理完一批事件产出一个 StreamingOutput（见 [core-traits StreamingRenderer](core-traits.md#streamingrenderer)），平台按本批内容渲染或增量更新平台消息（不预先组装为最终整条消息），不再单独定义平台消息结构。

| 字段 | 类型 | 说明 |
|------|------|------|
| `msg_type` | string | 消息格式类型（如 `"text"`、`"interactive"`），由 Renderer 按内容特征选择 |
| `payload` | any | 平台原生格式的消息体，结构由各平台 Renderer 定义。Gateway 中间件和 Adapter 发送不解析 payload 内容 |

**输出格式决策**：由各平台 Renderer 按内容特征选择输出格式，规则详见 [IM Adapter §平台渲染选择](../im_adapter/README.md#平台渲染选择)。大致原则：纯文本、无格式标记 → `"text"`；含 markdown 格式/换行/Thinking/ToolUse/ToolResult 块 → `"interactive"`。终端渠道例外：terminal 渠道无富格式消息形态，RenderedOutput 恒为 `"text"`——富内容（Thinking/工具块）已在 payload 内转为 ANSI 样式文本（见 [cli/Terminal Renderer](../cli/renderer.md)）。

#### StreamingOutput

StreamingOutput 是流式渲染过程中单批事件的处理产出：本批投递的文本内容列表（完整文本行或强制输出时的行内片段），加本批内累积完整的非文本块。被 common 的 [StreamingRenderer](core-traits.md#streamingrenderer) 处理单批事件产出、各平台插件持有并委托调用；其流式发送逻辑将本批内容组装为 RenderedOutput 后经 IMPlugin 的发送能力投递（不经 IMPlugin 的渲染方法）。组装以**本批**为粒度：`msg_type` 取平台流式形态（如飞书卡片 `"interactive"`），`payload` 由平台流式发送逻辑按本批 `text_messages` 与 `render_blocks` 增量更新平台消息；每批各自组装并投递一份增量 RenderedOutput，而非等待整条消息完成——与 [RenderedOutput](#renderedoutput) 的「不预先组装为最终整条消息」一致。gateway 在流式出站管线中传递该结构——满足共享类型准入条件。

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

**过滤范围**：三个等级的过滤口径以需求 [processor_chain §F4](../../requirements/processor_chain.md) 为准：

| 等级 | 值 | 过滤行为 |
|------|---|---------|
| full | `"full"` | 展示全部内容块：文本、思维链、工具调用与结果、媒体块（Image / Audio / File） |
| normal | `"normal"` | 移除思维链块（Thinking）与工具调用块（ToolUse），保留文本、工具结果与媒体块 |
| off | `"off"` | 仅保留纯文本块（Text），移除 Thinking / ToolUse / ToolResult 与媒体块 |

**作用范围**：Verbosity 控制展示内容，不影响 LLM 推理深度和 Agent 行为模式。仅有 `/verbose` 指令通过 VerboseHandler 写入 Session 的 Verbosity 字段，无其他写入者。切换等级不影响当前正在输出的消息——仅对后续新消息生效。

### CompactionResult / CompactionError

compaction（对话历史压缩）操作的产出与错误。CompactionResult 描述一次压缩的结果；CompactionError 是压缩失败的错误。两者经 [SlashEffectExecutor](core-traits.md#slasheffectexecutor) 的压缩方法在 session（产出）与 Gateway（消费）之间传递，故收录 common。

**CompactionResult 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `performed` | bool | 是否实际执行了压缩。false 表示本次未达压缩条件、压缩后无变化 |
| `before_tokens` | int | 压缩前上下文的 token 估算值 |
| `after_tokens` | int | 压缩后 transcript（仅 boundary 消息）的 token 估算值 |
| `boundary_message_id` | string | 承载摘要的 boundary 消息标识 |
| `auto_triggered` | bool | 是否由自动阈值触发（false 为手动触发） |
| `summary` | string | 面向用户的结果描述（压缩前后用量与结果概要），Gateway 直接回发 |

**CompactionError 变体**：LlmCallFailed（摘要生成的 LLM 调用失败）、SessionNotFound（会话不存在）、SummaryUnusable（摘要解析失败或产出不可用）、NothingToCompact（无消息可压缩）、HandlerUnavailable（所需 handler 不可用）。

**唯一链路**：`/compact` → SlashResult::Compact → SlashEffectExecutor 压缩方法 → `Result<CompactionResult, CompactionError>` → [SlashResultExecutor](core-traits.md#slashresultexecutor) 以 ReplyAction::Reply 回发 summary 或错误提示。压缩是 SlashResult 的副作用而非回复动作，无第二条触发入口。

**边界**：本题只承载跨 trait 边界的压缩产出与错误。压缩的触发条件、保留区、熔断与摘要格式等**行为语义**归 session 模块，设计定义见 [session compact-process](../session/compact-process.md) 与 [session lifecycle](../session/session-lifecycle.md)；压缩配置的**跨模块共享数据结构** CompactConfig 定义于 common（见下）。

- **CompactConfig**：压缩相关配置的共享数据结构，被 session（执行压缩）、gateway、config（解析与校验）、cli、daemon 多模块消费；因 cli、config 等消费方按 [STANDARDS 依赖方向允许边表](../STANDARDS.md)不依赖 session，该数据结构随 common 定义（配置的行为语义仍归 session）：

| 配置项 | 类型 | 默认值 | 说明 |
|------|------|------|------|
| 告警阈值 | float | 0.25 | 剩余空间占上下文窗口百分比不高于此值时提示 User 即将压缩 |
| 压缩阈值 | float | 0.20 | 剩余空间占上下文窗口百分比不高于此值时触发自动压缩；须低于告警阈值 |
| 保留区比例 | float | 0.16 | 自动压缩保留原文的 token 数占上下文窗口百分比；手动/溢出兜底压缩按 0 处理 |
| 熔断失败次数 | int | 3 | 连续自动压缩失败达此次数后进入保护暂停 |
| 摘要模型 | string? | 未配置时取当前 Session 的模型 | 生成压缩摘要所用模型；配置为独立摘要模型时覆盖当前 Session 模型（按 Agent 独立配置，无全局默认） |

各配置项语义与默认值以 [session §F3](../../requirements/session.md)（长对话压缩）为准，压缩执行细节见 [session compact-process](../session/compact-process.md)。

### ReasoningLevel / AgentRole

会话行为相关的跨模块枚举。

- **ReasoningLevel**：推理深度档位——low / medium / high / max，默认 high；off 表示关闭推理输出（供应商不支持关闭时降至最低可用档位）。作为 [InternalRequest](#internalrequest--internalmessage--systemblock--tooldefinition) 字段随 LLM 请求传递，各协议映射为供应商原生参数；`/reasoning` 指令经 [SlashEffectExecutor](core-traits.md#slasheffectexecutor) 设置，由网关层解析生效档位——当所选模型不支持请求档位时自动降级到该模型支持的最高档位（需求见 [llm §F4](../../requirements/llm.md)）。
- **AgentRole**：Agent 身份枚举——MainAgent（主 Agent）/ SubAgent（分身 Agent），标识 Agent 层级（与 [FragmentContext](#fragmentcontext) 的 SessionRole（主/子 Session）相关但不同层：前者描述 Agent 身份，后者描述 Session 角色）。

### InternalRequest / InternalMessage / SystemBlock / ToolDefinition

系统内部协议无关的 LLM 请求结构族。[LlmCaller](core-traits.md#llmcaller) 的一次调用以 InternalRequest 表达，各 Provider 协议层将其映射为供应商原生请求格式（映射规则见 [llm protocol-mapping](../llm/protocol-mapping.md)）。它承载消息序列、采样参数、两段式 system prompt（静态可缓存段 + 动态段）与工具定义，是 LLM 调用契约的载荷。

**InternalRequest 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `model` | string | 本次请求使用的模型标识 |
| `messages` | list(InternalMessage) | 有序对话消息序列 |
| `temperature` | float | 采样温度。调用方未显式指定时取该模型配置的推荐默认温度（内置知识库填充，见 [llm §F7](../../requirements/llm.md)）；模型配置未提供时为 0 |
| `max_tokens` | int? | 生成上限，可选 |
| `stream` | bool | 是否流式，默认否 |
| `system_static` / `system_dynamic` | string? | system prompt 静态（可缓存）段与动态段，两段契约见 [system_prompt kv-cache](../system_prompt/kv-cache.md) |
| `system_blocks` | list(SystemBlock)? | 缓存适配器产出的结构化 system 块，可选 |
| `tools` | list(ToolDefinition)? | 随请求传入的工具 schema 定义，可选 |
| `session_id` | string? | 会话标识，可选。与缓存命中校验无关——供应商前缀缓存依据内容前缀（见 [llm cache-adapter](../llm/cache-adapter.md)），本字段不参与命中判定 |
| `extra_body` | map(string→any) | 供应商特定附加参数，可选 |
| `reasoning_level` | [ReasoningLevel](#reasoninglevel--agentrole) | 推理深度档位，各协议映射为原生参数（默认 high） |
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

### RiskLevel / PermissionRequest / PermissionOperation / PermissionDecision / PermissionDenied

权限判定与审批的载荷族，随 [PermissionGate](core-traits.md#permissiongate)、[AuditLogger](core-traits.md#auditlogger) 在 common 与消费/实现方之间传递。

- **RiskLevel**：权限请求的风险级别枚举——Low / Medium / High / Critical。仅用于审批提示与审计，不决定是否进入审批（高风险名单另成一套判定）。

**PermissionRequest 字段**（一次权限判定的输入）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `agent_id` | string | 发起操作的 Agent |
| `user_id` | string | 本轮 caller 的 User ID——触发本轮执行的入站消息发送者映射（与 [NormalizedMessage.account_id](#normalizedmessage) 同物：两者都是该发送者在 CloseClaw 中的 User 标识，仅作用域不同）；不取会话 Owner 或 checkpoint 中持久化的 sender |
| `is_sub_session` | bool | 发起方是否为子 Session。为真时拒绝静默返回、不入审批队列（需求 [permission §F9](../../requirements/permission.md)） |
| `session_id` | string? | 操作归属的会话标识，供审批请求归属 Session（审批 ID 按 Session 独立计数）；无会话上下文时为空 |
| `operation` | PermissionOperation | 待评估的操作 |
| `purpose` | string? | 命令类操作由 Agent 提供的用途说明，写入审批卡片 |

**PermissionOperation 变体**（每个变体自身携带评估所需的操作参数，覆盖 permission 权限维度）：FileRead（`path`）/ FileWrite（`path`）/ Command（`command`、`sandbox_escape`（是否请求移出沙盒在宿主执行））/ Network（`host`、`port`）/ ToolCall（`tool_name`）/ UserMessage（`direction`：入站/出站）/ AgentMessage（`from_agent`、`to_agent`）/ Spawn（`child_agent_id`、`parent_session_id`）/ ConfigWrite（`path`）。八类权限维度中「跨 Agent 通信」由 AgentMessage 与 Spawn 两个变体表达，其余维度与变体一一对应；判定逻辑归 permission 模块，本结构只承载输入。

- **PermissionDecision**：一次判定的结论——Allowed，或 Denied（`reason` 原因 + `risk_level` 风险级别）。需 Owner 审批的 Deny 由实现方在调用内等待，等待语义见 [core-traits PermissionGate](core-traits.md#permissiongate)。
- **PermissionDenied**：权限校验被拒的错误形态——携带 `reason` 与 `risk_level`，与 PermissionDecision::Denied 字段口径一致。`reason` 仅描述操作被拒，不含规则名称、优先级或配置路径等内部信息（需求 [permission §F8](../../requirements/permission.md)）。

**拒绝错误字段口径**：权限域的拒绝统一携带 `reason` + `risk_level`（PermissionDecision::Denied 与 PermissionDenied 同构）；[CommunicationError](#communicationconfig--communicationcheckresult--communicationerror) 的 Denied 是 Agent 通信白名单拒绝、不经权限判定，仅携带 `reason`。

**边界**：本族仅承载权限判定的输入输出语义；规则求值、审批队列、审计落盘均归 permission 模块（见 [permission 模块](../permission/README.md)）。

### HookConfig / HookParams / HookType

Session hook 审查的配置族（按 Agent 配置）。HookConfig 是单个 hook 的配置——`hook_type`（类型）、`enabled`（是否启用）、`params`（可调参数）；HookType 是 hook 类型枚举——PlanCheck（只计划未执行）、LoopCheck（重复工具调用循环）；HookParams 是 hook 可调阈值——`loop_check_repetition_threshold`（判定循环的连续相似调用数）。由 agent 配置定义、session 的 Hook 审查器（HookReviewer）消费。hook 审查行为见 [session run-health](../session/run-health.md)。

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

**SpawnError**——子会话生成校验的错误（SpawnValidator 两步的统一错误载体）：DepthExceeded（超出生成深度上限）、MaxChildrenReached（达到最大并发子会话数）、AgentNotAllowed（目标 agent 不在 allowlist）、AgentIdRequired（配置要求 agentId 但未提供）、ConfigNotFound（目标 agent 配置缺失）、Permission（权限被拒，载荷复用 [PermissionDenied](#risklevel--permissionrequest--permissionoperation--permissiondecision--permissiondenied)，不重复定义拒绝载荷）。

### AuditLogEntry / AuditDisposition / AuditLogFilter

[AuditLogger](core-traits.md#auditlogger)（common DI trait）契约的载荷族，随权限审计的写入与查询在 permission（实现方 + 消费方）与 daemon、tools（消费方）之间传递。

**AuditLogEntry**——单条审计日志：

| 字段 | 类型 | 说明 |
|------|------|------|
| `timestamp` | string | 事件时间（ISO 8601） |
| `agent_id` | string | 涉及操作的 agent |
| `user_id` | string | 本轮 caller 的 User ID——与 [PermissionRequest](#risklevel--permissionrequest--permissionoperation--permissiondecision--permissiondenied) 同口径，供按触发者定位审批/拒绝审计 |
| `tool_name` | string | 请求类型名（操作维度，如 file / exec / network；非 ToolRegistry 工具名） |
| `operation` | string | 操作描述（如 `write <path>`、命令文本） |
| `reason` | string | 处置的人类可读原因 |
| `risk_level` | [RiskLevel](#risklevel--permissionrequest--permissionoperation--permissiondecision--permissiondenied) | 操作风险级别 |
| `disposition` | AuditDisposition | 最终处置（单次放行/加入白名单放行/拒绝） |
| `session_id` | string? | 审批所属会话标识（审批 ID 按 Session 内递增，需求 [permission §F5](../../requirements/permission.md)）；无会话上下文时为空 |

**AuditDisposition**——审计处置枚举：Approved（单次放行）、Whitelisted（加入白名单放行）、Rejected（拒绝）。对应需求 [permission §F5](../../requirements/permission.md) 的三种 Owner 决策。

**AuditLogFilter**——审计条目查询过滤条件（各字段均可选，为空表示不过滤）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `agent_id` | string? | 仅返回该 agent 的条目 |
| `user_id` | string? | 仅返回该 User 触发（本轮 caller）的条目 |
| `disposition` | AuditDisposition? | 仅返回该处置的条目 |
| `session_id` | string? | 仅返回该会话的条目 |
| `since` | string? | 仅返回 timestamp ≥ 此值（ISO 8601）的条目 |
| `until` | string? | 仅返回 timestamp ≤ 此值（ISO 8601）的条目 |

由审计查看工具（tools）经 [AuditLogger](core-traits.md#auditlogger) 的查询能力消费。

### AgentConfigInfo

[AgentConfigLookup](core-traits.md#agentconfiglookup)（common DI trait）的返回类型——按 agent_id 查得的该 agent 配置子集（子 Agent 生成与子会话超时告警相关的最小集合，非完整 agent 配置档案）。

| 字段 | 类型 | 说明 |
|------|------|------|
| `subagents_model` | [ModelSpec](#modelspec)? | 该 agent 配置的子 Agent 模型覆盖，未配置为空 |
| `timeout_warning` | int? | 子 Agent 执行时长告警阈值（秒），空表示回退全局默认 |
| `timeout` | int? | 子 Agent 硬超时（秒），空表示回退全局默认（48 小时），参与 spawn 生效值解析 |
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
| `initial` | bool | 是否为初始 Agent（系统基础入口）。配置向导创建时置真（初始 Agent 的 ID 默认 `master`，见 [agent §F5](../../requirements/agent.md)），全程至多一个；供 CLI 解析缺省目标 Agent（解析规则见 [cli/chat.md](../cli/chat.md)） |
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
| `compact` | [CompactConfig](#compactionresult--compactionerror)? | 压缩配置 per-agent 覆盖，未声明回退全局默认（需求 [session §F3](../../requirements/session.md)） |

ResolvedAgentConfig 是**静态配置视图**——所有字段已完成解析（默认值补齐），但不含运行时派生能力（权限过滤后的工具清单等由下游消费模块按需派生，见 [agent-registry §架构](../agent/agent-registry.md)）。`tools` / `skills` 保留静态字面值（`["*"]` 即字面 `["*"]`，不展开为具体工具名）。

**SubagentsConfig 字段**（子 Session 创建控制，由 agent 配置定义、session 的 spawn 流程消费）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `allow_agents` | list(string) | 允许 spawn 的目标 Agent ID 白名单；`["*"]` 表示不限制，空数组表示禁止 spawn 任何子 Session（见 [agent §F9](../../requirements/agent.md)） |
| `require_agent_id` | bool? | spawn 时是否必须显式指定 agentId，默认 `false` |
| `max_spawn_depth` | int? | 本 Agent 允许的最大子孙层级数（0 表示禁止 spawn）。实际生效值取父 Agent 与本 Agent（目标）配置中更严格者（需求 [agent §F9](../../requirements/agent.md)） |
| `max_children` | int? | 最大并发活跃子 Session 数 |
| `timeout` | int? | 子 Session 硬超时（秒）；spawn 显式参数 > 目标 Agent 配置 > 全局默认（48 小时）。须不小于 `timeout_warning` |
| `timeout_warning` | int? | 子 Session 超时预警时长（秒）；解析顺序同 `timeout`（spawn 显式参数 > 目标 Agent 配置 > 全局默认） |
| `timeout_notify_interval_ratio` | float? | 预警后循环通知间隔比例（相对 timeout_warning），取值 [0.1, 2.0]，默认 0.5 |
| `model` | [ModelSpec](#modelspec)? | 子 Session 默认模型覆盖 |

**MemoryConfig**：记忆子系统的 per-agent 覆盖结构（字段级覆盖全局 `memory.json`）。作为 ResolvedAgentConfig 的 `memory` 字段类型在此收录；字段结构与默认值以 [memory/config](../memory/config.md) 为权威定义，本文档不重复。**字段级合并契约（本共享结构的跨模块语义）**：其全部叶子字段为可选（None=继承全局），合并方向为 per-agent 覆盖全局（agent 值优先，未声明处回退全局 `memory.json`）；ResolvedAgentConfig.memory 为 null 表示不声明 memory 段、整体回退全局。三种声明形态的最终状态：不声明 → 全部继承全局；声明空对象 → 字段均未声明、等价全部继承全局；声明部分字段 → 仅覆盖声明项。

MemoryConfig 的嵌套子配置结构（均为其字段类型，字段结构见上述权威定义）：

#### MemoryStorageConfig

`storage` 字段类型。

#### MiningConfig

`mining` 字段类型。

#### TranscriptCleanRules

`mining.transcript_clean_rules` 字段类型。

#### DreamingConfig

`dreaming` 字段类型。

#### DreamingScoringConfig

`dreaming.scoring` 字段类型。

#### DreamingThresholdConfig

`dreaming.threshold` 字段类型。

#### DreamingCapacityConfig

`dreaming.capacity` 字段类型。

#### DreamingDiaryConfig

`dreaming.diary` 字段类型。

#### SearchConfig

`search` 字段类型。

#### ForgettingConfig

`forgetting` 字段类型。

### 消息/内容辅助类型

**ContentBlockType**——ContentBlock 的块类型分类枚举，用作 [StreamEvent](#streamevent) 的 `block_type` 与流式渲染的分类：Text / Thinking / ToolUse / ToolResult / Image / Audio / File。

**MessageType**——入站消息类型枚举，为 [NormalizedMessage](#normalizedmessage) 的 `message_type` 字段：

| 变体 | 说明 |
|------|------|
| Text | 纯文本消息（默认变体） |
| Image | 图片消息 |
| File | 文件消息 |
| Audio | 音频消息 |
| Post | 含内嵌媒体的富文本 post——展开文本进 `content`，内嵌媒体引用进 `media_refs` |

**MediaType**——媒体类型枚举，为 [MediaRef](#normalizedmessage) 的 `media_type` 字段：Image（图片）/ File（文件）/ Audio（音频）。

**ProcessError**——Processor Chain 入站/出站处理的错误：

| 变体 | 载荷 | 说明 |
|------|------|------|
| ProcessorFailed | `name`、`source` | 链中某个 processor 失败 |
| InvalidMessage | `string` | 进出站消息格式非法 |
| ChainFailed | `string` | 链自身失败（如空链配置错误） |

**AdapterError**——IM Adapter 操作的错误，为 [IMPlugin](core-traits.md#implugin) 契约的错误载荷：

| 变体 | 载荷 | 说明 |
|------|------|------|
| InvalidPayload | `string` | 平台事件 payload 非法 |
| AuthFailed | — | 平台鉴权失败 |
| SendFailed | `string` | 发送失败 |
| InvalidSignature | — | 签名校验失败 |
| IoError | `io::Error` | IO 错误 |
| UnsupportedOperation | — | 平台不支持的操作 |

### 会话/注入辅助类型

**SessionRole**——会话角色枚举，为 [FragmentContext](#fragmentcontext) 字段：Main（顶层 Agent 会话，加载 MEMORY.md 与 BOOTSTRAP.md 等受身份门控内容）/ Sub（被 spawn 的子会话，无论身份加载模式为何都抑制受门控内容）。「无 Bootstrap 文件的 Session」不是 SessionRole 的枚举值，而是由 `no_bootstrap` / `bootstrap_dir_present` 叠加在角色之上的 Session 类型（见 [FragmentContext](#fragmentcontext)）。

**SectionType**——[PromptFragment](#promptfragment) 的片段类型枚举：Bootstrap（bootstrap 文件）/ Tools（工具清单）/ Skills（技能清单）/ Memory（长期记忆）。

**BootstrapMode**——身份加载模式枚举，取自 agent 配置的 `bootstrapMode`，为 [FragmentContext](#fragmentcontext) 字段：Minimal（精简，仅运行时必需的身份/工具文件）/ Full（完整文件集，含需持久上下文/记忆的文件）。

**RequestContext**——当前入站消息的元数据，由 Gateway 在每次 LLM 调用前写入 session，供动态层构建使用；置于 common 以避免 session→gateway 反向依赖：

| 字段 | 类型 | 说明 |
|------|------|------|
| `sender_id` | string | 消息发送者标识（Open ID） |
| `channel` | string | 渠道标识（如 `"feishu"`、`"telegram"`） |
| `timestamp` | int | 消息创建时的毫秒级 Unix 时间戳（与 NormalizedMessage.timestamp 同单位） |
| `chat_name` | string | 会话/群名称，无则为空串 |

**InjectionParams**——system prompt 注入链的参数契约，由 SessionManager → session → System Prompt Builder 传递；设计契约要求 session_id、agent_id 与 Session 角色必填，身份加载模式与 ToolRegistry 引用可缺省（缺省时由 Builder 按 agent 配置与 Provider 级默认回退）。Bootstrap 文件目录与无 Bootstrap 文件声明由 Builder 经 AgentLookup 查询，不经由本结构传递：

| 字段 | 类型 | 说明 |
|------|------|------|
| `session_id` | string | 会话标识，用于日志与追踪透传 |
| `agent_id` | string | Agent 标识，决定加载哪个 agent 的 bootstrap 文件及工具/技能可见性规则 |
| `bootstrap_mode_override` | BootstrapMode? | 身份加载模式覆写；空表示用 agent 配置默认值 |
| `activated_skills` | list(string) | 本会话条件激活的技能名，供 SkillsFragmentProvider 纳入清单 |
| `session_role` | SessionRole | 会话角色，决定身份门控行为 |
| `tool_registry` | ToolRegistryQuery? | 可选 ToolRegistry 引用；空时回落到 Provider 级默认 |

**TurnCounter**——会话轮次计数器（一「轮」= 一次 User 消息进入至 Agent 完成响应，与 [session §F3](../../requirements/session.md) 的压缩检测轮次口径一致）；计数器内部私有，仅暴露新建 / 自增 / 读取。

**PendingMessage**——待处理消息（未最终确认、等待注入对话的消息），为 [SessionLookup](core-traits.md#sessionlookup) / [SlashSessionQuery](core-traits.md#slashsessionquery)「向统一消息队列推送」的载荷：

| 字段 | 类型 | 说明 |
|------|------|------|
| `message_id` | string | 消息标识 |
| `content` | string | 消息内容 |
| `created_at` | int | 创建时间（毫秒级 Unix 时间戳，与 NormalizedMessage.timestamp 同单位） |
| `sent` | bool | 是否已发送 |
| `priority` | [NotificationPriority](#backgroundtask--taskstate--runningtaskinfo--completionnotification--notificationpriority--backgroundtaskerror)? | 注入优先级，非 User 消息按此排序注入；User 消息与未携带优先级的非 User 消息按后述规则排序（需求 [session §F10](../../requirements/session.md)） |
| `role` | string? | 消息角色（user / assistant / workflow），用于 transcript 格式化 |
| `target_channel` | string | 目标渠道标识 |
| `platform` | string? | 出站消息的平台标识 |
| `content_blocks` | string? | 出站消息内容块（序列化 JSON，ContentBlock[]） |

`content_blocks` 存在时优先于 `content`（与 InternalMessage 同口径）；两者均为序列化载荷，反序列化失败时按该项缺失处理。本结构与 SessionCheckpoint 的 `pending_messages` 字段不同物——后者是 transcript 快照，本结构是统一消息队列的入队载荷。

`role` 为 `workflow` 时表示 workflow Engine 注入的控制消息（步骤目标、验收清单、跳转问题、恢复提示），经统一消息队列进入对话历史供 LLM 消费，不作为用户可见消息出站，也不写用户可见出站记录（消息生命周期见 [workflow/session-integration](../workflow/session-integration.md)）。

### Slash 执行辅助类型

**SlashContext**——斜杠指令执行上下文，为 [SlashHandler](core-traits.md#slashhandler) 的入参：

| 字段 | 类型 | 说明 |
|------|------|------|
| `command` | string | 指令名（不含前导 `/`）；多命令 handler（如 workdir 系列）据此分支 |
| `sender_id` | string | 消息发送者标识 |
| `is_owner` | bool | 调用方是否为 Owner（按 User ID 判定）。Owner 专属指令（如 `/workflow`、审批指令、`/system` 管理）据此拒绍非 Owner 调用（需求 [slash §F6](../../requirements/slash.md)、§F13、§F15） |
| `session_id` | string | 指令被调用的会话 ID |
| `system_appends` | list(string) | 会话当前的 Owner 动态指令子集，供 `/system list` 读取；由 Gateway 从 Session 取得后写入——仅当调用方为 Owner 时填入，非 Owner 调用留空（需求 slash §F4） |
| `channel` | string | 渠道标识 |

**SystemAppendAction**——`/system` 追加动作，为 [SlashResult](#slashresult) 变体载荷：Add（携带要追加的指令文本）/ Clear（清空全部已追加指令）。

**ReplyAction**——[SlashResultExecutor](core-traits.md#slashresultexecutor) 产出、Gateway 分派的回复动作：

| 变体 | 载荷 | 说明 |
|------|------|------|
| Reply | ContentBlock[] | 内容块回复（经出站 Processor Chain：Verbosity 过滤 → 出站日志 → IM Adapter 渲染） |
| Nothing | — | 无需动作 |

压缩不在本枚举内：手动压缩由 SlashResult::Compact 经 SlashEffectExecutor 触发，结果以 Reply 回发（见 [CompactionResult](#compactionresult--compactionerror)）。

### LLM/流式/中间件辅助类型

**LLMError**——LLM 操作的错误，为 [LlmCaller](core-traits.md#llmcaller) 的错误载荷（[StreamEvent](#streamevent) 的 Error 事件载荷为错误消息文本）：

| 变体 | 载荷 | 说明 |
|------|------|------|
| AuthFailed | `string` | 鉴权失败 |
| RateLimitExceeded | — | 触发限流 |
| ModelNotFound | `string` | 模型不存在 |
| InvalidRequest | `string` | 请求非法 |
| ApiError | `string` | 供应商 API 错误 |
| NetworkError | `string` | 网络错误 |
| Cancelled | — | 请求被会话取消（级联停止或显式 /stop），永不重试 |
| PartialContent | `reason`、`thinking_blocks` | 流式中断错误，保留完整的 Thinking 块以写入会话历史 |

**ErrorKind**——LLM 错误的分类，用于重试策略判定：Transient（瞬时：429/5xx/超时，退避重试）/ Auth（认证错误，轮换凭据）/ Billing（计费/配额，长冷却）/ InvalidRequest（无效请求，不重试、切换模型）/ Unknown（未知，按瞬时处理但限制次数）。

**StreamDone**——流式完成通知载荷，为 [StreamingSink](core-traits.md#streamingsink) 完成通知的参数：

| 字段 | 类型 | 说明 |
|------|------|------|
| `model` | string | 产生最终响应的模型名 |
| `usage` | [UnifiedUsage](#unifiedresponse--unifiedusage)? | 供应商上报的 token 用量；部分供应商流式响应不带 usage 时为空 |

其 `usage` 与流式收尾 [StreamEvent](#streamevent) 的 MessageEnd 载荷 [UnifiedUsage](#unifiedresponse--unifiedusage) 同源（均为本次流式调用的最终用量）——StreamDone 是 StreamingSink 侧对同一次完成事件的载荷，MessageEnd 是 LLM 事件流侧的收尾事件，两者携带同一次流式完成的用量，不由各自独立统计。

**MiddlewareContext**——[OutboundMiddleware](core-traits.md#outboundmiddleware) 契约的上下文（链运行于 IMPlugin 渲染后、发送前，以不可变引用传入，中间件不得修改）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `session_id` | string | 该消息所属会话标识 |
| `channel` | string | 目标 IM 渠道 |
| `chat_id` | string | 目标平台 chat / group 标识 |

**MiddlewareError**——[OutboundMiddleware](core-traits.md#outboundmiddleware) 契约的错误：MiddlewareFailed（链中某中间件失败）/ Rejected（显式拒绝出站消息，短路整条链、消息不发送）。

### 工具契约载荷族

围绕 [Tool](core-traits.md#tool-trait)、[ToolRegistry](core-traits.md#toolregistry)、[ToolRegistrar](core-traits.md#toolregistrar)、[ToolRegistryQuery](core-traits.md#toolregistryquery)、[ToolSession](core-traits.md#toolsession)、[ToolExecutor](core-traits.md#toolexecutor) 的载荷类型：

**ToolFlags**——工具运行时标记：

| 字段 | 类型 | 说明 |
|------|------|------|
| `concurrency_class` | enum | 并发分类：ConcurrencySafe（并发安全）/ Serial（需串行）/ Expensive（资源消耗型：并发安全但消耗大，需限流）。未声明、字段缺失或取值非法一律按 Serial 处理（fail-closed，需求 [tools §F6](../../requirements/tools.md)） |
| `recommended_concurrency` | int? | 资源消耗型工具的推荐并发上限（如 3）；非资源消耗型可为空 |
| `is_read_only` | bool | 工具只读，不修改文件或状态 |
| `is_destructive` | bool | 工具可能覆盖或删除数据，需显式确认 |
| `is_deferred_by_default` | bool | 工具详情默认不载入 system prompt |

**ToolDescriptor**——工具摘要信息（system prompt 生成用）：`name`（工具名）、`group`（分组）、`summary`（清单用简短摘要）、`detail`（写入 system prompt 的详细描述）、`input_schema`（入参 JSON Schema）、`flags`（[ToolFlags](#工具契约载荷族)）。

**ToolBox**——桥接 `Tool` 与类型擦除注册的包装（`Arc<dyn Tool>`）。

**RegistryError**——工具注册表操作错误（区别于 ToolRegistrarError）：AlreadyRegistered（工具名已注册）/ Conflict（完整冲突详情：工具名、已注册方、尝试注册方）/ Frozen（注册表已冻结、不再接受注册）/ Internal（注册内部错误）。

**ToolRegistrarError**——Registrar 级注册错误：Conflict（工具名已被另一 Registrar 注册）/ Internal（Registrar 内部错误）。

**ToolContext**——工具调用时的运行时上下文：

| 字段 | 类型 | 说明 |
|------|------|------|
| `agent_id` | string | 调用该工具的 agent ID |
| `user_id` | string | 本轮 caller 的 User ID——触发本轮执行的入站消息发送者映射；不取会话 Owner 或 checkpoint 中持久化的 sender |
| `workdir` | string? | 当前工作目录绝对路径（若已设置）。分支、未提交改动等仓库状态不经本字段提供，由 Git 操作工具按需查询 |
| `session_id` | string? | 拥有本次工具调用的会话 ID（若已知） |
| `call_id` | string? | LLM 侧的工具调用标识 |
| `session` | ToolSession? | 所属会话的强引用（经 ToolSession 抽象） |
| `manual_background_signal` | 异步通知? | 来自 Session 的手动后台化信号，工具可等待该通知 |
| `media_store` | MediaStoreAccess? | 可选媒体存储，用于解析 `[type: key]` 引用 |

**ToolResult**——工具调用链路的结果结构（Tool::execute 返回类型）：`data`（结构化结果 JSON）、`new_messages`（待注入 agent 上下文的消息）。与 [ContentBlock](#contentblock) 的 ToolResult **变体**（出站内容块）同名但不同物。

**ToolMessage**——注入 agent 上下文的消息：`content`（消息内容）、`is_meta`（是否为元信息）。

**ToolCallError**——工具执行错误：NotFound（未找到）/ PermissionDenied（权限被拒）/ InvalidArgs（参数非法）/ ExecutionFailed（执行失败）/ NotImplemented（该工具未实现该调用）。

**PromptGenerationContext**——`generate_prompt` 的输入上下文：`agent_id`、`workdir`（string?，工作目录绝对路径）、`available_tool_names`（当前可用工具名，只传名字不传 descriptor）、`tools`（agent 级白名单）、`disallowed_tools`（黑名单，白名单过滤后应用）、`agent_role`、`agent_type`。

**ReadRange**——[ToolSession](core-traits.md#toolsession) 契约载荷：文件读取范围（`offset` 起始行、`limit` 最多行数）。

**FileReadCache**——文件读取去重缓存的载荷：记录 per-turn 已读文件的 `mtime` 与已读 `ranges`，供同一轮内重复读取判定文件是否变更（去重语义见 [tools read-tool](../tools/read-tool.md)）。为 [ToolSession](core-traits.md#toolsession) 契约的文件读取载荷：session（实现方）产出、tools（消费方）经契约读取。

**PendingToolCall**——多工具并行调度的调用描述（gateway 与 tools 共用）：`id`（调用标识）、`tool_name`、`args`（JSON 参数）、`file_path`（目标文件路径，可选）、`concurrency_class`（并发分类，取自 [ToolFlags](#工具契约载荷族)）。**DispatchGroup** 为分发策略枚举：Parallel / MutexByFile / Throttled（按 `recommended_concurrency` 限流）/ Serial。调度器与按文件互斥映射的实现（含锁实现）属领域逻辑，定义于 [tools multi-tool-calls](../tools/multi-tool-calls.md)，不入 common。

### 其他契约辅助类型

- **DynamicPromptContext**：[DynamicPromptBuilder](core-traits.md#dynamicpromptbuilder) 契约的动态层构建上下文：已存储的 System Prompt 静态层、当前请求元数据（RequestContext）、工作目录、会话创建时间、是否已压缩、是否面向子 agent。
- **AgentToolsConfig**：Agent 工具白/黑名单配置（`tools` 白名单、`disallowed_tools` 黑名单，均可选；`tools` 为 `None` / 空数组 / `["*"]` 表示全部工具），为 [AgentToolsConfigQuery](core-traits.md#agenttoolsconfigquery) 的返回类型。
- **ConditionalSkillMatch**：条件技能匹配结果（`name` 技能名、`listing_line` 含 ⚡ 注解的渲染清单行），为 [SkillListingProvider](core-traits.md#skilllistingprovider) 的载荷。

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

卡片交互事件不进入上述入站流动：Adapter 识别后忽略并记录调试日志，不回复、不注入对话（见建模边界）。

流式场景下的增量流动以 [StreamEvent](#streamevent) 事件流表达，流动路径即下文「ContentBlock[] 的出站流动路径」的流式分支（事件的产生与消费分工见模块关系节）；渲染过程中的单批产出为 [StreamingOutput](#streamingoutput)（完整文本行或强制输出的行内片段 + 本批完成的非文本块），生命周期止于本次流式发送完成。

LLM 非流式调用的响应流动：LlmCaller 返回 [UnifiedResponse](#unifiedresponse--unifiedusage) → Session 封装后其 ContentBlock[] 进入上述出站路径；用量 UnifiedUsage 由 [RunningStats](#runningstats--cacheanomalyinfo) 累加，供用量统计与缓存命中一致性校验。

ContentBlock[] 的出站流动路径：

```
LLM UnifiedResponse / SlashResult 变体
  ↓
ContentBlock[] 进入出站处理链路
  ↓
[Processor Chain 出站: VerbosityFilter → OutboundRawLog]
  ↓
ProcessedMessage { content_blocks, metadata }
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

图中出现的两处日志是不同层次的两份记录：链内的 OutboundRawLog 是 Processor Chain 出站的调试日志（按 Verbosity 过滤后的内容）；[Gateway 出站日志] 指发送成功后 Gateway 写入 session checkpoint 的出站历史记录（含 timestamp、session_id、platform、ContentBlock[]），规则详见 [gateway outbound-flow](../gateway/outbound-flow.md)。

ContentBlock[] 流式与非流式走同一条预处理管线——Verbosity 过滤同时适用于批量和流式。流式模式下增量内容以 [StreamEvent](#streamevent) 事件流形式在链上传递：VerbosityFilter 按块边界逐事件过滤；非过滤内容不引入额外缓冲或拷贝。两者的差异在渲染阶段：批量模式一次性渲染，流式模式增量渲染。

各共享类型流动路径的详细描述见下文各类型的数据流节。

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
Gateway — 先检查 message_type：非文本消息（含媒体）按当前范围处理——记调试日志并回复「暂不支持该类型消息」，不进入对话（需求 [gateway §F3](../../requirements/gateway.md)；媒体可得性校验与上下文形态属媒体能力启用后的目标形态，见 [im_adapter media-store](../im_adapter/media-store.md)）；对话消息从 content_blocks[0] 取 Text 内容做路由决策（/ 开头 → 斜杠指令；否则 → LLM 对话），从 metadata 取 session_key 传入 SessionManager（仅作日志追踪；SessionManager 以稳定路由键 platform+sender_id+peer_id+account_id 做查找）
```

出站方向：

```
ContentBlock[]（LLM 产出 / SlashResult 变体）→ Processor Chain 出站（VerbosityFilter → OutboundRawLog）
  ↓
ProcessedMessage {
  content_blocks: [过滤后的 ContentBlock[]],
  metadata: { verbosity_level }
}
  ↓
IM Adapter 渲染（消费 content_blocks）→ 发送；发送成功后 Gateway 写出站历史
```

ProcessedMessage 的生命周期：Processor Chain 产出 → Gateway 消费后即完成使命；结构本身不进入 Session 持久化（发送成功后 Gateway 另将出站历史写入 session checkpoint，那是另一份交付记录）。

### SlashResult

SlashResult 的执行流程：

1. Gateway 将 / 开头的消息路由到 SlashDispatcher
2. SlashDispatcher 解析指令名和参数，查找对应 Handler
3. Handler 处理完成后返回 SlashResult 变体
4. Gateway 构造 SideEffectContext
5. 需权限评估的指令：Gateway 经 [PermissionGate](core-traits.md#permissiongate) 判定（校验通过方继续执行，拒绝则返回权限错误）
6. 权限校验通过后，SlashResult 变体通过 SideEffectContext 触发执行，完成副作用，分两条路径：
   - 回复路径：产出 ContentBlock[] → 出站 Processor Chain → IM Adapter 渲染发送
   - 会话路径：执行 Session 操作（创建、停止、压缩等）

SlashResult 的生命周期：Handler 返回 → Gateway 构造 SideEffectContext 并触发执行 → 各变体通过 SideEffectContext 完成副作用后销毁。

### FragmentContext / PromptFragment

FragmentContext 和 PromptFragment 的流动嵌入在 system prompt 静态层的构建流程中：

```
SessionManager 触发构建（新 Session / 恢复 archive / compaction），传 Session 角色（主/子）与无 Bootstrap 文件声明
  ↓
System Prompt Builder 构建 FragmentContext（agent_id + session_role + bootstrap_mode + no_bootstrap + bootstrap_dir + bootstrap_dir_present）
  ↓
遍历已注册的 PromptFragmentProvider → 传入 FragmentContext → 各 Provider 产出 PromptFragment
  ↓
按优先级拼接所有 PromptFragment.content
  ↓
写入 ConversationSession 的 System Prompt 字段
```

FragmentContext 由 Builder 一次性构建，所有 Provider 共享同一上下文。PromptFragment 由各 Provider 独立产出，生命周期止于 Builder 完成拼接。

### RenderedOutput

RenderedOutput 的流动嵌入在 IM Adapter 出站渲染流程中：

```
ContentBlock[]（经 Processor Chain 出站处理后）
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
过滤后的 ContentBlock[] 继续后续出站链路（OutboundRawLog → Renderer）
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
usage (UnifiedUsage) → RunningStats 累加 → 用量统计、缓存命中一致性校验、调试日志与指标上报
```

流式调用不走 UnifiedResponse：增量以 StreamEvent 交付，收尾时 MessageEnd 携带 Optional UnifiedUsage 提供同一套用量口径（见 StreamEvent 节），同样汇入 RunningStats。

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
agent 配置层提供通信白名单原始配置（默认仅父 Agent），session spawn 流程据此生成通信路由表 CommunicationConfig
  ↓
session spawn 流程取子/父 Agent 的 CommunicationConfig 做通信权限校验
  ↓
CommunicationCheckResult（允许）/ CommunicationError（拒绝或缺失）
```

### RiskLevel / PermissionRequest / PermissionOperation / PermissionDecision / PermissionDenied

```
消费方（tools / session / gateway）就一次操作发起权限判定
  ↓
经 common 权限 trait（PermissionGate）
  ↓
权限引擎求值（经 PermissionGate）→ PermissionDecision（Allowed）或 Denied（reason + risk_level）
  ↓
需 Owner 审批的 Deny → 等待语义见 [core-traits PermissionGate](core-traits.md#permissiongate)
  ↓
调用方拿到最终结论；审批与拒绝结论经 AuditLogger 落盘
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
任务到达终态（Completed / Failed / Killed）→ 生成任务通知（带 NotificationPriority）；例外：被 Agent 主动终止的任务、以及 Session 停止引发的批量终止不生成终态通知（需求 [tools §F5](../../requirements/tools.md)）
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
SpawnValidator 权限校验（前置校验产物）→ 经 PermissionGate 完成权限判定
  ↓
权限通过 → 子会话工具据此创建子会话
  │  权限被拒 → SpawnError 的权限变体（复用 PermissionDenied）
```

### AuditLogEntry / AuditDisposition / AuditLogFilter

```
权限引擎（或审批流）对危险操作作出批准/拒绝处置
  ↓
构造 AuditLogEntry（操作内容 + RiskLevel + 处置）
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

### ResolvedAgentConfig / SubagentsConfig / MemoryConfig

```
config 按注册清单加载各 Agent config.json、补齐默认值 → ResolvedAgentConfig（含 subagents: SubagentsConfig、memory: MemoryConfig?）
  ↓
daemon 装配填充 AgentRegistry（以 agent_id 为键）
  ↓
下游模块经 AgentRegistry 只读查询：session / permission / system_prompt / tools / skills / gateway 消费配置字段
  ↓
memory 子模块（memory-miner / dreaming / active-searcher）按 ResolvedAgentConfig.memory 覆盖各自命名空间配置
```

ResolvedAgentConfig 由 config 一次性产出、运行期只读；其 `subagents` / `memory` 子结构随主结构整体流动。MemoryConfig 的 per-agent 字段级合并契约见 §架构 MemoryConfig 节。

### 会话/工具/斜杠/LLM 等辅助契约类型

这些辅助类型不构成独立的跨模块流动，而作为其宿主契约的载荷随调用传递：

- **会话/注入辅助**（SessionRole / SectionType / BootstrapMode / RequestContext / InjectionParams / TurnCounter / PendingMessage）：随 session 构建、system prompt 注入、消息排队等流程传递。
- **工具契约载荷族**：随 Tool / ToolRegistry / ToolRegistrar / ToolSession / ToolExecutor 的注册与调用传递。
- **Slash 执行辅助**（SlashContext / SystemAppendAction / ReplyAction）：随斜杠指令分派与 SideEffectContext 执行传递。
- **LLM/流式/中间件辅助**（LLMError / ErrorKind / StreamDone / MiddlewareContext / MiddlewareError）：随 LlmCaller / StreamingSink / OutboundMiddleware 调用传递。
- **system prompt 辅助**（DynamicPromptContext）：随 DynamicPromptBuilder 调用传递。
- **其他**（AgentToolsConfig / ConditionalSkillMatch）：随 AgentToolsConfigQuery / SkillListingProvider 调用传递。

## 模块关系

### NormalizedMessage

- **生产者**：IM Adapter 各平台插件（入站解析）——包括飞书、Discord、Telegram 等 IM 平台的 Adapter，以及 CLI 模块的 TerminalAdapter
- **消费者**：Processor Chain 入站（读取 NormalizedMessage 做内容标准化和 session_key 计算，产出 [ProcessedMessage](#processedmessage)）
- **无关**：LLM Provider（不接触 NormalizedMessage，只消费 ContentBlock[]）、Session（通过 Gateway 间接消费路由字段，不直接接触 NormalizedMessage）、Slash Command（斜杠指令消息本身经入站链归一化为 NormalizedMessage，但 SlashDispatcher 消费的是处理后的消息文本，不直接消费 NormalizedMessage 结构）

### ContentBlock

- **生产者**：Session（LLM 对话产出 UnifiedResponse，含 ContentBlock[]）、SlashDispatcher（斜杠指令回复以 SlashResult 变体产出 ContentBlock[]）、Processor Chain 入站 ContentNormalizer（入站方向包装标准化文本为 ContentBlock::Text 放入 ProcessedMessage.content_blocks）
- **消费者**：Processor Chain 出站（VerbosityFilter → OutboundRawLog）→ IM Adapter（按块类型渲染为平台原生格式并发送）
- **无关**：IM Adapter 入站链（入站方向产 NormalizedMessage，不涉及 ContentBlock[]）、Session 生命周期管理（不直接操作 ContentBlock[]，仅通过 Gateway 间接消费）、LLM Provider（LLM 调用返回原始 ContentBlock[]，由 Session 统一封装为 UnifiedResponse 后进入共享类型流；LLM Provider 不参与跨模块 ContentBlock 结构定义和传递流程）、[Gateway](../gateway/README.md)（Gateway 编排 Processor Chain 调度，不直接执行内容过滤/解析）

### StreamEvent

- **生产者**：LLM 模块（Protocol 层解析各协议 SSE 原生事件，ModelInterpreter 归一化为 StreamEvent，映射规则见 [llm protocol-mapping](../llm/protocol-mapping.md)）
- **消费者**：流式出站链路——Session（接收事件流并转发 Gateway）、Gateway（增量阶段调度 Processor Chain 与 IM Adapter 流式渲染）、Processor Chain 出站（VerbosityFilter 按块边界逐事件过滤）、IM Adapter 流式渲染器（逐事件消费，Text 块依赖 BlockDelta 携带的 [ContentDelta](#contentdelta) 逐行输出）
- **无关**：入站链路（入站不产生流式事件）、SlashDispatcher（斜杠指令回复为完整 ContentBlock[]，走批量模式）

### ContentDelta

- **生产者**：LLM 模块（协议 SSE 归一化为 StreamEvent 时随 BlockDelta 产出）
- **消费者**：流式渲染组件（StreamingRenderer 按 delta 变体累积行缓冲和块状态）、Session（重组完整 ContentBlock 写入对话历史）
- **无关**：批量路径（非流式响应直接返回完整 ContentBlock[]，无增量）、入站链路

### UnifiedResponse / UnifiedUsage

- **生产者**：LLM 模块（各供应商协议响应归一化产出 UnifiedResponse；LlmCaller 实现方 gateway 返回给 Session）
- **消费者**：Session（content_blocks 进入出站链路、写入对话历史；usage 记录统计）；UnifiedUsage 另被 StreamEvent::MessageEnd 作为收尾用量携带（消费者同 StreamEvent 流式链路）
- **无关**：IM Adapter 入站链、Permission、斜杠指令分派

### StreamingOutput

- **生产者**：common 的 [StreamingRenderer](core-traits.md#streamingrenderer) 默认实现（平台插件持有实例并委托调用，每次处理一批事件、刷新或超时检查后产出一批）
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

### RunningStats / CacheAnomalyInfo

- **生产者**：Session（持有并随每次 LLM 调用累加；压缩流程按需读取快照；缓存命中一致性校验写入异常事件）
- **消费者**：session（compaction 阈值判断、缓存命中一致性校验）、slash（/status 呈现命中率与累计用量）、llm（re-export 供模块内使用）
- **无关**：Processor Chain 出站过滤、IM Adapter 渲染

### CallTarget

- **生产者**：gateway 生产调用方与 llm 回退链（成功调用后上报实际服务的供应方/模型）
- **消费者**：session 与 gateway（记录本次调用实际服务的供应方/模型）
- **无关**：Processor Chain、IM Adapter、LLM 协议映射层

### ProcessedMessage

- **生产者**：Processor Chain 入站（ContentNormalizer 包装标准化文本为 ContentBlock::Text + SessionRouter 写 session_key 到 metadata）、Processor Chain 出站（VerbosityFilter 按当前等级过滤 ContentBlock[]）、Gateway（写出站 metadata 与出站历史）
- **消费者**：Gateway（入站：消费 content_blocks 做路由决策（session_key 用于日志追踪，路由查找用稳定路由键）+ metadata.message_type 做分型路由判断；出站：消费 content_blocks 做出站日志后传给 IM Adapter）、IM Adapter（消费 content_blocks 渲染为平台格式并发送）、CLI TerminalRenderer（同 IM Adapter，渲染为 ANSI 终端文本）
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

### CompactionResult / CompactionError

- **生产者**：session 模块（经 SlashEffectExecutor）→ **消费者**：Gateway（回发结果/错误）
- **无关**：LLM Provider、IM Adapter；压缩行为语义（触发条件、保留区、熔断、摘要格式）归 session 模块，配置结构本身（CompactConfig）已在上方 §架构定义

### ReasoningLevel / AgentRole

- **ReasoningLevel 生产者**：slash 模块（`/reasoning` 指令经 SlashEffectExecutor 设置）、config 的 `llm.reasoning_level` 全局默认档位；**消费者**：session / LLM 协议层（映射为原生参数）
- **AgentRole**：随 Agent 配置 / 会话创建确定；**消费者**：session、gateway（会话角色门控）
- **无关**：IM Adapter、Processor Chain

### InternalRequest / InternalMessage / SystemBlock / ToolDefinition

- **生产者**：Session（构造请求）、LLM Client 的缓存适配器（产出 system_blocks 与工具缓存标记）
- **消费者**：LLM 协议层（映射为供应商原生请求）
- **无关**：消息出站链路的共享类型（本族是 LLM 调用载荷，不进入出站 Processor Chain）

### CommunicationConfig / CommunicationCheckResult / CommunicationError

- **生产者**：agent 配置层提供通信白名单配置（默认仅父 Agent），session spawn 流程据此生成 CommunicationConfig 路由表（根 Session 无通信配置）
- **消费者**：session 的 spawn 流程（通信权限校验）
- **无关**：LLM Provider、IM Adapter、Processor Chain

### RiskLevel / PermissionRequest / PermissionOperation / PermissionDecision / PermissionDenied

- **生产者**：PermissionGate 实现方（permission 的权限引擎与审批队列；daemon 包装注入）
- **消费者**：PermissionGate 消费方（tools、session、gateway）
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
- **消费者**：agent（AgentRegistry 以 `agent_id` 为键存储并提供只读查询）、daemon（装配填充注册表）、gateway（经 AgentRegistry 查询）、session / permission / system_prompt / tools / skills（经 AgentRegistry 查询消费配置字段）、memory 子模块（memory-miner / dreaming / active-searcher 按 ResolvedAgentConfig.memory 覆盖各自命名空间配置）
- **无关**：LLM Provider、IM Adapter、Processor Chain

### MediaRef / SideEffectContext / CompactConfig / MemoryConfig 嵌套子类型

- **MediaRef**：生产者 IM Adapter 入站解析（媒体落盘后填引用），消费者 Gateway（媒体可得性与形态决策）与下游需读媒体的工具；媒体能力暂缓，当前不产出
- **SideEffectContext**：Gateway 构造并交给 SlashResultExecutor 执行时使用，不经其他模块，生命周期止于单个 SlashResult 执行
- **CompactConfig** 的载体：全局默认值在 config 的 session 配置（`config/session.json` 的 compact 节），per-agent 覆盖经 Agent 配置档案的 `compact` 字段（与 `memory` 的 per-agent 覆盖同构）。行为语义归 session 模块。
- **MemoryConfig 嵌套子类型**（MemoryStorageConfig / Dreaming* / SearchConfig / ForgettingConfig 等）：字段结构以 [memory/config](../memory/config.md) 为权威，本文档仅登记其归属 common（作为 ResolvedAgentConfig.memory 的字段类型）、不重复定义字段
- **无关**：LLM Provider、IM Adapter 入站链

### 会话/工具/斜杠/LLM 等辅助契约类型

- **生产者/消费者**：随各自宿主契约的实现方与消费方（见 [core-traits](core-traits.md) 模块关系）；本身无独立流动。
- **无关**：LLM Provider、IM Adapter 入站链（MessageType/MediaType/AdapterError 归属 IM 契约，ContentBlockType/ProcessError 归属 Processor 契约）