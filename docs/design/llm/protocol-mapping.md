# 协议映射

## 概述

协议映射规范 OpenAI 与 Anthropic 两种 LLM 协议的响应到统一内容块与统一流式事件（[StreamEvent](../common/shared-types.md#streamevent)）的转换规则。

本规范在 LLM 模块五层架构中充当 Protocol 层与 ModelInterpreter 层的桥梁：Protocol 层按协议解析原始响应，ModelInterpreter 按此映射归一化。规范中立于具体供应商，只描述协议本身；各供应商的特有行为（如 MiniMax OpenAI 协议下 thinking 混在文本中的标签格式）在各 provider 文档中说明。

## 架构

### 统一内容块

LLM 响应路径只产出以下四种内容块类型；`ContentBlock` 的其余变体（图片、音频、文件）不由 LLM 响应产生，见 [common/shared-types](../common/shared-types.md#contentblock)：

| 统一块 | 含义 |
|--------|------|
| Text | 模型生成的文本回复 |
| Thinking | 模型的思考过程（是否向界面展示由展示等级决定） |
| ToolUse | 模型请求调用工具 |
| ToolResult | 工具执行结果（作为历史消息传入） |

### 协议→统一块映射

**OpenAI 协议**：

| 统一块 | 协议来源 |
|--------|---------|
| Text | `choices[].message.content` |
| Thinking | `choices[].message.reasoning_content` |
| ToolUse | `choices[].message.tool_calls[]` |
| ToolResult | 消息数组中以 `role: "tool"` 出现 |

OpenAI 协议下，content 和 reasoning_content 在同一个 message 对象中共存。`reasoning_content` 是兼容端点的扩展字段，不是标准 OpenAI 字段；端点不返回时该响应不产生 Thinking 块。两者各自独立产出内容块：content 非空则产出 Text 块，reasoning_content 非空则产出 Thinking 块。content 为空时，reasoning_content 仍产出 Thinking 块，不转成 Text 块，也不得把思考内容当作普通文本输出；是否向界面展示由展示等级决定。

**Anthropic 协议**：

| 统一块 | 协议来源 |
|--------|---------|
| Text | `content[].type: "text"` → `text` |
| Thinking | `content[].type: "thinking"` → `thinking`（含 `signature` 签名字段） |
| ToolUse | `content[].type: "tool_use"` |
| ToolResult | `content[].type: "tool_result"` |

Anthropic 协议下，content 是类型化结构数组。每个元素通过 `type` 字段区分内容块类型，thinking 块独立于 text 块。signature 是 thinking 块的可追溯签名字段，与 thinking 文本一并保留。

### 统一流式事件

流式输出通过 [StreamEvent](../common/shared-types.md#streamevent) 五种统一事件传递（类型语义与消费契约见 common 定义），本表定义两种协议 SSE 到该事件的映射规则，屏蔽协议在事件粒度上的差异：

| StreamEvent 事件 | OpenAI SSE 来源 | Anthropic SSE 来源 |
|---------|----------------|-------------------|
| BlockStart（块开始） | 首个非空 `delta.content` / `delta.reasoning_content` / `delta.tool_calls[i].id` 出现（同一增量中的多个 `tool_calls` 各开一个 ToolUse 块） | `content_block_start` 事件 |
| BlockDelta（块增量） | `delta.content` / `delta.reasoning_content` / `delta.tool_calls`（按 `index` 归属到对应 ToolUse 块） | `content_block_delta` 事件（`text_delta` / `thinking_delta` / `signature_delta` / `input_json_delta`） |
| BlockEnd（块结束） | 当前块为 Text/Thinking 时，`finish_reason` 为 `stop` 或 `length` 结束该块；首个 `delta.content` 出现时结束 Thinking 块；首个 `delta.tool_calls` 出现时结束当前 Text/Thinking 块；`finish_reason=tool_calls` 结束全部 ToolUse 块 | `content_block_stop` 事件 |
| MessageEnd（消息结束） | 结束原因到达后消费可选用量尾块，再由 `[DONE]` 或已给出结束原因的流正常结束确认完成；文本与工具调用路径相同 | `message_delta(stop_reason)` 后接 `message_stop` |
| Error（错误） | HTTP 错误状态码 + 错误 body，或流内无状态码的连接中断 | HTTP 错误状态码 + `error` 事件，或流内 `error` 事件与连接中断 |

Anthropic SSE 事件序列的典型顺序：`message_start` → `content_block_start(thinking)` → 若干 `thinking_delta` → 一个 `signature_delta` → `content_block_stop` → `content_block_start(text)` → 若干 `text_delta` → `content_block_stop` → `message_delta` → `message_stop`。

工具调用场景的典型顺序：`message_start` → `content_block_start(tool_use)` → 若干 `input_json_delta` → `content_block_stop` → `message_delta` → `message_stop`。`input_json_delta` 的 `partial_json` 字段携带工具调用的 JSON 参数片段，粒度因供应商而异（逐字符到一次性全量），首次可为空字符串。

混合响应（文本 + 工具调用）的典型顺序：`message_start` → `content_block_start(text)` → 若干 `text_delta` → `content_block_stop` → `content_block_start(tool_use)` → 若干 `input_json_delta` → `content_block_stop` → `message_delta` → `message_stop`；各内容块按 `index` 递增，块边界由 `content_block_start/stop` 给出。

OpenAI SSE 事件序列的典型顺序：`delta.role=assistant` 帧 → 若干 `delta.reasoning_content`（先于 content，归一化为 Thinking 块增量）→ 首个 `delta.content` 触发 Text 块开始 → Text 增量若干帧 → `finish_reason=stop` 或 `length` 结束内容块 → 消费可选用量尾块 → 流完成后产出 MessageEnd。推理与文本增量交替出现时，按首次出现顺序切换当前块，不重开已结束的块。

混合响应（文本 + 工具调用）：Text 增量若干帧 → `delta.tool_calls` 首次出现时隐式结束 Text 块 → 启动 ToolUse 块 → `finish_reason=tool_calls` 结束 ToolUse 块 → 消费可选用量尾块 → 流完成后产出 MessageEnd；不能在工具结束原因刚到达时丢弃后续用量。

并行工具调用：同一增量中的多个 `tool_calls` 各自成块，`tool_calls[].index` 只用于把增量归属到对应工具调用；StreamEvent 的块序号按本次响应内块的出现顺序从零递增，与工具调用序号不要求相等。工具调用的 id 与 name 来自首个包含该调用的增量（Anthropic 为 `content_block_start`），作为 ToolUseId、ToolUseName 增量先行产出。全部 ToolUse 块在 `finish_reason=tool_calls` 时结束，`MessageEnd` 只产出一次。

结束原因归一化：统一 `finish_reason` 取值限定为 `stop` / `length` / `tool_use`。OpenAI 的 `stop`、`tool_calls`、`length` 分别对应 `stop`、`tool_use`、`length`；Anthropic 的 `end_turn`、`stop_sequence` 对应 `stop`，`tool_use`、`max_tokens` 分别对应 `tool_use`、`length`。协议未给出或给出未列出的取值时为缺失，保留原始值供诊断，不默认成 `stop`，也不据此推断成功或失败。

流中途断开：流既无结束原因也无结束标记（`[DONE]` / `message_stop`）即中断时，只产出 `Error` 事件，不产出 `MessageEnd`，不按正常完成处理；已收到的内容块是否写入历史由上层按不完整响应规则决定。

与 Anthropic 的差异：OpenAI 的 `finish_reason` 仅在流末尾出现一次，不区分内容块；Anthropic 通过 `content_block_stop` 逐块标注结束边界。

### 用量字段

两种协议的用量字段归一化为 [UnifiedUsage](../common/shared-types.md#unifiedresponse--unifiedusage)（协议未携带的字段以缺失值表示）。下表以 `tests/fixtures/llm/` 的真实采集响应为唯一事实依据，逐项标注证据状态：**实测有值**（实采出现该字段且有非零取值）、**字段存在、无正值**（实采出现该字段，观测值均为零或多数样本缺失）、**实采未见**（观察到的响应中没有该字段）。

| 统一字段 | 含义 | OpenAI 协议来源 | Anthropic 协议来源 |
|---------|------|----------------|-------------------|
| prompt_tokens | 输入 token 总数（含缓存命中与缓存写入） | `usage.prompt_tokens`（实测有值，已含缓存部分） | `usage.input_tokens` + `usage.cache_read_input_tokens` + `usage.cache_creation_input_tokens`（实测有值；`input_tokens` 仅计未命中部分） |
| completion_tokens | 输出 token 数 | `usage.completion_tokens`（实测有值） | `usage.output_tokens`（实测有值） |
| reasoning_tokens | 推理消耗 token 数 | `usage.completion_tokens_details.reasoning_tokens`（实测有正值；部分样本整体缺失该字段） | 实采未见独立字段；推理消耗计入 `output_tokens`，不单列 |
| cache_read_tokens | 命中缓存的输入 token 数 | `usage.prompt_tokens_details.cached_tokens`（实测有正值；部分样本该对象整体缺失） | `usage.cache_read_input_tokens`（实测有正值） |
| cache_write_tokens | 写入缓存的 token 数 | 实采未见独立字段。兼容端点另返回 `prompt_cache_hit_tokens` 与 `prompt_cache_miss_tokens` 作为命中、未命中的拆分，未命中不等于写入，也不能由 `prompt_tokens - cached_tokens` 或 `miss` 反推写入 | `usage.cache_creation_input_tokens`（字段存在，实采观测值均为零，无正写入样本） |

「实采未见」只说明现有样本没有该落点，不构成该协议永不提供该分项的判定；实采出现新落点后按其取值扩展本表，不沿用旧结论。

取值规则：

- **缺失与零是两种事实**：字段未出现、值为 `null`、细节对象为空，均按缺失处理，不补造为零；缓存相关字段按需求 [llm §F9](../../requirements/llm.md) 显示为零，其余分项的展示由上层定义。字段出现且取值为零才是明确的零。
- **命中与写入互不重叠**：输入 token 总数同时包含两者。OpenAI 的 `prompt_tokens` 已含缓存部分，不得再加一次命中量；Anthropic 的 `input_tokens` 只计未命中部分，须加上命中与写入两项；缺失的缓存分项在合成输入总数时按零计入（未报告即视为该项为零），但报告字段仍保持缺失，不补造为零。跨协议一致性校验统一以「输入 token 总数」为口径。
- **原文优先**：`expect` 标注、供应商 README 与 API summary 是加工结论，不是供应商原始响应；字段、取值与语义的判断以响应原文为准。供应商特有形态（如过短 reasoning 视为普通文本、thinking 标签混排）在 provider 文档定义其对统一块的最终归属。
- **输出拆分**：输出总量包含推理消耗。协议同时报告输出总量与推理量时，文本输出量按输出总量减推理量得出；推理量未报告时不做拆分，不臆造文本与推理的分布。

流式场景下的用量到达时机与合成规则：

- **OpenAI**：最终 chunk 携带 `usage`，协议仅在请求启用 `stream_options.include_usage` 时保证返回。chunk 的 `usage` 为 `null`、以及整条流没有任何携带 `usage` 的 chunk，都按缺失处理；不累加各 chunk，不由增量帧数或字符数估算，也不依据请求元数据反推该开关是否发送。工具调用路径的用量尾块与文本路径同样处理，不能在结束原因到达后丢弃尾块。
- **Anthropic**：`message_start` 提供输入侧初值，`message_delta` 给出最终值。最终值逐字段覆盖初值，不与初值累加；`message_delta` 缺失的分项回退 `message_start` 的初值；整条流没有最终用量时，初值不完整即按缺失处理。流式实现须处理初值为零或缺失、最终值逐字段覆盖初值、以及整条流没有最终用量的情形；现有实采覆盖初值为零、初值只含部分分项与最终值覆盖，其余按健壮性要求处理。
- `MessageEnd` 于协议流结束处产出（Anthropic 在 `message_stop` 之后），用量在完整到达后并入（OpenAI 为携带 usage 的最终 chunk；Anthropic 的最终用量来自 `message_delta`；协议未提供用量时其 usage 为空），输入 token 总数按本节定义合成，不使用流式增量重新计数。协议未携带的字段归一化为缺失（由上层按 0 显示，见 [session/llm-session-enhancements](../session/llm-session-enhancements.md) 的用量统计）。

### 多轮对话增量处理

多轮对话记录以历史追加为主：上一轮的消息不被修改，只在尾部追加新消息。这是前缀缓存生效的前提；实际发送前被测侧可能清理孤立 Thinking 或压缩历史，前缀因此不总是纯追加，缓存命中校验以前缀快照为准，见 [session/llm-session-enhancements](../session/llm-session-enhancements.md)。

多轮场景的 fixture 有两种记录方式：普通多轮（`multi-turn`）在 `request` 中给出完整 messages 数组与单次响应；逐轮增量场景（`cache`、`context-pressure`）使用 `turns` 记录每轮的完整 messages 与对应响应，展示消息列表在各轮之间的增量变化。

### 本地测试关联

[Fake LLM Server](../fake_llm/README.md) 的黑盒并发测试使用独立于协议正文的 HTTP 关联信息，不把会话身份编码进提示词、模型名、工具或缓存前缀。普通供应商请求默认不携带该信息；只有供应商接入配置显式启用本地测试关联后才生成，配置载体见 [config](../config/README.md)。这不改变双协议的原生请求/响应格式，也不要求替换被测二进制或为每个 Session 改端点。

关联信息包含测试运行命名空间和匿名会话键。命名空间由运行器在服务端创建，并写入本轮本地测试接入配置；LLM 模块在本轮运行内共享匿名映射，将 [InternalRequest 的会话身份](../common/shared-types.md#internalrequest--internalmessage--systemblock--tooldefinition) 映射为稳定的不透明键，不发送原始 Session、Agent 或 User 标识。不同 Session 的键不相同，同一 Session 在重试、流式/非流式与模型/协议切换时保持同键；子 Session 使用自身身份，不沿用父键。映射按测试运行隔离，运行结束后销毁，不作为权限凭据或真实供应商缓存键。

会话上下文由 [Session 调用链](../session/llm-session-enhancements.md) 填入并由 LlmCaller 实现方透传，Protocol 不将其序列化进 JSON，Provider 在发 HTTP 请求时附加关联信息。缺少会话身份不伪造公共键，服务端按 [场景引擎](../fake_llm/scenario-engine.md) 的无状态/有状态规则处理。模型发现没有会话身份，只携带运行命名空间，使用服务端独立发现序列。

本地测试模式仅允许实际连接到本机回环地址的端点，关闭代理和重定向；启用配置却指向非回环目标时拒绝该接入，不得先发送后告警。不能只凭主机名或配置中的声称判定为本地。关联信息不接受 extra_body 或调用方自定义 HTTP 头覆盖，不传播至外部供应商；其他测试客户端可按相同带外契约提供匿名键，服务不读取 CloseClaw 内部存储。

### 消息历史缓存

各协议在请求 messages 数组上的缓存标记策略不同。消息历史的缓存标记使下一轮请求的完整前缀（上轮的 system prompt + 全部 messages）命中 cache，仅新增的消息尾部按全价计费。

**Anthropic 协议**：协议本身支持显式缓存标记。在 messages 数组的尾部消息上打 `cache_control: {"type": "ephemeral"}`，指向前缀匹配点。消息数组只追加不修改，前缀稳定使缓存持续命中。是否接受或生效该标记取决于供应商 API 能力，由 [cache-adapter](cache-adapter.md) 适配与降级；标记的断点预算也见该文档。

**OpenAI 兼容协议**：使用服务端自动前缀缓存，无需客户端在 messages 上显式标记。消息数组只追加、前缀不变，历史消息的前缀部分自动被服务端缓存覆盖。

**缓存写入量**：Anthropic 的 `cache_creation_input_tokens` 在现有实采中均为零，说明采样未触发写入回写，不能据此判定该协议不产生写入，也不能据此推断 OpenAI 兼容协议存在等价字段；缓存写入的真实取值以实采为准，见用量表的证据状态。

> 消息历史缓存标记属于 Protocol 层的序列化行为，与缓存适配器（处理 system prompt 缓存标记）职责分离。缓存适配器详见 [cache-adapter](cache-adapter.md)。

## 数据流

```text
请求方向：
  上层业务 -> Protocol 层按协议序列化请求（含缓存标记与用量开关）
    -> Provider 附加本地测试关联信息（仅本地测试模式）
      -> 供应商 API

响应方向（本规范定义）：
  供应商 API 返回 JSON 或 SSE
    -> Provider 层返回 HTTP body
      -> Protocol 层按协议解析为内部结构
        -> ModelInterpreter 按本映射归一化
          -> Text / Thinking / ToolUse 内容块与 StreamEvent

分支：
  非流式：一次性产出内容块与用量
  流式：按块边界产出 BlockStart / BlockDelta / BlockEnd，结束于 MessageEnd
  协议错误：产出 Error（HTTP 错误状态码 + 错误体/错误事件）
  流中断：只产出 Error，不产出 MessageEnd，不按正常完成处理
```

## 模块关系

- **上游**：Protocol 层（提供按协议解析后的内部结构）
- **下游**：ModelInterpreter（消费映射规则，产出统一内容块和流式事件）
- 本规范被各 provider 文档引用
- **协议测试基线**：真实采集响应（`tests/fixtures/llm/`）是协议字段、取值与流式形态的事实来源，本模块的解析与归一化应与其保持一致；[fake_llm/fixtures](../fake_llm/fixtures.md) 的合成样本用于生成编码回归与消费方形状契约测试，不替代真实采集，也不作为供应商行为证明。
- **本地测试关联消费方**：[fake_llm/endpoints](../fake_llm/endpoints.md) 解析带外匿名关联信息；生产者为 [Session 调用链](../session/llm-session-enhancements.md)，配置载体见 [config](../config/README.md)。关联仅用于测试输入分区，不参与内容块、用量或缓存前缀映射。
