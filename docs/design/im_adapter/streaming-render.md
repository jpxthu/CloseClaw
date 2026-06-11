# 流式渲染

## 概述

流式渲染是 IM Adapter 模块的通用渲染子功能，负责在 LLM 流式输出时逐条渲染 ContentBlock 增量。用户无需等待完整响应即可看到输出内容。该能力作为 IMPlugin trait 的默认方法提供，各平台插件自动继承，按需覆盖平台差异化渲染逻辑。

## 架构

流式渲染在各平台 Renderer 中增加增量处理能力，基于 LLM 流式事件逐块消费。平台通过覆盖 IMPlugin trait 的流式默认方法实现差异化：

```
LLM 流式事件（StreamEvent）
  ├── BlockStart — 新内容块开始
  ├── BlockDelta — 内容块增量文本
  ├── BlockEnd   — 内容块结束
  └── MessageEnd — 消息流结束
  ↓
流式渲染
  ├── 行缓冲 — 累积不完整语义单元
  ├── 类型路由 — 按块类型选择渲染路径
  └── 增量输出 — 完整行立即推送
  ↓
Gateway → IMPlugin 立即发送
```

**行缓冲规则**：

- 以句末标点（`。！？.!?\n`）为行边界，达到边界立即输出当前行
- 代码块内按换行符输出，不做句末标点等待
- 缓冲区超过固定阈值（约 100 字符）时强制输出，防止长时间无响应
- 非 Text 块（Thinking、Tool 等）不参与流式行缓冲，在 BlockEnd 时一次性输出

**首行输出时机**：首次收到 BlockDelta 后，缓冲到达首个句末标点或换行符时输出，目标延迟 200ms 内

## 数据流

```
LLM StreamEvent 序列到达 Gateway
  ↓
Gateway 将事件转发给 Renderer
  ↓
Renderer 按事件类型处理：
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
  │     → Thinking/Tool 块 → 以原始 ContentBlock 传递，交由下游平台 Renderer 完成最终格式渲染（如飞书的折叠推理区、工具操作卡片）
  │     → Text 块 → 输出完整块内容
  └── MessageEnd
        → 刷新所有缓冲
        → 释放流式渲染上下文
  ↓
增量输出 → Gateway → IMPlugin 发送
```

## 模块关系

- **上游**：IMPlugin trait 默认方法（作为通用渲染能力供各平台继承）
- **下游**：IMPlugin（接收增量渲染输出并发送）
- **所属**：IM Adapter 模块的通用子功能
