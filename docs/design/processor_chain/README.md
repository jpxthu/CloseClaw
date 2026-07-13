# Processor Chain

## 概述

- 关联需求文档：[requirements/processor_chain.md](../requirements/processor_chain.md)
- 核心职责：管理入站消息的内容标准化/session_key 计算和出站消息的内容过滤、DSL 解析和日志记录。入站方向对 IM Adapter 归一化后的 NormalizedMessage 做内容清洗和 session_key 计算，出站方向按 priority 顺序执行 Verbosity 过滤、DSL 解析和出站日志。流式出站同样经 Processor 链——VerbosityFilter 和 DslParser 对流式增量文本零开销透传。

核心职责：
- 入站：接收 IM Adapter 产出的 NormalizedMessage → 内容清洗 → metadata 填充 → 交付上层。非文本消息（image/file/audio）经 ContentNormalizer 时跳过文本标准化，直接透传
- 出站：接收 LLM 的结构化输出 → Verbosity 过滤 → DSL 解析 → 出站日志 → 交付渲染

## 架构

消息流经过主要模块：IM Adapter（平台适配）→ Processor 链（消息变换）→ Gateway 渲染与发送。

```
=== 入站 ===
IM 平台 webhook（飞书 / Discord / Telegram）
  ↓
IM Adapter（入站）
  → 平台特定格式 → NormalizedMessage（统一中间结构）
  ↓
Processor 链（入站，按 priority 顺序执行，纯变换）
  - RawLogProcessor（priority 10）→ 原始消息写入日志。仅在 raw_log_dir 配置时注册
  - SessionRouter（priority 20）   → 计算 session_key，写入 metadata
  - ContentNormalizer（priority 30）→ 文本标准化（去控制字符、压缩空行、去尾空格）。非文本消息跳过标准化
  ↓
Gateway
  → 调用 SessionManager.resolve(session_key, platform, sender_id, peer_id, account_id)，SessionManager 内部提取稳定路由键做查找 → 获得 session_id
  → 路由：/ 开头 → SlashDispatcher；普通消息 → Session → LLM → UnifiedResponse（ContentBlock[]）
  ↓
Processor 链（出站，按 priority 顺序执行）
  - VerbosityFilter（priority 5）  → 按 Session Verbosity 等级过滤 ContentBlock[]
  - DslParser（priority 10）     → 从 ContentBlock[] 中解析 DSL 指令
  - OutboundRawLog（priority 20） → 写入出站日志
  ↓
IM Adapter（出站）— 含 Renderer + Adapter
  - Renderer 完成 ContentBlock[] → 平台原生格式的转换
  - Adapter 根据 (peer_id, thread_id) 发送到对应会话/话题
  - 各平台提供自身 Renderer + Adapter 实现（飞书、CLI 等）
```

