# 消息处理与渲染

## 概述

processor_chain 模块负责将 LLM 的结构化输出处理为各 IM 平台的原生消息格式，并统一调度整个出站处理链路。

核心职责：
- 解析 LLM 输出中的 DSL 指令
- 将结构化内容块渲染为平台原生格式
- 维护出站 Processor 链的顺序执行

## 架构

出站链路是单层 Processor 链，渲染器作为链上的 Processor 存在，不独立分层：

```
LLM 输出 ContentBlock[]（结构化内容块数组）
  ↓
Gateway 收集 ContentBlock[]，构造 ProcessedMessage
  ↓
Processor 链（出站方向，按 priority 顺序执行）
  ├── DslParser（priority 10）
  │     → 扫描 Text 块中的 DSL 指令
  │     → 解析为结构化指令，写入 metadata
  │     → 从 Text 块中移除 DSL 行
  │
  └── <Platform>Renderer（priority 20）
        → 输入：ContentBlock[] + DSL 解析结果
        → 按块类型选择渲染策略
        → 生成平台原生格式 payload
        → 输出：ProcessedMessage（content 为平台 JSON）
  ↓
Gateway 提取平台 payload → IM Adapter 发送
```

每个 IM 平台提供一个渲染 Processor，均实现统一的 Processor 接口。Gateway 根据目标平台选择对应的渲染 Processor 注册到链中。

## 数据流

```
LLM Provider 输出 UnifiedResponse（含 ContentBlock[]）
  → Session 写入 messages[]
    → Gateway 从 Session 读取 ContentBlock[]
      → 构造 ProcessedMessage，启动 Processor 链
        → DslParser 解析 DSL 指令
          → <Platform>Renderer 将 ContentBlock[] 渲染为平台格式
            → Gateway 提取平台 payload
              → IM Adapter 发送
```

关键分支：
- DSL 指令仅从 Text 块中识别，Thinking 和 Tool 块不参与 DSL 解析
- 平台选择在 Gateway 层完成，通过注册不同的渲染 Processor 实现
- 无目标平台或平台不支持时，回退到纯文本输出

## 模块关系

- **上游**：Session（提供 ContentBlock[] 消息数据）、LLM Provider（生成 UnifiedResponse）
- **下游**：IM Adapter（接收渲染后的平台消息并发送）
- **子文档**（按出站链执行顺序排列，新增文档依此规则）：
  - [出站链路](outbound-chain.md) — 完整出站流程与 Processor 链调度
  - [DSL 解析器](dsl-parser.md) — DSL 指令解析机制
  - [渲染处理器](renderer.md) — ContentBlock 到平台消息的渲染框架
  - [飞书渲染](renderer-feishu.md) — 飞书平台渲染规则
  - [代码块渲染](code-render.md) — 代码块语法高亮渲染
  - [流式渲染](streaming-render.md) — 流式增量输出渲染
