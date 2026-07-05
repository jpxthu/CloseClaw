# ProcessedMessage

## 概述

ProcessedMessage 是 Processor Chain 的输出结构，Gateway 的消费入口。入站和出站方向共用同一结构，content_blocks 在不同方向携带不同复杂度的内容，metadata 携带方向相关的计算结果。

> **本文档定义的 ProcessedMessage 在 common crate 中实现。引用本模块的下游文档通过 [NormalizedMessage](inbound-message.md)、[ContentBlock](content-block.md) 等链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### ProcessedMessage

| 字段 | 类型 | 说明 |
|------|------|------|
| `content_blocks` | ContentBlock[] | 处理后的内容块数组。入站方向为单个 ContentBlock::Text（ContentNormalizer 标准化后的文本），出站方向为经 DslParser 处理后的 ContentBlock[]（Text 块已剥离 DSL 行，其余块透传） |
| `metadata` | map(string→string) | 方向相关的键值对。入站含 `session_key`（SessionRouter 计算的消息级标识）和 `message_type`（来自原始 NormalizedMessage，供 Gateway 做非文本路由判断），出站含 `dsl_result`（DslParser 产出的 DslParseResult，JSON 序列化） |

入站和出站不区分类型——同一个 ProcessedMessage 结构，内容形态和 metadata 字段按方向不同而不同。

## 数据流

**入站方向**：

```
NormalizedMessage → Processor Chain 入站（RawLog → SessionRouter → ContentNormalizer）
  ↓
ProcessedMessage {
  content_blocks: [ContentBlock::Text("标准化后文本")],
  metadata: { session_key: "{timestamp}-{hash}", message_type: "<原始 message_type>" }
}
  ↓
Gateway — 先检查 message_type：非 text（image/file/audio）构造错误回复经简化出站路径发送；text 消息从 content_blocks[0] 取 Text 内容做路由决策（/ 开头 → 斜杠指令；否则 → LLM 对话），从 metadata 取 session_key 传给 SessionManager
```

**出站方向**：

```
ContentBlock[]（LLM 产出 / SlashResult 变体）→ Processor Chain 出站（VerbosityFilter → DslParser → OutboundRawLog）
  ↓
ProcessedMessage {
  content_blocks: [去 DSL 后的 ContentBlock[]],
  metadata: { dsl_result: "<DslParseResult JSON>" }
}
  ↓
Gateway 出站日志 → IM Adapter 渲染（消费 content_blocks + metadata[dsl_result]）→ 发送
```

ProcessedMessage 的生命周期：Processor Chain 产出 → Gateway 消费后即完成使命，不进入 Session 持久化。

## 模块关系

- **生产者**：Processor Chain 入站（ContentNormalizer 包装标准化文本为 ContentBlock::Text + SessionRouter 写 session_key 到 metadata）、Processor Chain 出站（DslParser 处理 ContentBlock[] + 写 dsl_result 到 metadata）
- **消费者**：Gateway（入站：消费 content_blocks + metadata.session_key 做路由决策；出站：消费 content_blocks + metadata.dsl_result 做出站日志后传给 IM Adapter）、IM Adapter（消费 content_blocks + metadata.dsl_result 渲染为平台格式并发送）、CLI TerminalRenderer（同 IM Adapter，渲染为 ANSI 终端文本）
- **无关**：NormalizedMessage（入站方向的上游产物，经 Processor Chain 处理后产出 ProcessedMessage，两者是不同的两个结构）、Session（Gateway 通过 ProcessedMessage 中的 session_key 找到 Session，但 Session 不直接操作 ProcessedMessage）、LLM Provider（不接触 ProcessedMessage，只产出 ContentBlock[]）
