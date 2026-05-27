# Terminal Renderer

## 概述

TerminalRenderer 是 terminal 渠道的出站渲染组件。它接收 ContentBlock[] 和 DSL 解析结果，将结构化内容转换为 ANSI 格式文本，输出到 stdout。

## 架构

TerminalRenderer 按 ContentBlock 类型分派渲染策略。不支持 ANSI 的终端自动回退纯文本。

```
ContentBlock[] + DslParseResult
  ↓
TerminalRenderer
  ├── 终端检测：检查 TERM 环境变量 / ANSI 支持
  │     ├── 支持 ANSI → 启用颜色和格式
  │     └── 不支持 → 全部回退纯文本
  ├── 遍历 ContentBlock[]
  │     ├── Text        → ANSI 格式化文本
  │     ├── Thinking    → 折叠块（ANSI dim 样式）
  │     ├── ToolUse     → 工具调用展示
  │     ├── ToolResult  → 工具结果展示
  │     ├── Image       → 占位符 "[image: name]"
  │     ├── Audio       → 占位符 "[audio: name]"
  │     └── File        → 占位符 "[file: name]"
  └── DSL 交互元素
        ├── 按钮 / 选择器 → 纯文本提示（终端无交互元素）
        └── 其他 DSL → 忽略
  ↓
stdout
```

### 块类型渲染规则

**Text 块**

纯文本直接输出。包含 markdown 格式标记（标题、粗体、斜体、列表、引用、链接、分割线）时，转为 ANSI 样式：标题用 bold，粗体用 bold，斜体用 italic，引用用 dim 前缀 `│ `，链接渲染为 `文本 (url)`，分割线渲染为 `───`。

**代码块**

按语言标注注入 ANSI 颜色码（关键字、字符串、注释等），语言标注从 markdown 代码块标记中提取。不支持的语言回退无高亮纯文本输出，保留反引号边界。代码块前插入语言标注行和行号。

**Thinking 块**

折叠展示：ANSI dim 样式包裹，首行 `[Thinking]`，末行 `[end of thinking]`，内容缩进 2 空格。不支持 ANSI 时用 `[Thinking]` / `[end of thinking]` 边界包围。

**ToolUse 块**

展示工具名称和参数摘要。ANSI 模式下工具名用 bold + cyan，参数用 dim。格式：`⚙ tool_name(arg1=val1, arg2=val2)`

**ToolResult 块**

展示工具执行结果。输出截断——超过终端的可用宽度时截断并追加 `... (truncated)`。ANSI 模式下用 dim 样式。

**不支持的内容块**

Image、Audio、File 等终端不支持的块类型，渲染为带文件名的占位符，不尝试输出二进制内容。

### 终端能力检测

渲染前检测终端 ANSI 支持：
- `TERM` 环境变量含 `xterm`、`screen`、`ansi`、`vt100` 或 `color` → 启用 ANSI
- Windows 下检测 Windows Terminal 环境 → 启用 ANSI
- 其余 → 回退纯文本模式

纯文本模式下，所有 ANSI 转义序列被移除，仅保留文本内容和边界标记。

## 数据流

```
Processor Chain 出站产 ProcessedMessage，含内容块和 DSL 解析元数据
  ↓
Gateway 选择 terminal 渠道的 TerminalRenderer
  ↓
终端能力检测 → 确定渲染模式（ANSI / 纯文本）
  ↓
遍历 content_blocks，按类型渲染
  ├── 每种块类型独立渲染，块间空行分隔
  └── 流式输出：每完成一个块即写入 stdout，不等待全部完成
  ↓
stdout
```

## 模块关系

- **上游**：Gateway（传递 ContentBlock[] 和 DSL 解析结果）
- **下游**：操作系统 stdout（渲染后的 ANSI 文本）
- **与模块内其他子功能**：由 CLI Chat 的 TerminalPlugin 持有和调用，IMPlugin trait 的渲染职责由 TerminalRenderer 实现
- **无关**：IM Adapter 的飞书 Renderer 等平台渲染器（渲染策略不同、目标输出不同，无共享逻辑）
