# 消息处理与渲染

## 概述

processor_chain 模块管理所有消息的入站和出站处理。入站方向将各 IM 平台的原始消息归一化为统一结构，出站方向解析 LLM 结构化输出中的 DSL 指令。渲染由 IM Adapter 模块完成，不属于 Processor 链。

核心职责：
- 入站：接收各平台原始消息 → 归一化为统一的中间格式 → 清洗后交付上层
- 出站：接收 LLM 的结构化输出 → 解析 DSL 指令 → 交付给 IM Adapter 模块渲染

## 架构

消息流经过三层：IM Adapter（平台适配）→ Processor 链（消息变换）→ IM Adapter（渲染与发送）。

```
=== 入站 ===
IM 平台 webhook（飞书 / Discord / Telegram）
  ↓
IM Adapter（入站）
  → 平台特定格式 → NormalizedMessage（统一中间结构）
  ↓
Processor 链（入站，按 priority 顺序执行，纯变换）
  ├── RawLogProcessor（priority 10）→ 原始消息写入日志
  ├── SessionRouter（priority 20）   → 计算 session_key，写入 metadata
  └── ContentNormalizer（priority 30）→ 清洗 + 标准化 markdown 格式
  ↓
Gateway
  → 从 metadata 取 session_key → SessionManager.resolve(key) 获得 session_id
  → 路由：/ 开头 → SlashDispatcher；普通消息 → Session → LLM → UnifiedResponse（ContentBlock[]）
  ↓
Processor 链（出站，按 priority 顺序执行）
  ├── DslParser（priority 10）  → 从 ContentBlock[] 中解析 DSL 指令
  └── RawLogProcessor（priority 20）→ 出站消息写入日志
  ↓
Renderer 层（传递格式 → 展示格式的唯一转换点，各渠道模块提供自身实现）
  ├── 飞书 Renderer + 飞书 Adapter（IM Adapter 模块）
  ├── CLI Renderer + CLI Adapter（CLI 模块）
  └── 其他平台 Renderer + Adapter
  ↓
IM Adapter（出站）→ 根据 (peer_id, thread_id) 发送到对应会话/话题，不参与格式转换
```

关键设计：
- **入站链纯变换**：只做内容计算和 metadata 填充，不管理 session 生命周期、不做路由决策。Session 创建和查找由 Gateway 调用 SessionManager 负责
- **出站方向 ContentBlock[] 作为传递格式**贯穿 Processor 链和 Renderer，不在链中途转为展示格式
- **Renderer 不在 Processor 链内**（详见 [IM Adapter 模块](../im_adapter/README.md)），渲染是终结操作，需要路由信息（msg_type），不适合链的"变换传递"语义
- **入站归一化**产 NormalizedMessage（platform, sender_id, peer_id, thread_id?, account_id?, content, timestamp），经 Processor 链清洗后交付 Gateway，Gateway 做路由决策后进入 Session

## 数据流

### 入站路径

```
IM webhook → IM Adapter 解析 → NormalizedMessage（platform, sender_id, peer_id, thread_id?, account_id?, content, timestamp）
  → Processor 链（RawLog → SessionRouter → ContentNormalizer）
    → ProcessedMessage（content + metadata { session_key }）
      → Gateway → SessionManager.resolve(session_key) → 路由到 Session / SlashDispatcher
```

NormalizedMessage 是平台无关的中间结构，承载消息的通用字段（发送者、内容、会话标识等）。IM Adapter 的入站部分负责将自己平台的格式转为此结构，content 为清洗后的消息文本。内容块（ContentBlock[]）概念仅在出站方向使用——LLM 输出 UnifiedResponse 时引入，经出站链处理后由 Renderer 渲染。

Processor 链在入站方向按 priority 升序执行，全链路操作 NormalizedMessage 的 content 字段（纯文本）。SessionRouter 计算 session_key 后写入 metadata。ContentNormalizer 对文本做清洗和标准化。链输出 ProcessedMessage（content + metadata），由 Gateway 消费。

### 出站路径

```
Session → LLM → UnifiedResponse（ContentBlock[]）
  → Processor 链（DslParser → RawLog）
    → ProcessedMessage（DSL 结果写入 metadata）
      → Renderer.render(content_blocks, dsl_result) → 平台格式 payload
        → IM Adapter 发送
```

DslParser 在出站链中遍历 ContentBlock[]，仅处理 Text 块中的 DSL 指令行，Thinking/ToolUse/ToolResult 块透传。解析结果写入 metadata 供 Renderer 使用。

Renderer 接收 ContentBlock[] 和 DSL 解析结果，按块类型选择渲染策略，一次性输出平台原生格式。

## 模块关系

- **上游**：Gateway（调度链执行）、Session（提供 LLM 输出的 ContentBlock[]）、IM Adapter（入站方向：产出 NormalizedMessage 供链消费）
- **下游**：[IM Adapter](../im_adapter/README.md) 模块（消费链输出并渲染为平台格式，发送渲染后的消息）
- **无关**：Slash Command 模块（斜杠指令不进入 LLM 对话流程，但经 Processor 链入站处理后由 Gateway 路由到 SlashDispatcher；SlashDispatcher 的输出经 Processor 链出站 + Renderer 回复）

### 子功能索引

| 文档 | 内容 |
|------|------|
| [入站链路](inbound-chain.md) | 入站 Processor 链、NormalizedMessage 统一中间格式、各处理器职责 |
| [出站链路](outbound-chain.md) | 出站 Processor 链、与 Renderer 的交接 |
| [DSL 解析器](dsl-parser.md) | 从 ContentBlock[] 中解析 DSL 指令 |

