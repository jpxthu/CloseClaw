# Fake LLM Server

## 概述

- 关联需求文档：[requirements/fake_llm.md](../../requirements/fake_llm.md)
- Fake LLM Server 是本地回环地址上的独立测试进程，同时支持 OpenAI 与 Anthropic 两种标准对话协议，按测试者声明的场景对 CloseClaw 的完整 LLM 调用链路做确定性、可复现的黑盒端到端测试。它是开发与测试工具，不进入发布构建。

## 架构

Fake LLM Server 是一个场景驱动的 HTTP 服务：监听本地回环地址，接收 CloseClaw 编译后二进制的全部 LLM 协议请求，按场景引擎给出的确定性决策响应，把响应按协议形态（完整 JSON 或 SSE 流式事件序列）投递回去。整体分四层：

```
CloseClaw 二进制（黑盒被测系统）
  │ HTTP（OpenAI / Anthropic 协议）
  ▼ 仅监听 127.0.0.1
┌─────────────────────────────────────────────┐
│ Fake LLM Server（独立测试进程，不进发布构建）      │
│                                             │
│  协议端点层 endpoints                          │
│    ├─ OpenAI /v1/chat/completions            │
│    ├─ Anthropic /v1/messages                 │
│    └─ 模型发现 /v1/models                     │
│  场景引擎 scenario-engine                     │
│    ├─ 请求特征匹配 → 命中唯一场景                 │
│    ├─ 多轮游标推进                             │
│    └─ KV cache 模拟状态机（kv-cache-simulation）│
│  投递层 delivery                              │
│    ├─ 完整 JSON / SSE 流式                    │
│    ├─ 错误注入（HTTP 错误 / 流中断）             │
│    └─ 延迟注入（首 token / 逐段 / 整体）         │
│  协议 fixture fixtures                        │
│    ├─ 响应形状的协议标准基线                     │
│    └─ 双端单元测试基线                          │
└─────────────────────────────────────────────┘
```

三个设计原则：

- **协议标准而非供应商模拟**：不模拟具体供应商或模型的特有行为，只实现两种协议的标准形态。供应商差异由 `tests/fixtures/llm/` 的供应商 fixture 在单元测试层面覆盖，Fake LLM Server 的响应形状以协议 fixture（fixtures）为唯一基线，来源中立。
- **确定性**：同一场景下同一输入永远产出同一输出，不依赖时序或随机性。延迟注入只影响事件的时间间隔，不影响事件序列与内容。
- **场景间隔离**：每个场景一个独立的引擎状态（匹配上下文、轮次游标、缓存状态），一个场景注入的错误或延迟不污染后续场景；多个并发会话（不同模型端点配置）互不干扰。

可观测性是横切能力而非独立层：端点、场景引擎、投递、缓存模拟各环节将匹配结果、响应与用量、缓存事件统一写入同一份测试日志，供测试失败时定位。

**子功能文档**：
- [endpoints](endpoints.md) — 进程模型与协议端点：回环监听、双对话协议端点、模型发现模拟
- [scenario-engine](scenario-engine.md) — 场景引擎：请求特征匹配、逐轮响应编排、七类响应形状
- [delivery](delivery.md) — 投递层：流式分帧与段间间隔控制、错误注入、延迟注入
- [kv-cache-simulation](kv-cache-simulation.md) — 前缀缓存生命周期模拟：自动前缀匹配、显式注入、断开与过期
- [fixtures](fixtures.md) — 协议 fixture：双协议响应形状基线与双端单元测试

## 数据流

```
CloseClaw 二进制发起 LLM 请求（OpenAI 或 Anthropic 协议）
  → 协议端点层接收（回环地址 + 协议路由）
    → 场景引擎：请求特征（协议、模型 ID、消息内容、工具定义、参数）
      → 命中唯一场景、推进多轮游标；KV cache 模拟状态机按前缀稳定性推导缓存字段
    ← 决策 =（协议响应对象、错误注入、延迟注入）
  → 投递层按场景声明选择流式或非流式投递
    ├─ 非流式：延迟后返回完整 JSON
    └─ 流式：首 token 延迟 → 逐段分帧 SSE → 段间间隔 → 结束事件
    两种路径均可注入错误：HTTP 错误（认证/限流/超时/服务端）或流中途截断

模型发现请求（GET /v1/models）
  → 协议端点层路由 → 场景决策 → 经投递层返回模型列表或注入错误
  ← CloseClaw 解析响应，链路行为由测试断言
  → 每次请求的匹配结果、响应与用量、缓存事件写入测试日志
```

## 模块关系

Fake LLM Server 是测试基础设施，与 CloseClaw 的关系是**黑盒替换**：不引用任何 CloseClaw 代码，通过配置 CloseClaw 的模型端点（models.json 的 base_url）指向本地地址接入，被测对象是编译后的真实二进制。协议正确性锚定 [llm/protocol-mapping](../llm/protocol-mapping.md)（协议→统一块映射）与 [llm/model-discovery](../llm/model-discovery.md)（模型发现行为）。

- **上游**：无代码上游（CloseClaw 二进制是它的 HTTP 客户端，非模块依赖）
- **下游**：无（测试断言由测试代码消费其响应与日志）
- **无关**：`tests/fixtures/llm/` 的供应商 fixture 体系（采集真实供应商响应，验证供应商特有行为；Fake LLM Server 实现协议标准，两者互补不重叠，见 [fixtures](fixtures.md) 的分工）
- **被测行为的权威定义**：跨轮用量统计、KV cache 命中率与命中率下降告警 → [session/llm-session-enhancements](../session/llm-session-enhancements.md)；用量字段提取与协议归一化、缓存策略 → [llm](../llm/README.md)。fake_llm 只制造输入，不定义被测行为的对错标准
