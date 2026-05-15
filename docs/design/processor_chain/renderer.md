# 渲染层抽象

## 概述

Rendering Layer 是消息出站的最后一环，负责将统一格式的 Session 消息渲染为各 IM 平台的原生格式。它定义跨平台的 Renderer 抽象，不同平台各提供一个渲染实现。

## 架构

渲染层由三部分组成：

**Renderer 抽象**：平台渲染器的统一接口。每个 IM 平台实现一个 Renderer，声明自己的平台标识。

```
Renderer
  ├── FeishuRenderer   — 飞书 interactive card
  ├── CliRenderer      — 终端纯文本
  └── ...              — 其他平台扩展
```

**渲染输入**：Renderer 从 Session 读取消息数组，每条消息包含：
- 消息角色（user / assistant / system / tool）
- 内容块数组（文本、推理、工具调用等类型）
- 时间戳

**渲染输出**：

```
RenderedOutput {
  msg_type: "text" | "interactive" | ...
  payload: 平台特定的 JSON
}
```

## 数据流

```
Session 消息数组 + DslParseResult（从 Processor 链 metadata 提取）
  ↓
Gateway 根据 target platform 选择 Renderer 实现
  ↓
Renderer.render(messages, dsl_result)
  → 遍历每条消息的内容块
  → 按块类型选择渲染策略：
      - 文本块 → 文本或格式内容
      - 推理块 → 推理内容（可配置隐藏或特殊格式）
      - 工具调用块 → 工具调用描述信息
  → DSL 指令 → 渲染为平台交互元素
  ↓
RenderedOutput { msg_type, payload }
  ↓
IM Adapter 发送
```

关键判断点：
- 消息含多个不同类型的内容块 → 选择能承载最丰富内容的输出格式
- 纯文本且无 DSL → text 消息
- 含格式标记、多内容块、或 DSL → 富格式消息

## 模块关系

- **上游**：Session（提供消息数据）、Processor 链（提供 DSL 解析结果 metadata）
- **下游**：IM Adapter（发送渲染后的消息）
- **平台实现**：
  - [飞书渲染器](renderer-feishu.md) — 飞书平台渲染规则
