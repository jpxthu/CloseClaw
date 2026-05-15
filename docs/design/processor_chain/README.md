# 消息处理与渲染

## 概述

processor_chain 模块负责消息的出站处理与平台渲染。

核心职责：
- 解析 LLM 输出中的 DSL 指令
- 将统一格式的消息渲染为各 IM 平台的原生格式
- 维护出站 Processor 链的执行

## 架构

出站链路分两层，由 Gateway 统一编排：

```
LLM 输出（UnifiedResponse）
    ↓
Session（写入 messages[]）
    ↓
Gateway
    ├── 调度 Processor 链出站方向
    │       └── DslParser  — 解析 DSL 指令，结果写入 metadata
    └── 提取 DSL 解析结果 → 传递给 Renderer
            ↓
        Rendering Layer
            └── Renderer  — 按平台渲染消息
                ├── FeishuRenderer  — 飞书 interactive card
                ├── CliRenderer     — 终端纯文本
                └── ...
            ↓
        IM Adapter（send_message）
```

Renderer 不经过 Processor 链，由 Gateway 从 Session 读取消息数组、提取 Processor 链的 DSL 解析结果后，一并传递给 Renderer 完成渲染。

## 数据流

```
LLM Provider 输出 UnifiedResponse
  → Session 写入 messages[]
    → Gateway 调度 Processor 链：DslParser 解析 DSL 指令
      → Gateway 提取 DSL 结果
      → Gateway 提取 DSL 解析结果
        → Renderer 读取 messages[] + DSL 结果，渲染为平台格式
          → IM Adapter 发送
```

## 模块关系

- **上游**：Session（提供消息数组）、LLM Provider（生成 UnifiedResponse）
- **下游**：IM Adapter（接收渲染后的平台消息并发送）
- **子文档**：
  - [出站链路](outbound-chain.md) — 完整出站流程与 Processor 链出站角色
  - [DSL 解析器](dsl-parser.md) — DSL 指令解析机制
  - [渲染层抽象](renderer.md) — Renderer 跨平台渲染框架
  - [飞书渲染器](feishu-renderer.md) — 飞书卡片生成规则
