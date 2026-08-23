# Terminal Renderer

## 概述

TerminalRenderer 是 terminal 渠道的出站渲染组件。它接收 ContentBlock[]（定义见 [common ContentBlock](../common/shared-types.md#contentblock)）和 DSL 解析结果，将结构化内容转换为 ANSI 格式的 RenderedOutput。渲染是纯数据转换，实际的 stdout 写入由 TerminalPlugin 的 send 方法完成——遵循 IM Adapter 框架「渲染与发送分离」的设计原则。

## 架构

TerminalRenderer 按 ContentBlock 类型分派渲染策略。流式渲染由 IM Adapter 模块的流式渲染组件驱动——TerminalRenderer 引用流式渲染器，TerminalPlugin 在流式模式中通过该渲染器逐行产生增量输出，不经过 TerminalRenderer 自身的批量渲染。

1. ContentBlock[] + DslParseResult 输入
2. 获取终端能力信息（platform 提供：ANSI 标记 + 宽度）→ 确定渲染模式（ANSI / 纯文本）
3. DSL 交互元素预处理：按钮/选择器 → 纯文本提示行（并入最终输出最前部，见数据流）
4. 遍历 ContentBlock[] 逐块渲染（策略见 §块类型渲染规则），逐块输出超过终端宽度时截断
5. 返回单个 RenderedOutput（msg_type 恒 "text"；ANSI 模式 payload 为 ANSI 文本，纯文本模式 payload 为剥离 ANSI 转义后的纯文本）

### 终端能力检测

终端能力检测由 [platform 模块](../platform/README.md)提供（检测规则与主流终端覆盖见 platform 文档），TerminalRenderer 在渲染时消费其结果：

- **ANSI 能力标记**：支持 → 启用 ANSI 模式；不支持 → 回退纯文本模式
- **终端可用宽度**：用于各块输出的截断判断

纯文本模式下，所有 ANSI 转义序列被移除，仅保留文本内容和边界标记；markdown 格式标记保留原样输出（与列表标记的处理一致——纯文本模式不做格式转换，只做 ANSI 剥离）。

### 块类型渲染规则

**Text 块 — 普通文本**

纯文本直接输出。包含 markdown 格式标记（标题、粗体、斜体、引用、链接、分割线）时，转为 ANSI 样式：标题用 bold，粗体用 bold，斜体用 italic，引用用 dim 前缀 `│ `，链接渲染为 `文本 (url)`，分割线渲染为 `───`。列表（`- `/`* ` 无序、`1. ` 有序）无对应 ANSI 样式，保留原始 markdown 标记原样输出。

**Text 块 — 代码块**

按语言标注注入 ANSI 颜色码（关键字、字符串、注释等），语言标注从 markdown 代码块标记中提取。不支持的语言回退无高亮纯文本输出，保留反引号边界。代码块前插入语言标注行和行号。代码块高亮策略详见 [IM Adapter 代码块渲染](../im_adapter/code-render.md)。

**Thinking 块**

折叠展示：ANSI dim 样式包裹，首行 `[Thinking]`，末行 `[end of thinking]`，内容缩进 2 空格。不支持 ANSI 时用 `[Thinking]` / `[end of thinking]` 边界包围。输出超过终端可用宽度时截断并追加 `... (truncated)`。

**ToolUse 块**

展示工具名称和参数。ANSI 模式下工具名用 bold + cyan，参数用 dim。参数以原始 JSON 字符串形式展示，格式为 `⚙ tool_name({"key":"value",...})`。参数不做 key=value 格式化解析。输出超过终端可用宽度时截断并追加 `... (truncated)`。

**ToolResult 块**

展示工具执行结果。输出截断——超过终端可用宽度时截断并追加 `... (truncated)`。ANSI 模式下用 dim 样式。

**不支持的内容块**

Image、Audio、File 等终端不支持的块类型，渲染为带文件名的占位符，不尝试输出二进制内容。

## 数据流

终端检测和 DSL 预处理在遍历内容块之前统一完成，然后逐块渲染。渲染是纯数据转换，不执行 I/O：

1. ContentBlock[] + DslParseResult 输入（定义见 [common DslParseResult](../common/shared-types.md#dslparseresult--dslinstruction)）
2. 获取终端能力信息（经 platform 模块：ANSI 能力标记 + 可用宽度）→ 确定渲染模式（ANSI / 纯文本）
3. DSL 交互元素预处理：按钮 / 选择器生成纯文本提示行（如 "[Button: label (action: xxx)]"）并汇总为提示行列表；其他 DSL 忽略
4. 遍历 ContentBlock[] 逐块渲染，按块类型分派渲染策略（各策略见 §块类型渲染规则），块间空行分隔，各块输出超过终端可用宽度时截断并追加 "... (truncated)"
5. 提示行列表并入最终输出：作为独立段落置于全部渲染内容最前，与正文之间空一行
6. 全部渲染完成后返回单个 RenderedOutput（msg_type 恒 "text"，见 [common RenderedOutput §输出格式决策](../common/shared-types.md#renderedoutput) 的终端渠道例外）
7. TerminalPlugin 的 send 方法将 payload 写入 stdout

空输入约定：ContentBlock[] 为空且无 DSL 提示行时，返回空 payload 的 RenderedOutput，不产生输出内容。

> **流式路径**：流式模式不走本组件的批量渲染逻辑，由 IM Adapter 流式渲染组件驱动（见 §架构），完整路径见 [CLI Chat §数据流](chat.md)。

## 模块关系

- **上游**：TerminalPlugin（调用 TerminalRenderer 完成渲染）
- **下游**：TerminalPlugin（消费 TerminalRenderer 产出的 RenderedOutput，通过 send 写入 stdout）——渲染是纯数据转换，除此之外不调用其他模块
- **与模块内其他子功能**：被 TerminalPlugin 持有和调用，作为 IMPlugin 渲染职责的 terminal 渠道实现。TerminalPlugin 在流式模式中取用流式渲染组件逐行产生增量输出
- **与 IM Adapter 的关系**：TerminalRenderer 是 IM Adapter 框架下 terminal 渠道的渲染实现，遵循 IMPlugin 约定——渲染返回 RenderedOutput，发送由插件完成。流式渲染使用 IM Adapter 模块的流式渲染组件作为共享渲染器
- **无关**：IM Adapter 各平台渲染实现（飞书、Discord 等）——渲染策略和目标格式不同，无共享逻辑
