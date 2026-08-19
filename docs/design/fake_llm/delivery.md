# 投递层

## 概述

投递层执行场景决策的「怎么发」：把响应按协议形态投递给被测 CloseClaw 二进制——完整 JSON 或 SSE 流式事件序列，以及在投递过程中注入错误（HTTP 错误、流中途中断）与延迟（首 token、逐段、整体）。

## 架构

### 流式分帧

流式场景下，响应内容按场景声明的分段粒度切分为事件序列。两种协议的流式事件序列均被完整模拟：

- **OpenAI**：SSE chunk 序列——`delta.content` / `delta.reasoning_content` / `delta.tool_calls` 增量帧，`finish_reason` 结束帧（`stop` / `tool_calls`），用量随最终 chunk 携带（与真实协议一致：仅当请求带 `stream_options.include_usage` 时返回用量，未带则不携带），末尾 `[DONE]`
- **Anthropic**：SSE 事件序列——`message_start` → `content_block_start` → 若干 `content_block_delta`（`text_delta` / `thinking_delta` / `signature_delta` / `input_json_delta`）→ `content_block_stop` → `message_delta`（含用量）→ `message_stop`，中间可插 `ping`

事件序列的映射关系与典型顺序以 [llm/protocol-mapping](../llm/protocol-mapping.md) 的统一流式事件表为权威——Fake LLM Server 发出的序列即该表「协议来源」列的完整实例化。分段粒度（每段多少内容、几个 token 一段、工具调用 JSON 参数的切片粒度）由场景声明，覆盖从逐字符到一次性全量的谱系，用于压测 CloseClaw 流式解析状态机的边界。

### 延迟注入

三类延迟，均由场景声明、可在多轮中逐轮变化：

| 类型 | 效果 | 验证目标 |
|------|------|---------|
| 首 token 延迟 | 收到请求后挂起一段时间再发首帧 | 请求超时、模型发现超时外的对话超时路径 |
| 逐段延迟 | 流式事件之间的间隔 | 流式读取的超时与取消处理 |
| 整体延迟 | 非流式响应返回前挂起 | 非流式调用的超时与回退 |

延迟注入只改变时间间隔，不改变事件序列与内容——确定性原则在投递层的体现。延迟值由场景显式给出固定值，不使用随机抖动。

### 错误注入

错误分两类：

- **HTTP 错误**：按协议标准返回错误状态码与错误体（认证失败 401、限流 429、服务端错误 5xx），错误体结构以 [fixtures](fixtures.md) 的错误 fixture 为基线。限流场景可声明 `Retry-After` 头，触发 CloseClaw 的退避重试
- **流中途中断**：流式输出若干事件后直接断开连接（不发结束事件、不发错误），验证 CloseClaw 对不完整流的处理——已输出内容保留、错误提示展示、不完整回复不写入上下文

错误可声明在多轮场景的特定轮次上（如第 1 轮限流、第 2 轮正常），验证重试成功路径。一个场景注入的错误或延迟只作用于该场景自身——场景间隔离由场景状态的独立存储保证（见 [scenario-engine](scenario-engine.md)）。

### 与模型发现的共用

模型发现端点（`/v1/models`）的错误注入（认证失败、超时挂起、服务端错误）复用本层的注入机制，超时注入即「整体延迟超过被测方超时阈值」的特例。被测方的重试与回退行为（10s 超时、瞬态错误重试 3 次、回退知识库）定义于 [llm/model-discovery](../llm/model-discovery.md)，本层只制造输入。

## 数据流

```
场景决策（形状 + 内容块 + 用量 + 投递控制）
  → 非流式路径
    → 整体延迟（若声明）
    → 错误注入检查点：声明 HTTP 错误则返回 HTTP 错误，否则序列化完整 JSON 返回
  → 流式路径
    → 首 token 延迟（若声明）
    → 按分段粒度切分事件序列，每帧之间按逐段延迟间隔发送
    → 用量随协议规定的收尾事件携带（OpenAI 最终 chunk / Anthropic message_delta）
    → 错误注入检查点：起点注入 → 返回 HTTP 错误（含 Retry-After 可选）；
                        中途注入 → 发出若干事件后断开连接
```

## 模块关系

- **模块内上游**：[scenario-engine](scenario-engine.md)（交付投递控制参数与响应内容）
- **模块内下游**：无（直接面向被测二进制的 HTTP 出口）
- **跨模块**：流式事件序列锚定 [llm/protocol-mapping](../llm/protocol-mapping.md) 的统一流式事件映射；错误响应结构以 [fixtures](fixtures.md) 错误基线为形状来源
