# 代码块渲染

## 概述

代码块渲染负责将消息中的代码块在各平台按自身能力渲染——代码块的识别与内容段划分由 common 内容段原语统一提供，各平台适配器按能力组装为平台原生格式。覆盖 `ContentBlock::Text` 与 `ToolResult` 内容中的代码块；流式路径下的代码块处理见 [流式渲染](streaming-render.md)。需求见 [im_adapter §F5](../../requirements/im_adapter.md)。

## 架构

代码块渲染集成在各平台 Renderer 中。核心逻辑分两步：识别代码块（委托 common 内容段原语，从 Text 块或 ToolResult 内容中划分出 CodeBlock 内容段，含语言标注）和按平台组装（各适配器将 CodeBlock 转为平台原生格式）。

**代码块边界**：代码块以单个 Text 块内的围栏为界——围栏须在同一 Text 块内闭合；跨块的围栏不识别为代码块，按未闭合文本处理。流式路径同样在 Text 块结束时重置代码模式，保证批量与流式判定一致。

**上游预处理**：进入渲染的文本以 Processor Chain 的实际输出为准（出站链定义见 [Processor Chain 出站链路](../processor_chain/outbound-chain.md)）。

**平台渲染策略**（未列出的新平台在确认自身能力前按纯文本档处理，新增平台时须在本文登记其能力档）：

| 平台 | 渲染方式 | 说明 |
|------|---------|------|
| 飞书 | 原生 markdown | 正文按内容段映射为卡片元素（首行一级标题 `#` 提取为卡片 header、分割线单独映射），围栏代码块交平台原生高亮（元素映射见 [飞书插件](platforms/feishu.md)） |
| 终端（CLI） | ANSI / 纯文本 | 保留围栏与语言标注行、逐行插入行号，ANSI 模式按语言着色；终端专属细节见 [cli/Terminal Renderer](../cli/renderer.md) |
| 其他 | 纯文本 | 以纯文本展示代码并标明语言（重建围栏与语言标注），不做语法高亮；不承诺还原原始围栏形态 |

需求 [§F5](../../requirements/im_adapter.md) 的三档能力对应关系：平台原生支持高亮的用原生能力（飞书）；平台不原生支持但显示环境可高亮的用显示环境能力（终端的 ANSI 着色）；两者都不可用的以纯文本展示（其他平台）。无语言标注的代码块不做语言高亮，按所在平台档位的纯文本形态展示。

## 数据流

1. Renderer 处理 `ContentBlock::Text` 或 `ToolResult` 内容，委托 common 内容段解析得到内容段序列（[ContentSegment](../common/shared-types.md#contentsegment--内容段落解析)）
2. 内容段按 common 解析规则切分（围栏代码块、未闭合围栏、分隔线、markdown 文本等，规则见 common，不在此重复）；未闭合围栏按普通 markdown 文本行处理，其中的 markdown 记号仍会被平台解释
3. 命中的 CodeBlock 按平台策略组装（各平台策略见 §架构「平台渲染策略」表）；该 Text 块或 ToolResult 的其余内容段按各自类型渲染
4. 整段渲染结果并入 RenderedOutput，由 Gateway 执行出站中间件后交 IMPlugin 发送（渲染与发送分离见 [IM Adapter README 出站路径](README.md#出站路径)）

## 模块关系

- **上游**：Processor Chain（出站方向：产出过滤后的 ProcessedMessage）、Gateway（调用插件渲染）、common（提供平台无关的内容段原语与内容段解析）
- **下游**：Gateway（消费 RenderedOutput，经中间件后交发送）、IMPlugin 发送能力（投递到平台；terminal 渠道写入 stdout）
- **共享类型**：[common/shared-types](../common/shared-types.md)（消费：ContentSegment、ContentBlock、ProcessedMessage；产出：RenderedOutput）
- **所属**：IM Adapter 模块的渲染子功能；common 内容段原语由各平台适配器持有并委托调用，terminal 渠道的代码块实现归属 [cli 模块](../cli/renderer.md)
