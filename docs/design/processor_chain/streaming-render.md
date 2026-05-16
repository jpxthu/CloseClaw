# 流式渲染

## 概述

流式渲染是渲染处理器的子功能，负责在 LLM 流式输出时逐条渲染 ContentBlock 增量，用户无需等待完整响应即可看到输出内容。

## 架构

流式渲染在渲染 Processor 中增加增量处理能力，基于 LLM 流式事件逐块消费：

```
LLM 流式事件（StreamEvent）
  ├── BlockStart — 新内容块开始
  ├── BlockDelta — 内容块增量文本
  ├── BlockEnd   — 内容块结束
  └── MessageEnd — 消息流结束
  ↓
流式渲染 Processor
  ├── 行缓冲 — 累积不完整语义单元
  ├── 类型路由 — 按块类型选择渲染路径
  └── 增量输出 — 完整行立即推送
  ↓
Gateway → IM Adapter 立即发送
```

**行缓冲规则**：

- 以句末标点（`。！？.!?\n`）为行边界，达到边界立即输出当前行
- 代码块内按换行符输出，不做句末标点等待
- 缓冲区超过固定阈值（约 100 字符）时强制输出，防止长时间无响应
- 非 Text 块（Thinking、Tool 等）不参与流式行缓冲，在 BlockEnd 时一次性输出

**首行输出时机**：

- 首次收到 BlockDelta 后，缓冲到达首个句末标点或换行符时输出
- 目标延迟：流式开始后 200ms 内输出首行

## 数据流

```
LLM StreamEvent 序列到达 Gateway
  ↓
Gateway 将事件转发给渲染 Processor
  ↓
渲染 Processor 按事件类型处理：
  ├── BlockStart → 标记块类型，初始化缓冲区
  ├── BlockDelta（Text）
  │     → 追加文本到行缓冲区
  │     → 检测句末标点或换行
  │       ├── 完整行 → 渲染该行 → 立即输出
  │       └── 不完整 → 继续缓冲
  ├── BlockDelta（Thinking/Tool）
  │     → 单独缓冲，不在流式阶段输出
  ├── BlockEnd
  │     → 刷新剩余缓冲内容
  │     → Thinking 块 → 渲染为折叠区并输出
  │     → Tool 块 → 渲染为工具卡片并输出
  └── MessageEnd
        → 刷新所有缓冲
        → 释放流式渲染上下文
  ↓
增量输出 → Gateway → IM Adapter 发送
```

## 模块关系

- **上游**：LLM Provider（产生 StreamEvent 序列）
- **下游**：IM Adapter（接收增量渲染输出并发送）
- **所属**：各平台渲染 Processor 的内部子功能
