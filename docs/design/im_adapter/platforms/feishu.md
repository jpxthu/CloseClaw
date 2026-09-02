# 飞书插件

## 概述

飞书插件是 IMPlugin trait 的飞书平台实现。它基于飞书官方 CLI（lark-cli）封装平台通信与格式渲染：通过 CLI 的事件订阅长连接接收飞书事件，通过 CLI 命令完成消息发送，将 LLM 结构化输出渲染为飞书消息格式。当前仅覆盖私聊场景（群聊暂不设计）。飞书插件作为独立平台插件按需启用——仅在配置文件中显式配置并启用飞书平台时才加载，未配置则不加载。

本设计的消息格式与事件字段细节以 `tests/fixtures/feishu/cli-poc/` 下的真实飞书 CLI 事件样本为权威来源（README 含字段语义实测记录与逐文件来源映射），平台接口行为与限制以飞书官方 CLI API 文档为权威来源——两者均为本设计的必要输入，开发实现与单元测试必须对照两者进行验证，不可仅凭既有代码或主观推断。平台接口真实性验证（fixture 对照 + 真实收发校验）见 [im_adapter README 平台接口真实性验证](../README.md#平台接口真实性验证)。

## 架构

飞书插件内部由 Adapter（协议通信）和 Renderer（格式转换）组成，通过 IMPlugin trait 对外暴露统一接口。Adapter 不直接连接飞书服务，而是托管 lark-cli 子进程完成平台通信。

### Adapter

**CLI 进程编排**：

- **事件接收**：托管 `lark-cli event consume` 长连接子进程，事件以 NDJSON 逐行输出到 stdout，Adapter 逐行读取解析。CLI 侧的连接维持与总线生命周期由 lark-cli 自管，插件不干预
- **进程守护**：监控子进程存活，异常退出后自动重启并继续消费；优雅关闭时先终止子进程再退出（见 [daemon shutdown](../../daemon/shutdown.md)）
- **事件去重**：按事件 ID 去重，平台重复投递只处理一次
- **凭证管理**：CloseClaw 配置仅保存 lark-cli profile 名；飞书凭证（app_secret 等）由 lark-cli 自行管理与刷新，不进入 CloseClaw 配置、不跨模块传递、不进入任何日志（日志脱敏遵循 [debug_log 框架](../../debug_log/README.md)）
- **外部依赖**：lark-cli 为外部二进制依赖，其事件结构与命令行为随版本可能漂移——以 fixture 采集时的版本为基准，漂移由平台接口真实性验证发现（见上文概述）

**事件解析**：解析层支持双事件格式——receive 类消息事件为扁平结构（字段在事件顶层），reaction.created 等非 receive 类事件为信封式结构（schema/header/event 混合），识别规则以 fixture 实测记录为准。解析产物按事件类型分流：

- **text 类型消息**（message_type=text）：提取 `content.text` 字段作为消息正文。飞书已将正文中的表情符号以占位符文本形式（如 `[OK]`、`[赞]`）写入 content.text，Adapter 原样透传，不做额外转换
- **post 类型消息**（含图片的富文本消息，message_type=post）：展开 title 和 content blocks 为文本，保留标题、段落、有序/无序列表（含多级嵌套）、文本样式（粗体/斜体/删除线/下划线及组合）、@提及（渲染为 `@用户名`）、引用块、行内代码；emoji 元素展开为占位符文本；代码块展开为 markdown 代码块；超链接、邮箱、电话暂不解析、不保留。内嵌图片按 `![Image](img_key)` 行内标记识别（标记内 key 即图片资源标识），提取该资源标识落盘后填入 media_refs，正文在原位置保留 `[图片]` 占位符
- **file 类型消息**（message_type=file）：从 content 标签提取 file_key 与文件名，落盘后填入 media_refs，正文可为空
- **sticker 类型消息**（message_type=sticker）：独立表情消息，提取表情标识渲染为文本占位符（如 `[OK]`）作为消息正文
- **引用/回复消息**：提取被引用消息的文本内容，按 [im_adapter README](../README.md#implugin-trait-契约) 的引用/回复消息处理规则拼接（截断至 500 字符、blockquote 格式）
- **媒体落盘**：所有提取到的媒体资源（图片、文件）在解析阶段立即下载落盘并填充 media_refs，机制见 [im_adapter media-store](../media-store.md)；大小上限为平台可得性约束、非系统配置项，直接调用平台 API、拒绝即失败，见「平台接口权威来源」
- **媒体下载失败**：下载失败或平台拒绝（含超出限制）的媒体不落盘、不进 media_refs，其资源标识记入 unavailable_media，由 Gateway 按媒体不可得处理（见 [im_adapter media-store](../media-store.md) 入站落盘）

**平台接口权威来源**：平台接口行为与限制（含媒体大小/格式限制，属平台可得性约束、非系统配置项——系统不预判、不预校验，直接调用平台 API，平台拒绝即按对应操作失败处理）以两个来源为准：`tests/fixtures/feishu/cli-poc/` 实测样本（事件结构、字段语义、命令行为）与飞书官方 CLI API 文档（接口行为与限制）。开发实现时遇到平台限制相关问题，须查官方 API 文档、并对照 fixture 实测验证，不可仅凭既有代码或主观推断；平台拒绝路径（下载失败记入 unavailable_media、发送失败向 Agent 返回错误）须作为单元测试覆盖。

**会话锚点构造**（peer_id / reply_ref）：

| 消息形态 | peer_id | reply_ref |
|---------|---------|-----------|
| 顶层消息（无 thread_id） | 对方用户标识 + 本条消息 ID——每条顶层消息开启独立会话 | 本条消息 ID |
| 话题回复（有 thread_id） | 对方用户标识 + thread_id——同一话题归入同一会话 | 话题根消息 ID（root_id） |

同一会话内 peer_id 取值相同、不同会话间互不相同。出站时按 reply_ref 定向投递：话题会话以根消息 ID 调用话题回复接口（reply-in-thread），顶层会话以消息 ID 定向回复原消息。

**账号映射**：以「平台 + 本机器人应用 + 发送者标识」为键查询身份映射表获取 CloseClaw 本地账号标识，参与 session 路由。飞书发送者标识（open_id）按「应用 × 发送者」隔离——同一用户在不同应用语境下标识不同，跨应用 ID 不可直接互换，故映射键必须包含接收方机器人应用（映射表见 [config accounts.json](../../config/README.md)；跨应用 ID 隔离的实测证据见 fixture README「跨应用 ID 翻译」）。一个账户可绑定多个平台的多个发送者标识。

**事件分流**（消息事件之外）：

- `reaction.created`（表情回应）：记录调试日志（平台、消息、表情）。不进入消息通路，不注入对话——系统感知并记录，作为交互行为的可观测信号
- `bot.added`（机器人加入群聊）：记录调试日志。群聊会话暂不设计，不建会话
- `card.action.trigger`（卡片按钮/选择器点击）：**暂缓**——不进入消息通路，仅记录调试日志，见「暂缓能力」
- **群聊 receive 事件**：群聊场景暂不设计，群消息事件仅记录调试日志，不入消息通路

### Renderer

将 LLM 的结构化内容块（ContentBlock[]）渲染为飞书消息格式。渲染分两步：输出类型决策和卡片组装。

**输出类型决策**：

| 条件 | 输出 |
|------|------|
| 纯文本，无格式标记、无换行 | text 消息（仅批量模式；流式模式见下方流式渲染） |
| 含标题标记（`#`/`##` 等任意级别） | interactive 卡片 |
| 含粗体、斜体、删除线、下划线、代码块、列表、引用、链接、分割线 | interactive 卡片 |
| 含换行符 | interactive 卡片 |
| 含 Thinking/ToolUse/ToolResult 块（单个或多个） | interactive 卡片 |
| 含 Image/Audio/File 块（单个或多个） | interactive 卡片 |
| 含 DSL 交互指令 | **暂缓**——当前不产生渲染输出（见「暂缓能力」；启用后按保留设计渲染为交互组件） |

**卡片组装**：

```
ContentBlock[] → 卡片组装：
1. header 提取 — 首行 # 标题作为卡片标题（蓝色模板）
2. body 渲染（并行处理各块类型）：
   - Text 块 → markdown 元素（飞书原生 markdown 渲染）
   - Thinking 块 → 渲染为折叠推理区块（飞书折叠面板，默认收起）
   - ToolUse 块 → 工具调用描述卡片
   - ToolResult 块 → 工具结果内容块
   - Image/Audio/File 块 → 本地媒体经 lark-cli 上传为平台资源后渲染为对应元素（上传自动获得平台访问地址，无需外部 url；本地文件来源与安全约束见 [im_adapter media-store](../media-store.md)）
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

Renderer 产出的 RenderedOutput 由 Adapter 映射为 lark-cli 命令发送：

- **文本消息**：文本发送命令（peer 定向）
- **卡片消息**：interactive 卡片发送命令
- **定向回复**：携带 reply_ref——话题会话以根消息 ID 话题回复（reply-in-thread），顶层会话以消息 ID 定向回复
- **媒体消息**：Image/Audio/File 直接经 lark-cli 上传发送（自动上传；CLI 仅接受进程工作目录的相对路径，fixture `send-commands.md` 实测，Adapter 须转换；平台大小/格式限制见「平台接口权威来源」——系统不预校验，平台拒绝即按发送失败处理）；发出媒体先保留副本到媒体存储出站子目录（见 [im_adapter media-store](../media-store.md)）
- **表情回应**：表情回应命令（reactions create）

发送目标由 Gateway 传入（peer_id + reply_ref）。发送失败时记录日志并降级，不导致进程崩溃，Agent 继续运行。

**流式渲染**：流式回复统一走流式卡片（增量更新需要卡片承载，输出类型决策表的 text 分支仅适用于批量模式）。飞书流式输出基于 cardkit 打字机更新，三步：创建流式卡片（streaming_mode）→ 发送引用该卡片的卡片消息 → 逐批更新卡片元素内容（sequence 递增 + uuid 防重）。更新批次节奏遵循 [im_adapter 流式渲染](../streaming-render.md) 的行缓冲与强制输出规则，并叠加平台更新节流——单批未达输出条件时合并内容延迟发送，避免超出平台更新频率限制。流式出错降级遵循 [Gateway 出站流程](../../gateway/outbound-flow.md)。

### 暂缓能力

以下能力设计保留、暂不实施，防止误排期；启用时按本文档既有设计开发，无需重新设计：

- **DSL 交互指令（button/selector）**：不渲染为飞书交互组件，输出类型决策忽略 DSL 标记。对应渲染设计（首个按钮 primary 样式、selector 原生下拉组件、平台能力不足降级为按钮列表）保留：批量模式下 DslParser 解析的 DSL 指令渲染为卡片 action 元素，流式模式下 DSL 指令仅日志记录与出站历史写入（详见 [Gateway 出站流程](../../gateway/outbound-flow.md)）
- **卡片交互事件（card.action.trigger）**：不进入消息通路，仅记录调试日志。启用时按既有设计接入：卡片交互事件建模为 [CardActionEvent](../../common/shared-types.md#cardactionevent)，作为工具调用回执经 tool_result 通道注入对话
- **群聊**：群聊会话暂不设计（含群聊粒度路由、@ 触发规则）。fixture 中的群聊样本仅作字段语义参考

## 数据流

### 入站路径

1. lark-cli event consume 子进程输出 NDJSON 事件行到 stdout
2. Adapter 逐行解析（双格式支持），按事件 ID 去重，trace_id 在事件到达时生成（见 [debug_log 框架](../../debug_log/README.md)）
3. 消息事件（私聊）：媒体下载落盘 → 构造 [NormalizedMessage](../../common/shared-types.md#normalizedmessage)（平台标识、发送者、peer_id、reply_ref、账号映射、正文、消息类型、media_refs 等全部字段）——仅私聊消息事件走此通路，其余事件见「事件分流」
4. NormalizedMessage 进入 Processor Chain 入站处理 ← 日志：入站解析（平台、消息类型、解析耗时）

### 出站路径

1. Processor Chain 出站产出 [ProcessedMessage](../../common/shared-types.md#processedmessage)
2. Gateway 选择飞书插件
3. Renderer 遍历 ContentBlock[] 做输出决策，产出 [RenderedOutput](../../common/shared-types.md#renderedoutput)（输出类型 + 平台载荷）；媒体块触发本地上传 ← 日志：出站渲染（平台、渲染耗时）
4. Adapter 按 msg_type 与 reply_ref 映射 lark-cli 命令发送（流式模式按 cardkit 三步逐批更新）← 日志：平台 API 发送（平台、目标、耗时）

### 对外工具

飞书插件通过 IM Adapter 的模块级工具注册入口（见 [README 对外工具](../README.md#对外工具)）注册以下工具分组到 ToolRegistry：

- **feishu_im**：飞书 IM 消息操作（发送、撤回、编辑、表情回应等）
- **feishu_calendar**：飞书日历管理
- **feishu_task**：飞书任务管理
- **feishu_bitable**：飞书多维表格操作
- **feishu_doc**：飞书文档操作
- **feishu_drive**：飞书云盘操作
- **feishu_sheet**：飞书电子表格操作

全部飞书工具默认延迟加载，首次调用时才初始化。工具执行与 Adapter 的消息收发共用 lark-cli 命令通道与凭证管理。各工具分组的详细参数见 [tools 模块文档](../../tools/README.md)。

## 模块关系

- **互相调用**：Gateway——入站方向插件解析 CLI 事件产出 NormalizedMessage 交给 Gateway；出站方向 Gateway 选择插件调用渲染和发送
- **所属**：IM Adapter 模块的平台插件
- **上游依赖**：lark-cli（外部二进制：事件订阅、消息发送、媒体上传的执行载体；profile 管理飞书凭证）
- **相关模块**：Config（accounts.json 身份映射表、飞书平台启用配置、profile 名配置）、debug_log（入站解析、出站渲染、平台发送各环节记录调试日志）、[im_adapter media-store](../media-store.md)（媒体落盘与出站读取约束）
- **无关**：其他平台插件（各自独立实现 IMPlugin trait）、Session（IMPlugin 不直接参与 session 生命周期管理；peer_id/reply_ref 经 Session 上下文存储后由 Gateway 在出站时取出传入）、LLM Provider（IMPlugin 不调用 LLM）、Slash Command（IMPlugin 不参与指令解析）
