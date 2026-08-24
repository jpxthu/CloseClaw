# Terminal Renderer

## 概述

TerminalRenderer 是 terminal 渠道的出站渲染组件。它接收 ContentBlock[]（定义见 [common ContentBlock](../common/shared-types.md#contentblock)）和 DSL 解析结果，将结构化内容转换为 RenderedOutput（按终端能力为 ANSI 或纯文本格式）。渲染是纯数据转换，实际的 stdout 写入由 TerminalPlugin 的 send 方法完成——遵循 IM Adapter 框架「渲染与发送分离」的设计原则。

## 架构

TerminalRenderer 按 ContentBlock 类型分派渲染策略。流式渲染是独立路径，不经过本组件——由 IM Adapter 模块的通用流式渲染组件驱动，TerminalPlugin 组合持有该组件并在流式模式中委托调用，逐行产生增量输出（详见 [IM Adapter 流式渲染](../im_adapter/streaming-render.md)）。批量渲染的顺序路径见 §数据流，渲染机制分两部分：终端能力检测（确定渲染模式与宽度约束）与块类型渲染策略（见 §块类型渲染规则）。

### 终端能力检测

终端能力检测由 [platform 模块](../platform/README.md)提供（检测规则与主流终端覆盖见 platform 文档），TerminalRenderer 在渲染时消费其结果：

- **ANSI 能力标记**：支持 → 启用 ANSI 模式；不支持 → 回退纯文本模式
- **终端可用宽度**：用于各块输出的截断判断

纯文本模式下，所有 ANSI 转义序列被移除，仅保留文本内容和边界标记（各块类型的结构性标记，如 Thinking 的 `[Thinking]`/`[end of thinking]`、ToolUse 的 `⚙` 前缀、代码块的反引号边界、语言标注行与行号）；markdown 格式标记（含引用前缀 `> `）保留原样输出——纯文本模式不做格式转换，只做 ANSI 剥离，宽度截断等布局规则不受渲染模式影响。

### 块类型渲染规则

**Text 块 — 普通文本**

纯文本直接输出。ANSI 模式下，包含 markdown 格式标记（标题、粗体、斜体、引用、链接、分割线）时转为 ANSI 样式：标题用 bold，粗体用 bold，斜体用 italic，引用用 dim 前缀 `│ `，链接渲染为 `文本 (url)`，分割线渲染为 `───`。纯文本模式下不做格式转换，全部格式标记（含引用前缀 `> `）原样保留。列表（`- `/`* ` 无序、`1. ` 有序）无对应 ANSI 样式，两种模式下均保留原始 markdown 标记原样输出。

**Text 块 — 代码块**

ANSI 模式下按语言标注注入颜色码（关键字、字符串、注释等；纯文本模式无颜色码），语言标注从 markdown 代码块标记中提取并转为独立的语言标注行（取代原反引号标记行的语言后缀），代码内容逐行附加行号——两种渲染模式、无论语言是否支持高亮均统一插入，反引号边界行两种模式下均保留。不支持的语言回退无高亮纯文本输出。代码块高亮策略详见 [IM Adapter 代码块渲染](../im_adapter/code-render.md)。

**Thinking 块**

折叠展示：ANSI dim 样式包裹，首行 `[Thinking]`，末行 `[end of thinking]`，内容缩进 2 空格。不支持 ANSI 时用 `[Thinking]` / `[end of thinking]` 边界包围，内容同样缩进 2 空格。

**ToolUse 块**

展示工具名称和参数，两种模式均以 `⚙` 前缀开头。ANSI 模式下工具名用 bold + cyan，参数用 dim；纯文本模式下保留 `⚙ tool_name(...)` 结构，仅去除样式。参数以原始 JSON 字符串形式展示，格式为 `⚙ tool_name({"key":"value",...})`。参数不做 key=value 格式化解析。

**ToolResult 块**

展示工具执行结果，内容按 Text 块规则渲染（含 markdown 转换与代码块高亮）。超出渲染后行数上限（约 20 行）时截断，截断标记为块末独立一行 `... (truncated)`；行内超宽截断遵循 §数据流第 4 步的通用截断规则。ANSI 模式下用 dim 样式。

**不支持的内容块**

Image、Audio、File 等终端不支持的块类型，渲染为带文件名的占位符（格式如 `[image: name]`，定义见 [common ContentBlock](../common/shared-types.md#contentblock) 各变体的终端渲染说明），不尝试输出二进制内容。

## 数据流

终端检测和 DSL 预处理在遍历内容块之前统一完成，然后逐块渲染。渲染是纯数据转换，不执行 I/O：

1. ContentBlock[] + DslParseResult 输入（定义见 [common DslParseResult](../common/shared-types.md#dslparseresult--dslinstruction)）
2. 获取终端能力信息（经 platform 模块：ANSI 能力标记 + 可用宽度）→ 确定渲染模式（ANSI / 纯文本）
3. DSL 交互元素预处理：按钮 / 选择器生成纯文本提示行（如 "[Button: label (action: xxx)]"，不应用 ANSI 样式，两种渲染模式一致）并汇总为提示行列表；其他 DSL 忽略
4. 遍历 ContentBlock[] 逐块渲染，按块类型分派渲染策略（各策略见 §块类型渲染规则），块间空行分隔，各块输出超过终端可用宽度时逐行截断，被截断的行尾追加 "... (truncated)"
5. 提示行列表并入最终输出：作为独立段落置于全部渲染内容最前，与正文之间空一行；提示行超过终端可用宽度时同样截断并追加 "... (truncated)"
6. 全部渲染完成后返回单个 RenderedOutput（msg_type 恒 "text"，见 [common RenderedOutput §输出格式决策](../common/shared-types.md#renderedoutput) 的终端渠道例外）
7. TerminalPlugin 的 send 方法将 payload 写入 stdout

空输入约定：ContentBlock[] 为空且无 DSL 提示行时，返回空 payload 的 RenderedOutput，不产生输出内容（TerminalPlugin 跳过写入，不调用 send）；ContentBlock[] 为空但存在 DSL 提示行时，正常输出提示行段落（正文为空）。

> **流式路径**：流式模式不走本组件的批量渲染逻辑，由 IM Adapter 流式渲染组件驱动（见 §架构），完整路径见 [CLI Chat §数据流](chat.md)。

## 模块关系

- **上游**：TerminalPlugin（调用 TerminalRenderer 完成渲染）、platform（提供终端能力检测结果——ANSI 能力标记 + 可用宽度，渲染模式与截断判断的输入）
- **下游**：TerminalPlugin（消费 TerminalRenderer 产出的 RenderedOutput，通过 send 写入 stdout）——渲染是纯数据转换，除此之外不调用其他模块
- **与模块内其他子功能**：被 TerminalPlugin 持有和调用，作为 IMPlugin 渲染职责的 terminal 渠道实现。TerminalPlugin 在流式模式中取用流式渲染组件逐行产生增量输出
- **与 IM Adapter 的关系**：TerminalRenderer 是 IM Adapter 框架下 terminal 渠道的渲染实现，遵循 IMPlugin 约定——渲染返回 RenderedOutput，发送由插件完成。流式渲染使用 IM Adapter 模块的流式渲染组件作为共享渲染器
- **无关**：IM Adapter 各平台渲染实现（飞书、Discord 等）——渲染策略和目标格式不同，无共享逻辑
