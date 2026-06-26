# 流式渲染

## 概述

流式渲染是 IM Adapter 模块的通用渲染子功能，负责在 LLM 流式输出时逐条渲染 ContentBlock 增量。用户无需等待完整响应即可看到输出内容。该能力作为 IMPlugin trait 的默认方法提供，各平台插件自动继承，按需覆盖平台差异化渲染逻辑。

## 架构

流式渲染是 IMPlugin 内部通用组件，由各平台插件持有。Gateway 通过 IMPlugin trait 的流式默认方法驱动渲染，平台通过覆盖默认方法实现差异化：

```
LLM 流式事件（StreamEvent）
  ├── BlockStart — 新内容块开始
  ├── BlockDelta — 内容块增量数据（Text / Thinking / Tool）
  ├── BlockEnd   — 内容块结束
  ├── MessageEnd — 消息流结束
  └── Error      — 流错误（渲染器不处理，交由 Gateway 处理）
  ↓
流式渲染
  ├── 行缓冲 — 累积不完整语义单元
  ├── 类型路由 — 按块类型选择渲染路径
  └── 增量输出 — 完整行立即推送
  ↓
Gateway → IMPlugin 立即发送

流式渲染器仅处理内容块（Text、Thinking、Tool）的增量输出。交互式 UI 元素（按钮、选择器等）通过工具调用结果由 Gateway 直接处理，不属于流式渲染器职责范围，不经过流式文本渲染管线。
```

**行缓冲规则**：

- 以句末标点（`。！？.!?\n`）为行边界，达到边界立即输出当前行
- 代码块内按换行符输出，不做句末标点等待
- 缓冲区超过固定阈值（约 100 字符）时强制输出，防止长时间无响应
- 非 Text 块（Thinking、Tool 等）不参与流式行缓冲，在 BlockEnd 时一次性输出

**首行输出时机**：首次收到 BlockDelta 后，缓冲到达首个句末标点或换行符时输出。若长时间未遇到标点，缓冲区超过固定阈值时强制输出

## 数据流

```
LLM StreamEvent 序列到达 Gateway
  ↓
Gateway 调用 IMPlugin 流式方法，内部驱动流式渲染器
  ↓
流式渲染器按事件类型处理：
  ├── BlockStart → 标记块类型，初始化缓冲区
  ├── BlockDelta（Text）
  │     → 追加文本到行缓冲区
  │     → 检测代码块边界标记（```），切换代码/文本模式
  │     → 检测句末标点或换行（文本模式），或换行（代码模式）
  │       ├── 完整行 → 渲染该行 → 立即输出
  │       └── 不完整 → 继续缓冲；若缓冲区超过阈值（约 100 字符）→ 强制输出
  ├── BlockDelta（Thinking/Tool）
  │     → 单独缓冲，不在流式阶段输出
  ├── BlockEnd
  │     → 刷新当前块剩余缓冲内容
  │     → Thinking/Tool 块 → 以原始 ContentBlock 传递，交由下游平台 Renderer 完成最终格式渲染（如飞书的折叠推理区、工具操作卡片）
  │     → Text 块 → 输出完整块内容
  ├── MessageEnd
  │     → 刷新所有缓冲，输出剩余内容
  │     → 清空当前块状态和行缓冲上下文
  └── Error
        → 空操作，流错误交由 Gateway 处理
  ↓
增量输出 → Gateway → IMPlugin 发送
```

## 模块关系

- **上游**：Gateway（通过 IMPlugin trait 流式方法驱动渲染，逐事件调用）
- **下游**：IMPlugin（接收增量渲染输出并通过 Adapter 发送到 IM 平台）
- **内部组件**：流式渲染器是 IMPlugin 的内部组件，由各平台插件通过 trait 默认方法持有和调用。平台可覆盖默认方法实现差异化渲染逻辑
- **与 Processor Chain 的关系**：流式文本路径不经出站 Processor Chain（文本增量直接输出，无需 DslParser 参与；交互式 UI 元素由工具调用路径单独处理）。非流式出站路径由 Gateway 统一经 Processor Chain（DslParser → RawLog）处理后渲染，详见 [Gateway 文档](../../gateway/README.md)
- **所属**：IM Adapter 模块的通用子功能
