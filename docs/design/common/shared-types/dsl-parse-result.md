# DslParseResult

## 概述

DslParseResult 是 DslParser 解析 ContentBlock::Text 中 DSL 指令行的输出结果。存储在 [ProcessedMessage](processed-message.md) 的 metadata 中，供下游 Renderer 消费。DslInstruction 是单条 DSL 指令的结构化表示。

> **本文档定义的 DslParseResult、DslInstruction 在 common crate 中实现。引用本模块的下游文档通过 [ProcessedMessage](processed-message.md)、[ContentBlock](content-block.md) 等链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### DslInstruction

DslInstruction 是单条 DSL 指令的结构化表示。DSL 指令是消息中的交互元素（按钮、选择器等），每条为一行，格式为 `::type[key1:value1;key2:value2;...]`。例如 `::button[label:确认;action:confirm;value:1]` 和 `::selector[label:选颜色;options:红,蓝;action:pick]`。

| 字段 | 类型 | 说明 |
|------|------|------|
| `instruction_type` | string | 指令类型。已知类型：`button`（按钮）、`selector`（选择器） |
| `params` | map(string→string) | 指令参数键值对，从 DSL 行中解析。例如 `::button[label:确认;action:confirm;value:1]` 解析为 `{label: "确认", action: "confirm", value: "1"}` |

### DslParseResult

DslParseResult 是 DslParser 的完整输出结构。

| 字段 | 类型 | 说明 |
|------|------|------|
| `instructions` | list(DslInstruction) | 解析出的 DSL 指令列表，按原文出现顺序排列。无 DSL 指令时为空列表 |

DslParseResult 与经 DslParser 剥离 DSL 行后的 ContentBlock[] 一同传递——ContentBlock[] 承载去 DSL 后的纯文本和其他内容块，DslParseResult 承载从 ContentBlock[] 中提取的结构化指令。两者通过 [ProcessedMessage](processed-message.md) 打包交付 Renderer。

## 数据流

DslParseResult 的流动嵌入在 ContentBlock[] 的出站路径中：

```
ContentBlock[]（来自 LLM UnifiedResponse / SlashResult）
  ↓
[Processor Chain 出站: VerbosityFilter] — 按 Session Verbosity 等级逐块过滤
  ↓
DslParser 遍历 Text 块，逐行扫描 DSL
  ├── 匹配 DSL 行 → 解析为 DslInstruction → 加入 instructions 列表 → 从 Text 块中移除该行
  └── 非 DSL 行 → 保留在 Text 块中
  ↓
DslParseResult { instructions } + 更新后的 ContentBlock[]
  ↓
[Processor Chain: OutboundRawLog] — 出站日志记录
  ↓
打包为 [ProcessedMessage](processed-message.md)
  ↓
Renderer 消费 DslParseResult：
  ├── button / selector → 渲染为平台交互元素（IM 平台卡片 button 组件、终端纯文本提示行）
  └── 其他指令类型 → Renderer 按平台能力处理或忽略
```

DslParseResult 的生命周期始于 DslParser 解析、终于 Renderer 渲染。中间经 OutboundRawLog（Processor Chain 出站日志）和 [ProcessedMessage](processed-message.md) 传递。DslParseResult 本身不被 Verbosity 过滤影响——DslParser 仅处理已通过过滤的 ContentBlock[]，因此 DslParseResult 中只包含可见块中的 DSL 指令。

## 模块关系

- **DslParseResult 生产者**：Processor Chain 出站（DslParser 解析 ContentBlock::Text 中的 DSL 指令行，产出 DslParseResult）
- **DslParseResult 消费者**：IM Adapter 各平台 Renderer（读取 DslParseResult 中的 DslInstruction 列表，渲染为平台交互元素）、CLI TerminalRenderer（将 button/selector 转为纯文本提示行）
- **DslInstruction 生产者**：Processor Chain 出站（DslParser 逐行解析 DSL 指令，每条产出一个 DslInstruction）
- **DslInstruction 消费者**：IM Adapter 各平台 Renderer（按 instruction_type 选择渲染策略）
- **无关**：Processor Chain 入站（DSL 解析仅在出站方向执行）、IM Adapter 入站链（入站方向不涉及 DSL）、LLM Provider（LLM 不感知 DSL）、Session（Session 不操作 DslParseResult）
