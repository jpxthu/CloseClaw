# Terminal Renderer

## 概述

TerminalRenderer 是 terminal 渠道的出站渲染组件。它接收 ContentBlock[] 和 DSL 解析结果，将结构化内容转换为 ANSI 格式的 RenderedOutput。渲染是纯数据转换，实际的 stdout 写入由 TerminalPlugin 的 send 方法完成——遵循 IM Adapter 框架「渲染与发送分离」的设计原则。

## 架构

TerminalRenderer 按 ContentBlock 类型分派渲染策略。流式渲染由 IM Adapter 模块的 DefaultStreamingRenderer 驱动——TerminalRenderer 持有 DefaultStreamingRenderer 实例，TerminalPlugin 在流式路径中直接取用该实例逐行产生增量输出，不经过 TerminalRenderer 自身的批量渲染。

```
ContentBlock[] + DslParseResult
  ↓
TerminalRenderer
  ├── 终端检测：检查 TERM 环境变量 / ANSI 支持，同时获取终端可用宽度
  │     ├── 支持 ANSI → 启用颜色和格式
  │     └── 不支持 → 全部回退纯文本
  ├── DSL 交互元素预处理
  │     ├── 按钮 / 选择器 → 纯文本提示行（终端无交互元素）
  │     └── 其他 DSL → 忽略
  ├── 遍历 ContentBlock[]
  │     ├── Text
  │     │     ├── 普通文本 → ANSI 格式化文本
  │     │     └── 代码块 → 注入 ANSI 语法高亮 + 行号
  │     ├── Thinking    → 折叠块（ANSI dim 样式）
  │     ├── ToolUse     → 工具调用展示（原始 JSON 参数）
  │     ├── ToolResult  → 工具结果展示
  │     ├── Image       → 占位符 "[image: name]"
  │     ├── Audio       → 占位符 "[audio: name]"
  │     └── File        → 占位符 "[file: name]"
  └── 输出截断：各块输出超过终端可用宽度时截断并追加 "... (truncated)"
  ↓
RenderedOutput { msg_type: "text", payload: ANSI 文本 }
```

### 终端能力检测

渲染前检测终端 ANSI 支持和尺寸：

- **ANSI 检测**：`TERM` 环境变量含 `xterm`、`screen`、`ansi`、`vt100` 或 `color` → 启用 ANSI。Windows 下检测 Windows Terminal 环境 → 启用 ANSI。其余 → 回退纯文本模式
- **终端宽度获取**：通过操作系统终端尺寸接口获取当前可用列数，用于各块的输出截断判断。宽度获取失败时回退到默认值（约 80 列）

上述检测覆盖主流终端：Ubuntu bash（通常 TERM=xterm-256color）、macOS Terminal（xterm-256color）、WSL2（xterm-256color）均默认启用 ANSI。

纯文本模式下，所有 ANSI 转义序列被移除，仅保留文本内容和边界标记。

### 块类型渲染规则

**Text 块 — 普通文本**

纯文本直接输出。包含 markdown 格式标记（标题、粗体、斜体、列表、引用、链接、分割线）时，转为 ANSI 样式：标题用 bold，粗体用 bold，斜体用 italic，引用用 dim 前缀 `│ `，链接渲染为 `文本 (url)`，分割线渲染为 `───`。

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

```
ContentBlock[] + DslParseResult
  ↓
终端能力检测 → 确定渲染模式（ANSI / 纯文本）+ 获取终端可用宽度
  ↓
DSL 交互元素预处理：
  ├── 按钮 / 选择器 → 生成纯文本提示行（如 "[Button: label (action: xxx)]"）
  └── 其他 DSL → 忽略
  ↓
遍历 content_blocks，按类型渲染
  ├── Text（普通文本）→ ANSI 格式化，markdown 标记转 ANSI 样式
  ├── Text（代码块）→ 注入语言标注行 + 行号 + 语法高亮 ANSI 颜色码
  ├── Thinking → 折叠块，dim 样式包裹，首尾边界标记
  ├── ToolUse → 工具名 + 原始 JSON 参数
  ├── ToolResult → 工具结果，按终端宽度截断
  ├── Image / Audio / File → 占位符
  ├── 每种块类型独立渲染，块间空行分隔
  └── 各块输出超过终端可用宽度时截断并追加 "... (truncated)"
  ↓
全部渲染完成后返回单个 RenderedOutput
  ↓
RenderedOutput { msg_type: "text", payload: ANSI 文本 }
  ↓
TerminalPlugin 的 send 方法将 payload 写入 stdout
```

> **流式路径**：不走上述 TerminalRenderer 的批量渲染逻辑。TerminalPlugin 在流式路径中直接取用 TerminalRenderer 持有的 DefaultStreamingRenderer 实例，Gateway 通过 IMPlugin trait 流式默认方法驱动，逐行产生增量 RenderedOutput 后立即写入 stdout。流式机制详见 [IM Adapter 流式渲染](../im_adapter/streaming-render.md)。

## 模块关系

- **上游**：TerminalPlugin（调用 TerminalRenderer 完成渲染，消费产出的 RenderedOutput 并通过 send 写入 stdout）
- **下游**：无——渲染是纯数据转换，不调用其他模块
- **与模块内其他子功能**：被 TerminalPlugin 持有和调用，作为 IMPlugin 渲染职责的 terminal 渠道实现。TerminalPlugin 在流式路径中直接取用 TerminalRenderer 持有的 DefaultStreamingRenderer 实例
- **与 IM Adapter 的关系**：TerminalRenderer 是 IM Adapter 框架下 terminal 渠道的渲染实现，遵循 IMPlugin 约定——渲染返回 RenderedOutput，发送由插件完成。流式渲染使用 IM Adapter 模块的 DefaultStreamingRenderer 作为共享组件
- **无关**：IM Adapter 各平台渲染实现（飞书、Discord 等）——渲染策略和目标格式不同，无共享逻辑
