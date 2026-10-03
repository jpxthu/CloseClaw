# 代码块渲染

## 概述

代码块渲染负责将消息中的代码块在各平台渲染为平台原生格式——内容段的识别与划分由 common 内容段原语统一提供，各平台适配器按自身能力 emit。本子功能覆盖批量渲染路径；流式路径下的代码块处理见 [流式渲染](streaming-render.md)。

## 架构

代码块渲染集成在各平台 Renderer 中，按平台能力选择渲染策略。核心逻辑分为两步：识别代码块（委托 common 内容段原语，从 Text 块文本中划分出 CodeBlock 内容段，含语言标注）和按平台 emit（各适配器将 CodeBlock 组装为平台原生格式）。上游 Processor Chain 的 DslParser 已从 Text 块剥离 DSL 行，进入渲染的文本不含 DSL 指令。

**平台渲染策略**（未列出的新平台默认采用「其他」策略）：

| 平台 | 渲染方式 | 说明 |
|------|---------|------|
| 飞书 | 原生 markdown | Text 块整段作为卡片 markdown 元素，围栏代码块由平台端原生高亮 |
| 终端（CLI） | ANSI / 纯文本 | ANSI 模式下按语言注入颜色码；两种模式均插入行号等结构性标记。终端专属细节见 [cli/Terminal Renderer](../cli/renderer.md) |
| 其他 | 纯文本 | 以纯文本形式展示代码，保留围栏标记原文 |

语言标注由 common 内容段解析从围栏标记中提取并随内容段携带，平台自行决定支持的语言列表。

## 数据流

1. Renderer 处理 ContentBlock::Text，委托 common 内容段解析得到内容段序列（[ContentSegment](../common/shared-types.md#contentsegment--内容段落解析)）
2. 内容段解析按行切分：围栏代码块收集为单个 CodeBlock（含语言标注）、未闭合围栏（含开围栏行）按普通 markdown 文本行原样处理、分隔线识别为 Hr、其余为 Markdown
3. 命中的 CodeBlock 按平台策略 emit（各平台策略见 §架构「平台渲染策略」表）
4. 该 Text 块的其余内容段按各自类型渲染，整段渲染结果并入 RenderedOutput，交由 IMPlugin 的 send 方法发送

## 模块关系

- **上游**：common（提供平台无关的内容段原语与内容段解析，见 [common/shared-types](../common/shared-types.md#contentsegment--内容段落解析)）
- **下游**：本子功能产出并入 RenderedOutput，由 IMPlugin 的 send 方法发送至平台
- **所属**：IM Adapter 模块的渲染子功能（持有并委托 common 内容段原语，平台专属 emit 在适配器自身；terminal 渠道的代码块实现归属 [cli 模块](../cli/renderer.md)）
