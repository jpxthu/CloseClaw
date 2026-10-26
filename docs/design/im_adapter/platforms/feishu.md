# 飞书插件

## 概述

- 关联需求文档：[requirements/im_adapter/feishu.md](../../../requirements/im_adapter/feishu.md)
- 飞书插件是 IMPlugin trait 的飞书平台实现。它基于飞书官方 CLI（lark-cli）封装平台通信与格式渲染：通过 CLI 的事件订阅长连接接收飞书事件，通过 CLI 命令完成消息发送，将 LLM 结构化输出渲染为飞书消息格式。当前仅覆盖私聊场景（群聊暂不设计）。飞书插件作为独立平台插件按需启用——仅在配置文件中显式配置并启用飞书平台时才加载，未配置则不加载。
- 多媒体能力按需求暂不设计：入站媒体下载落盘、media_refs / unavailable_media 填充、出站媒体上传发送均为暂缓设计，当前不实现（见 [im_adapter media-store](../media-store.md) 当前不实现范围）。

本设计仅覆盖 lark-cli 扁平事件（receive 类）与信封式非消息事件；webhook SDK 采集的旧样本（`event.message` 嵌套结构）不纳入当前设计。消息格式与事件字段细节以 `tests/fixtures/feishu/cli-poc/` 下的真实飞书 CLI 事件样本为权威来源（README 含字段语义实测记录与逐文件来源映射），平台接口行为与限制以飞书官方 CLI API 文档为权威来源；开发实现与单元测试的对照要求见 [im_adapter README 平台接口真实性验证](../README.md#平台接口真实性验证)。验证脚本随飞书插件提供，用真实账号经 lark-cli 发起发送与接收校验，凭证经 profile 提供、不进入代码库。

## 架构

飞书插件内部由 Adapter（协议通信）和 Renderer（格式转换）组成，通过 IMPlugin trait 对外暴露统一接口。Adapter 不直接连接飞书服务，而是托管 lark-cli 子进程完成平台通信。

### Adapter

**接入模式**：长连接（lark-cli event consume），平台不提供消息自动重发——队列满拒绝或入站解析丢弃的消息即终止，用户需重新发送；系统仅就队列满回复一次繁忙提示（队列语义见 [Gateway 入站流程](../../gateway/inbound-flow.md)）。

**CLI 进程编排**：

- **事件接收**：托管 `lark-cli event consume` 长连接子进程，事件以 NDJSON 逐行输出到 stdout；插件逐行读取原始事件并投入 Gateway 入站队列（附来源平台插件标识），出队后由 Adapter 解析。CLI 侧的连接维持与总线生命周期由 lark-cli 自管，插件不干预
- **订阅范围与进程模型**：消息事件（`im.message.receive_v1`）与 `im.message.reaction.created_v1` 各由一个 consume 子进程按事件类型订阅；`card.action.trigger` 因平台单消费者限制且能力暂缓，当前不订阅；`im.chat.member.bot.added_v1` 当前不订阅
- **进程守护**：监控子进程存活，异常退出或正常退出（达到事件上限/超时）后自动重启并继续消费；优雅关闭时先终止子进程再退出（见 [daemon shutdown](../../daemon/shutdown.md)）
- **事件去重**：按事件 ID 去重，平台重复投递只处理一次；与 Gateway 的队列重放去重（按消息身份）分层负责，不互相替代
- **凭证管理**：CloseClaw 配置仅保存 lark-cli profile 名；飞书凭证（app_secret 等）由 lark-cli 自行管理与刷新，不进入 CloseClaw 配置、不跨模块传递、不进入任何日志（日志脱敏遵循 [debug_log 框架](../../debug_log/README.md)）
- **外部依赖**：lark-cli 为外部二进制依赖，其事件结构与命令行为随版本可能漂移——以 fixture 采集时的版本为基准，漂移由平台接口真实性验证发现（见上文概述）

**事件解析**：解析层支持双事件格式——receive 类消息事件为扁平结构（字段在事件顶层），reaction.created 等非 receive 类事件为信封式结构（schema/header/event 混合），识别规则以 fixture 实测记录为准。事件标识 event_id 用于去重与日志追踪；tenant_key 仅随事件记录、不进入归一化结构，多租户隔离不在当前设计范围。解析产物按事件类型分流：

- **text 类型消息**（message_type=text）：receive 类事件为扁平结构，提取事件顶层 `content` 字段（字符串）作为消息正文。飞书已将正文中的表情符号以占位符文本形式（如 `[OK]`、`[赞]`）写入 content，Adapter 原样透传，不做额外转换
- **post 类型消息**（含图片的富文本消息，message_type=post）：CLI 已将 post 平文本化为一段 content——标题文本并入正文（平台事件不提供独立标题结构）。content 中若带有 markdown 样式标记（粗体/斜体/删除线/下划线/列表/引用/行内代码等）则原样保留并按 markdown 展开；样式是否出现在 content 中取决于平台客户端与 CLI 版本的展平结果（fixture 未覆盖全部样式）。Adapter 按 markdown 文本展开，不重建结构化 post 段，仅做三项转换：content 中的 @ 平台标签转为用户名文本（保留 @ 形式）、emoji 展开为占位符文本、内嵌图片标记替换为占位符。链接、邮箱、电话不解析、不保留——展平后若以 markdown 链接或联系方式形式出现，解析时整体丢弃（不将链接与 URL 送入正文）。内嵌图片按 `![Image](img_key)` 行内标记识别（标记内 key 即图片资源标识），正文在原位置以 `[图片]` 占位符替代；媒体能力暂缓——当前不下载图片、不填充 media_refs
- **file / image / audio 类型消息**（message_type=file/image/audio）：随媒体能力暂缓，当前不解析、不落盘、不填充 media_refs；启用前需解冻需求并重新设计（当前范围见 [im_adapter media-store](../media-store.md)）
- **sticker 类型消息**（message_type=sticker）：独立表情消息，提取表情标识渲染为文本占位符（如 `[OK]`）作为消息正文
- **会话与话题字段**：`chat_id` 标识消息所属会话、`chat_type` 区分私聊（p2p）与群聊（group，暂不设计）；`thread_id` 标识话题，`root_id` 为话题根消息、`reply_to` 为直接父消息（话题内消息均有；无 thread_id 的顶层消息无）。私聊的会话对端按「会话锚点构造」组合，chat_id 本身不进入 peer_id。无 thread_id 但有 reply_to 的普通回复按顶层消息处理（开新 Session），引用正文不拼接（引用不在范围），reply_ref 为本条消息 ID
- **引用/回复消息**：当前范围不识别引用——事件携带的 `reply_to` / `root_id` 不用于回查被引用正文，被引用内容不拼接进 content（需求 [im_adapter §F2](../../../requirements/im_adapter.md)）；出站定向投递仍用它们作为 reply_ref 目标
- **媒体落盘（暂缓）**：媒体下载落盘、media_refs 填充当前不实现；启用前需解冻需求并重新设计，范围见 [im_adapter media-store](../media-store.md) 与「平台接口权威来源」
- **媒体下载失败（暂缓）**：当前不实现媒体下载，unavailable_media 不填充；启用前需重新设计（见 [im_adapter media-store](../media-store.md)）

**平台接口权威来源**：平台接口行为与限制（含媒体大小/格式限制，属平台可得性约束、非系统配置项——系统不预判、不预校验，经 lark-cli 调用平台接口，平台拒绝即按对应操作失败处理）以两个来源为准：`tests/fixtures/feishu/cli-poc/` 实测样本（事件结构、字段语义、命令行为）与飞书官方 CLI API 文档（接口行为与限制）。开发实现时遇到平台限制相关问题，须查官方 API 文档、并对照 fixture 实测验证，不可仅凭既有代码或主观推断。已实测的平台限制包括：`card.action.trigger` 单消费者（第二消费者被拒）、cardkit 卡片 `element_id` 必填、cardkit 更新限流 1000 次/分（流式节流须遵守）；引用回查命令尚需补采 fixture，当前以官方 CLI 文档为准。工具调用失败向 Agent 返回错误须作为单元测试覆盖；消息发送失败按降级处理（向 User 提示，不向 Agent 回错）。

**会话锚点构造**（peer_id / reply_ref）：

| 消息形态 | peer_id | reply_ref |
|---------|---------|-----------|
| 顶层消息（无 thread_id） | 对方用户标识 + 本条消息 ID——每条顶层消息开启独立会话 | 本条消息 ID |
| 话题回复（有 thread_id） | 对方用户标识 + thread_id——同一话题归入同一会话 | 话题根消息 ID（root_id） |

同一会话内 peer_id 取值相同、不同会话间互不相同；当平台自然锚点不足以区分 Session 时，插件在取值内补充更细粒度的上下文信息。`thread_id` 标识话题（`omt_` 前缀），同一话题内各消息的 `root_id` 相同、即话题根消息 ID，出站定向投递以该 ID 作为 reply_ref——需求所述「出站携带话题标识定向回复」在平台侧即以话题根消息 ID 为回复目标；顶层消息无 thread_id，其 reply_ref 为本条消息 ID。话题首次出现时按 thread_id 作为会话对端建立 Session，后续该话题下的回复持续归入同一 Session；话题回复同时是对某条消息的引用，但引用正文不拼接（引用不在范围，见 [im_adapter §F2](../../../requirements/im_adapter.md)）。出站时按 reply_ref 定向投递：话题会话以根消息 ID 调用话题回复接口（reply-in-thread），顶层会话以消息 ID 定向回复原消息。

**账号映射**：接收方机器人应用由插件配置的 profile 确定（单实例单应用）；receive 类事件不带 app_id，信封式事件含 `header.app_id` 时用于校验、与 profile 不一致则丢弃并告警，`event.app_id` 为操作方应用、不用于身份映射。机器人应用不进入归一化字段，用于 Agent 路由。以「平台 + 本机器人应用 + 发送者标识」为键查询身份映射表获取 CloseClaw 本地账号标识，参与 session 路由。飞书发送者标识（open_id）按「应用 × 发送者」隔离——同一用户在不同应用语境下标识不同，跨应用 ID 不可直接互换，故映射键必须包含接收方机器人应用，同一用户跨应用的多个标识经身份映射归并为同一账号（映射表见 [config accounts.json](../../config/README.md)；跨应用 ID 隔离的实测证据见 fixture README「跨应用 ID 翻译」）。一个账号可绑定多个平台的多个发送者标识。

**入站过滤**：text 类型空 content 消息丢弃；post 类型 content 为空且无媒体引用时丢弃（媒体能力暂缓，当前仅按 content 判定）；解析失败的事件丢弃并记录告警日志，不产出 NormalizedMessage。

**事件分流**（消息事件之外）：

- `reaction.created`（表情回应）：记录调试日志（平台、消息、表情）。不进入消息通路，不注入对话——系统感知并记录，作为交互行为的可观测信号
- `bot.added`（机器人加入群聊）：记录调试日志。群聊会话暂不设计，不建会话
- `card.action.trigger`（卡片按钮/选择器点击）：**暂缓**——不进入消息通路，仅记录调试日志，见「暂缓能力」
- **群聊 receive 事件**：群聊不在产品设计范围；当前实现仅记录调试日志，不入消息通路

### Renderer

将 LLM 的结构化内容块（ContentBlock[]）渲染为飞书消息格式。渲染分两步：输出类型决策和卡片组装。卡片渲染与普通 IM 消息渲染保持同一量级，不引入额外显著处理开销（需求 [feishu §非功能-性能](../../../requirements/im_adapter/feishu.md)）。

**输出类型决策**：

| 条件 | 输出 |
|------|------|
| 纯文本，无格式标记、无换行、无 Thinking/ToolUse/ToolResult 块 | text 消息（仅批量模式；流式模式见下方流式渲染） |
| 含标题标记（`#`/`##` 等任意级别） | interactive 卡片 |
| 含粗体、斜体、删除线、下划线、行内代码、代码块、列表、引用、链接、分割线 | interactive 卡片 |
| 含换行符 | interactive 卡片 |
| 含 Thinking/ToolUse/ToolResult 块（单个或多个） | interactive 卡片 |
| 含 Image/Audio/File 块（单个或多个） | interactive 卡片（媒体能力暂缓，当前不出现此类出站块） |
| 交互控件（按钮/选择器） | **不支持**——入站交互事件不在当前范围，出站不渲染交互控件（见需求 [im_adapter](../../requirements/im_adapter.md)） |

判定按最强信号：任一富格式条件命中即输出 interactive 卡片；仅全部条件均不命中时才是 text 消息。

**卡片组装**：

```
ContentBlock[] → 卡片组装:
1. header 提取 - 首行 # 标题作为卡片标题(蓝色模板)
2. body 渲染(并行处理各块类型):
   - Text 块 → 先经 common 内容段解析,再按内容段映射为卡片 markdown 元素(飞书原生 markdown 渲染)
   - Thinking 块 → 渲染为折叠推理区块(飞书折叠面板,默认收起)
   - ToolUse 块 → 工具调用描述卡片(工具名 + 参数概要)
   - ToolResult 块 → 工具结果内容块;其中的代码块按代码块规则渲染(见 [代码块渲染](../code-render.md))
   - Image/Audio/File 块 → 暂缓:启用后本地媒体经 lark-cli 上传为平台资源再渲染为对应元素(上传自动获得平台访问地址,无需外部 url;本地文件来源与安全约束见 [im_adapter media-store](../media-store.md))
3. 产出飞书卡片 JSON
```

### Markdown 元素映射

| Markdown | 飞书卡片元素 |
|---------|-----------|
| `# 标题` | header.title（蓝色模板），标题行不进入 body |
| `## 标题` 及以下 | markdown 元素，原生渲染 |
| 段落 | 飞书 markdown 正文文本 |
| `**粗体**` / `*斜体*` | 飞书原生 markdown 渲染 |
| `~~删除线~~` / `<u>下划线</u>` | 飞书原生 markdown 渲染 |
| `` `行内代码` `` | 飞书 markdown 行内代码 |
| ` ```lang\n代码块\n``` ` | 飞书 markdown 代码块（平台自行高亮） |
| `> 引用` | 飞书 markdown 引用块 |
| `- 列表` / `1. 列表` | 飞书 markdown 列表 |
| `[链接](url)` | 飞书 markdown 链接 |
| `---` | hr 分割线元素 |

### 消息发送

Renderer 产出的 RenderedOutput 由 Adapter 映射为 lark-cli 命令发送，按两个维度组织：

- **按消息形态**：文本消息走文本发送命令；卡片消息走 interactive 卡片发送命令
- **按投递方式**：携带 reply_ref——话题会话以根消息 ID 话题回复（reply-in-thread），顶层会话以消息 ID 定向回复；无 reply_ref 的简化出站提示以 sender_id 作为 p2p 私聊投递目标
- **媒体消息（暂缓）**：Image/Audio/File 经 lark-cli 上传发送（自动上传；CLI 仅接受进程工作目录的相对路径，fixture `send-commands.md` 实测——Adapter 将媒体存储中的路径转换为相对进程工作目录的路径；平台大小/格式限制与拒绝处理见 [im_adapter media-store](../media-store.md) 与「平台接口权威来源」）；发出媒体先保留副本到媒体存储出站子目录。当前不实现

发送目标由 Gateway 从会话上下文传入（sender_id + reply_ref；队列满等无解析消息的场景由插件对事件做最小解析取 sender_id）。发送失败时记录日志并降级，不导致进程崩溃，由 Gateway 向 User 发送「回复发送失败」提示，不向 Agent 回错。

**流式渲染**：流式回复统一走流式卡片（增量更新需要卡片承载，输出类型决策表的 text 分支仅适用于批量模式）。飞书流式输出基于 cardkit 打字机更新，三步：创建流式卡片（streaming_mode）→ 发送引用该卡片的卡片消息 → 逐批更新卡片元素内容（sequence 递增 + uuid 防重）。更新批次节奏遵循 [im_adapter 流式渲染](../streaming-render.md) 的行缓冲与强制输出规则，并叠加平台更新节流——单批未达输出条件时合并内容延迟发送，遵守 1000 次/分限流。流式卡片的发送需携带 reply_ref 以按会话定向（话题 reply-in-thread / 顶层定向回复），具体命令组合以真实收发验证为准。**Thinking 折叠区**：流式卡片创建时预留折叠推理区（默认收起、初始为空），Thinking 块结束时整块写入，流式正常结束时折叠区留在卡片内。**标题**：流式卡片在首个输出单元确定后创建；首行若为 `#` 标题，创建时将其设为卡片 header、标题行不进入正文；否则卡片不设 header，内容按正文 markdown 渲染。后续批次只做元素级增量更新。**流式中断**：已缓冲的 Thinking 内容作为完整块补发到折叠推理区，与 Gateway 追加的「回复中断」错误提示一并呈现；错误提示经简化出站路径发送，不写入折叠区。流式出错降级遵循 [Gateway 出站流程](../../gateway/outbound-flow.md)。

### 暂缓能力

**设计保留、暂不实施**（启用时按本文档既有设计开发，无需重新设计）：

- **交互控件（button）**：不渲染为飞书交互控件；入站按钮交互事件到达后忽略并记录调试日志（见需求 [im_adapter 飞书 §F2](../../requirements/im_adapter/feishu.md)），出站不产出交互元素。
- **卡片交互事件（card.action.trigger）**：当前不订阅、不进入消息通路，事件到达后忽略并记录调试日志（需求 [im_adapter 飞书 §F2](../../../requirements/im_adapter/feishu.md)）

**未设计、当前不实现**（启用前需解冻需求并重新设计）：

- **媒体收发**：入站媒体下载落盘、media_refs / unavailable_media 填充、出站媒体上传发送均不实现；范围见 [im_adapter media-store](../media-store.md)
- **群聊**：群聊会话未设计（含群聊粒度路由、@ 触发规则）。fixture 中的群聊样本仅作字段语义参考，当前实现仅记录调试日志

## 数据流

### 入站路径

1. lark-cli event consume 子进程输出 NDJSON 事件行到 stdout，插件将原始事件投入 Gateway 入站队列（附来源平台插件标识）
2. 出队后 Adapter 逐行解析（双格式支持），按事件 ID 去重，trace_id 在解析时生成（见 [debug_log 框架](../../debug_log/README.md)）；队列重放去重由 Gateway 按消息身份负责
3. 消息事件（私聊）：构造 [NormalizedMessage](../../common/shared-types.md#normalizedmessage)（平台标识、发送者、peer_id、reply_ref、账号映射、正文、消息类型等字段）——仅私聊消息事件走此通路，其余事件见「事件分流」。媒体下载落盘与 media_refs / unavailable_media 填充随媒体能力暂缓，当前不执行
4. NormalizedMessage 进入 Processor Chain 入站处理 ← 日志：入站解析（平台、消息类型、解析耗时）

### 出站路径

1. Processor Chain 出站产出 [ProcessedMessage](../../common/shared-types.md#processedmessage)
2. Gateway 选择飞书插件
3. Renderer 遍历 ContentBlock[] 做输出决策，产出 [RenderedOutput](../../common/shared-types.md#renderedoutput)（输出类型 + 平台载荷）；媒体块的本地上传随媒体能力暂缓 ← 日志：出站渲染（平台、渲染耗时）
4. Adapter 按消息类型（message_type）与 reply_ref 映射 lark-cli 命令发送（流式模式按 cardkit 三步逐批更新）← 日志：平台 API 发送（平台、目标、耗时）

### 对外工具

飞书插件通过 IM Adapter 的模块级工具注册入口（见 [README 对外工具](../README.md#对外工具)）注册以下工具分组到 ToolRegistry。注册分组清单以 [tools/tool-registrar](../../tools/tool-registrar.md) 的 Registrar 表为准；本节只列能力范围，组内具名工具属实现细节，本文档不展开。

| 分组 | 能力范围 |
|------|---------|
| feishu_im | 飞书 IM 消息操作：发送、撤回、编辑消息，添加/撤回表情回应等 |
| feishu_calendar | 飞书日历：日程创建、查询、修改、删除 |
| feishu_task | 飞书任务：任务创建、查询、修改、删除 |
| feishu_bitable | 飞书多维表格：表格数据读写、记录管理 |
| feishu_doc | 飞书文档：文档创建、编辑、读取 |
| feishu_drive | 飞书云盘：文件上传、下载、管理 |
| feishu_sheet | 飞书电子表格：表格创建、编辑、读取 |

全部飞书工具默认延迟加载（详细描述不注入 System Prompt 一级索引）；工具实例首次调用时初始化。工具执行与 Adapter 的消息收发共用 lark-cli 命令通道与凭证管理。工具执行失败时向 Agent 返回明确错误信息与失败原因，不导致进程崩溃。

## 模块关系

- **互相调用**：Gateway——入站方向插件解析 CLI 事件产出 NormalizedMessage 交给 Gateway；出站方向 Gateway 选择插件调用渲染和发送
- **所属**：IM Adapter 模块的平台插件
- **上游依赖**：lark-cli（外部二进制：事件订阅、消息发送、媒体上传的执行载体；profile 管理飞书凭证）
- **相关模块**：Config（accounts.json 身份映射表、飞书平台启用配置、profile 名配置）、debug_log（入站解析、出站渲染、平台发送各环节记录调试日志）、[im_adapter media-store](../media-store.md)（媒体能力当前不实现）
- **下游**：Session（peer_id / reply_ref 经 Gateway 存入 Session 上下文，出站时取出回传；与 [im_adapter README 模块关系](../README.md#模块关系) 一致）
- **无关**：其他平台插件（各自独立实现 IMPlugin trait）、LLM Provider（IMPlugin 不调用 LLM）、Slash Command（IMPlugin 不参与指令解析）
