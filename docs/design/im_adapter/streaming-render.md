# 流式渲染

## 概述

流式渲染是 IM Adapter 模块的通用渲染子功能，负责在 LLM 流式输出时逐条渲染 ContentBlock 增量（定义见 [common ContentBlock](../common/shared-types.md#contentblock)）。用户无需等待完整响应即可看到输出内容。该能力作为 IMPlugin trait 的默认方法提供——各平台插件通过组合持有流式渲染器实例，由 trait 默认方法委托调用，平台可按需覆盖方法实现差异化渲染逻辑。

## 架构

流式渲染是 IMPlugin 内部通用组件，由各平台插件持有。流式渲染在 Gateway 的增量阶段被调用——ContentBlock[]（经 VerbosityFilter 过滤 + DslParser 透传）到达后进入流式渲染。完整链处理（DslParser 解析 DSL 指令 + 出站日志）在流式渲染全部完成后由 Gateway 在收尾阶段调度执行。各平台通过 trait 默认方法委托调用流式渲染器，可按需覆盖实现差异化：

```
ContentBlock[] 增量（经 VerbosityFilter 过滤 + DslParser 透传）
  ↓
IMPlugin（内部流式渲染器）
  ├── 行缓冲 — 对 Text 块内容逐字/逐行积累
  ├── 类型路由 — 按块类型选择渲染路径
  └── 增量输出 — 完整行通过 IMPlugin 立即发送

流式渲染器接收增量阶段的 ContentBlock[]（经 VerbosityFilter 过滤、DslParser 透传），以增量方式渲染——Text 块内容逐缓冲行输出，Thinking/Tool 块等待完整内容后一次交付平台格式渲染器。交互式 UI 元素（按钮、选择器等）通过工具调用结果由 Gateway 直接处理，不属于流式渲染器职责范围。
```

**行缓冲规则**：

- 以句末标点（`。！？.!?\n`）为行边界，达到边界立即输出当前行
- 代码块内按换行符输出，不做句末标点等待
- 目标平台需要完整代码块才能正确渲染语法高亮时，以完整代码块为单位输出（代码块结束后一次性发送），不逐行输出代码块内容
- 缓冲区超过固定阈值（约 100 字符）时强制输出并清空缓冲区，防止长时间无响应
- 缓冲内容超过 200ms 未触发输出事件时，强制输出当前缓冲内容，避免长时间无响应
- 非 Text 块（Thinking、Tool 等）不参与流式行缓冲，累积完整内容后一次交付平台格式渲染器
- 代码/文本模式状态：检测 ``` 边界标记切换，用于决定行边界判定规则（代码模式按换行，文本模式按句末标点）

首行输出需在首个 Text 块到达后 200ms 内完成——若缓冲内容在 200ms 内未达输出条件，强制输出当前缓冲内容。

## 数据流

```
ContentBlock[] 增量到达 Gateway（来自 Session 的 LLM 流式响应）
  ↓
[Gateway: Processor Chain 增量阶段]
  → VerbosityFilter 过滤
  → DslParser 透传（零开销）
  → 跳过 OutboundRawLog
  ↓
Gateway 交付 ContentBlock[] 给 IMPlugin 流式渲染器
  ↓
流式渲染器按 ContentBlock 类型逐块处理：
  ├── Text 块 → 逐内容渲染
  │     → 追加文本到行缓冲区
  │     → 检测代码块边界标记（```），切换代码/文本模式
  │     → 检测句末标点或换行（文本模式），或换行（代码模式），或完整代码块结束（平台需要完整代码块时）
  │       ├── 完整行／代码块 → 渲染该行／代码块 → 立即输出
  │       └── 不完整 → 继续缓冲；若缓冲区超过阈值（约 100 字符）或 200ms 超时 → 强制输出
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
（每轮增量渲染完成后、逐片发送前，Gateway 可插入审计、频率限制等中间件）
  ↓
全部 ContentBlock[] 到齐
  ↓
[Gateway: Processor Chain 收尾阶段]
  → DslParser 完整解析 DSL 指令
  → OutboundRawLog 写入出站日志
  → 此阶段不产生新渲染输出
```

## 模块关系

- **上游**：Gateway（交付经 Processor Chain 处理后的 ContentBlock[] 给 IMPlugin，IMPlugin 内部触发流式渲染）
- **下游**：IMPlugin（接收增量渲染输出并通过 Adapter 发送到 IM 平台）
- **内部组件**：流式渲染器是 IMPlugin 的内部组件，由各平台插件通过 trait 默认方法持有和调用。平台可覆盖默认方法实现差异化渲染逻辑
- **与 Processor Chain 的关系**：Gateway 按交付模式协调链执行。流式出站走增量阶段——ContentBlock[] 经 VerbosityFilter 过滤、DslParser 透传后进入流式渲染。完整链处理（DslParser 解析 DSL 指令 + 出站日志）在流式渲染完成后由 Gateway 在收尾阶段调度。批量模式一次性执行完整链后渲染，详见 [Gateway 文档](../gateway/README.md)
- **所属**：IM Adapter 模块的通用子功能
