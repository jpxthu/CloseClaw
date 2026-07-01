# 流式渲染

## 概述

流式渲染是 IM Adapter 模块的通用渲染子功能，负责在 LLM 流式输出时逐条渲染 ContentBlock 增量（定义见 [common ContentBlock](../common/shared-types.md#contentblock)）。用户无需等待完整响应即可看到输出内容。该能力作为 IMPlugin trait 的默认方法提供——各平台插件通过组合持有流式渲染器实例，由 trait 默认方法委托调用，平台可按需覆盖方法实现差异化渲染逻辑。

## 架构

流式渲染是 IMPlugin 内部通用组件，由各平台插件持有。ContentBlock[]（经 Gateway Verbosity 过滤 + Processor Chain DslParser 处理）后进入流式渲染。各平台通过 trait 默认方法委托调用流式渲染器，可按需覆盖实现差异化：

```
ContentBlock[]（经 Gateway Verbosity 过滤 + Processor Chain DslParser + Gateway 出站日志）
  ↓
IMPlugin（内部流式渲染器）
  ├── 行缓冲 — 对 Text 块内容逐字/逐行积累
  ├── 类型路由 — 按块类型选择渲染路径
  └── 增量输出 — 完整行通过 IMPlugin 立即发送

流式渲染器接收 ContentBlock[]（已完成 DSL 解析和 Verbosity 过滤），以增量方式渲染——Text 块内容逐缓冲行输出，Thinking/Tool 块等待完整内容后一次交付平台格式渲染器。交互式 UI 元素（按钮、选择器等）通过工具调用结果由 Gateway 直接处理，不属于流式渲染器职责范围。
```

**行缓冲规则**：

- 以句末标点（`。！？.!?\n`）为行边界，达到边界立即输出当前行
- 代码块内按换行符输出，不做句末标点等待
- 缓冲区超过固定阈值（约 100 字符）时强制输出并清空缓冲区，防止长时间无响应
- 非 Text 块（Thinking、Tool 等）不参与流式行缓冲，累积完整内容后一次交付平台格式渲染器

**首行输出时机**：收到 ContentBlock[] 后开始渲染，首个 Text 块的缓冲到达句末标点或换行符时输出首行。若长时间未遇到标点，缓冲区超过固定阈值时强制输出

## 数据流

```
ContentBlock[]（经 Processor Chain 处理）到达 Gateway
  ↓
[Gateway: 出站日志] → 记录出站消息到日志
  ↓
Gateway 交付 ContentBlock[] 给 IMPlugin 流式渲染器
  ↓
流式渲染器按 ContentBlock 类型逐块处理：
  ├── Text 块 → 逐内容渲染
  │     → 追加文本到行缓冲区
  │     → 检测代码块边界标记（```），切换代码/文本模式
  │     → 检测句末标点或换行（文本模式），或换行（代码模式）
  │       ├── 完整行 → 渲染该行 → 立即输出
  │       └── 不完整 → 继续缓冲；若缓冲区超过阈值（约 100 字符）→ 强制输出
  ├── Thinking/Tool 块
  │     → 累积完整内容，待全块就绪后一次交付平台格式渲染器（如飞书的折叠推理区、工具操作卡片）
  ├── Image/Audio/File 块
  │     → 不参与流式渲染，交由平台格式渲染器处理
  └── 流错误
        → 空操作，不产生增量输出，流错误直接交由 Gateway 处理
  ↓
全部块处理完成 → 刷新所有缓冲 → 输出剩余内容 → 清空块状态和行缓冲上下文
  ↓
增量输出通过 IMPlugin 发送到 IM 平台
```

## 模块关系

- **上游**：Gateway（交付经 Processor Chain 处理后的 ContentBlock[] 给 IMPlugin，IMPlugin 内部触发流式渲染）
- **下游**：IMPlugin（接收增量渲染输出并通过 Adapter 发送到 IM 平台）
- **内部组件**：流式渲染器是 IMPlugin 的内部组件，由各平台插件通过 trait 默认方法持有和调用。平台可覆盖默认方法实现差异化渲染逻辑
- **与 Processor Chain 的关系**：流式出站与批量出站走同一条预处理管线——ContentBlock[] 先经 Gateway Verbosity 过滤，再经 Processor Chain DslParser 解析（流式文本零开销透传），然后进入流式渲染。非流式路径同样经此管线处理后渲染，详见 [Gateway 文档](../gateway/README.md)
- **所属**：IM Adapter 模块的通用子功能