关键设计：
- **入站链纯变换**：只做内容计算和 metadata 填充，不管理 session 生命周期、不做路由决策。Session 创建和查找由 Gateway 调用 SessionManager 负责
- **出站方向 ContentBlock[] 作为传递格式**（完整变体定义见 [common ContentBlock](../common/shared-types.md#contentblock)）贯穿 Processor 链和 Renderer，不在链中途转为展示格式
- **Verbosity 过滤在链内**：VerbosityFilter 是出站链第一个 Processor（priority 5），按 Session Verbosity 等级在 DSL 解析前过滤。保证流式和非流式出站统一经过滤
- **出站日志在链内**：OutboundRawLog（priority 20）在 DslParser 之后记录处理完毕的出站内容，仅在 raw_log_dir 配置时注册
- **Renderer 不在 Processor 链内**（详见 [IM Adapter 模块](../im_adapter/README.md)），渲染是终结操作，需要路由信息（msg_type），不适合链的"变换传递"语义
- **入站归一化**产 NormalizedMessage（完整字段定义见 [common 共享类型](../common/shared-types.md)），经 Processor 链清洗和 metadata 填充后交付 Gateway，Gateway 做路由决策后进入 Session

### 子功能索引

| 文档 | 内容 |
|------|------|
| [入站链路](inbound-chain.md) | 入站 Processor 链、NormalizedMessage 统一中间格式、各处理器职责 |
| [出站链路](outbound-chain.md) | 出站 Processor 链（VerbosityFilter → DslParser → OutboundRawLog）、与 Renderer 的交接 |
| [DSL 解析器](dsl-parser.md) | 从 ContentBlock[] 中解析 DSL 指令 |

## 数据流

### 入站路径

```
IM webhook → IM Adapter 解析 → NormalizedMessage（platform, sender_id, peer_id, thread_id?, account_id, content, message_type, media_refs, timestamp）
  → Processor 链（RawLogProcessor → SessionRouter → ContentNormalizer）
    → [ProcessedMessage](../common/shared-types.md#processedmessage)（content_blocks + metadata { session_key, message_type }）
      → Gateway → SessionManager.resolve(session_key, platform, sender_id, peer_id, account_id)，SessionManager 内部提取稳定路由键做 session 查找/创建 → 路由到 Session / SlashDispatcher
```

NormalizedMessage 定义见 [common 共享类型](../common/shared-types.md)。IM Adapter 的入站部分负责将自己平台的格式转为此结构。内容块（ContentBlock[]）概念主要用于出站方向——LLM 输出 UnifiedResponse 时引入，经出站链处理后由 Renderer 渲染。入站方向经 ContentNormalizer 标准化后的文本以 ContentBlock::Text 传入 ProcessedMessage（详见 [common ContentBlock](../common/shared-types.md#contentblock)）。

Processor 链在入站方向按 priority 升序执行。链中处理器可操作 NormalizedMessage 的 content 字段（如 ContentNormalizer 做文本标准化——去除控制字符和 ANSI 转义序列、压缩连续空行、去行尾空格），也可向 metadata 写入计算结果（如 SessionRouter 写入 session_key）。平台格式转换和清洗由 IM Adapter 在解析阶段完成。链输出 [ProcessedMessage](../common/shared-types.md#processedmessage)，由 Gateway 消费。

### 出站路径

```
Session → LLM → UnifiedResponse（ContentBlock[]）
  ↓
Processor 链（VerbosityFilter → DslParser → OutboundRawLog）
    ↓
[ProcessedMessage](../common/shared-types.md#processedmessage)（DSL 结果写入 metadata["dsl_result"]，处理后 ContentBlock[]）
    ↓
IM Adapter 渲染为平台格式 payload
    ↓
IM Adapter 发送
```

出站链按 VerbosityFilter(pri 5) → DslParser(pri 10) → OutboundRawLog(pri 20) 顺序执行。VerbosityFilter 逐块过滤，DslParser 遍历过滤后的 Text 块解析 DSL 指令，Thinking/ToolUse/ToolResult 块透传。OutboundRawLog 记录处理完毕的出站内容。

Renderer 接收 ContentBlock[] 和 DSL 解析结果，按块类型选择渲染策略，一次性输出平台原生格式。

## 模块关系

- **上游**：Gateway（调度链执行）、Session（LLM 对话产出的 ContentBlock[]，属出站数据流上游）、IM Adapter（入站方向：产出 NormalizedMessage 供链消费）
- **下游**：[IM Adapter](../im_adapter/README.md) 模块（消费链输出并渲染为平台格式，发送渲染后的消息）
- **无关**：入站与出站 Processor Chain 互为独立链路，互不干扰
- **间接相关**：Slash Command 模块（斜杠指令经入站链做 session_key 计算 + 内容清洗后由 Gateway 路由到 SlashDispatcher；SlashDispatcher 的输出经出站链 DSL 解析 + 日志后由 Renderer 回复。数据流双向穿越 Processor 链，但非直接调用依赖）

