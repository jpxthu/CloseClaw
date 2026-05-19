# DSL 解析器

## 概述

DslParser 在出站方向从 LLM 结构化输出（ContentBlock[]）中识别和解析 DSL 指令。DSL 指令定义消息中的交互元素（按钮、选择器等），格式形如 `::button[label:确认;action:confirm;value:1]`，每行一条。

## 架构

DslParser 在出站链中以最高优先级首先执行：

```
输入：ContentBlock[]（LLM 结构化输出，仅处理 Text 块）
  ↓
遍历 ContentBlock[]，定位 Text 块
  ↓
逐行扫描 Text 块内容，匹配 DSL 指令模式
  ↓
解析 DSL 语法，生成结构化指令（DslInstruction）
  ↓
从 Text 块中移除 DSL 指令行
  ↓
解析结果写入 ProcessedMessage.metadata["dsl_result"]
  ↓
输出：ProcessedMessage（ContentBlock[] 中 Text 块已去 DSL 行，metadata 含解析结果）
```

DslParser 只处理 ContentBlock::Text，Thinking、ToolUse、ToolResult 块直接透传到链的下一步。

## 数据流

```
ContentBlock[]（来自 LLM UnifiedResponse）
  → Processor 链按 priority 调度 DslParser
    → 遍历 ContentBlock[]：
        ├── Text 块 → 逐行扫描
        │     ├── 匹配 DSL 语法 → 解析为结构化指令 → 从 Text 中移除该行
        │     └── 非 DSL 行 → 保留在 Text 块中
        ├── Thinking 块 → 透传
        ├── ToolUse 块 → 透传
        └── ToolResult 块 → 透传
    → 输出：
        ├── content_blocks：更新后的 ContentBlock[]（Text 块已去 DSL 行）
        └── metadata["dsl_result"]：DslParseResult（含解析出的指令和去 DSL 后的纯文本）
  ↓
Renderer 消费 metadata 中的 DSL 指令，渲染平台交互元素
```

文本内容以 ContentBlock::Text 承载，DslParser 遍历所有 Text 块并逐行扫描。DSL 指令行从 Text 块中剥离后不影响其他内容块，非 Text 块不受影响直接透传。

## 模块关系

- **上游**：Processor 链框架（按 priority 调度执行）
- **下游**：Renderer 层（从 metadata 读取 DSL 解析结果，渲染平台交互元素）
- **无关**：入站 Processor 链（DSL 解析只在出站方向执行）
