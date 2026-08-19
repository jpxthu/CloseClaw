# 协议 fixture

## 概述

协议 fixture 是 JSON 格式的双协议响应形状基线：覆盖 OpenAI 与 Anthropic 协议的完整协议面——响应结构、流式事件序列、工具调用、推理/思考、用量字段、错误响应结构。它既是 Fake LLM Server 生成响应的形状来源，也是双端单元测试的基线：一端验证 Fake LLM Server 生成的响应符合协议格式，另一端验证 CloseClaw 能正确解析这些响应。

## 架构

### 与供应商 fixture 的分工

两套 fixture 体系互补不重叠：

| | 协议 fixture（本模块） | 供应商 fixture（tests/fixtures/llm/） |
|--|--|--|
| 数据来源 | 协议标准手写基线 | 真实供应商 API 采集 |
| 覆盖对象 | 协议本身的形态 | 供应商/模型特有行为 |
| 服务层 | 单元测试 + Fake LLM Server 响应形状 | 单元测试 |
| 典型内容 | 标准响应结构、SSE 事件序列、错误结构 | thinking 标签混排、供应商错误码、特有参数 |

Fake LLM Server 不模拟供应商特有行为（需求 F2），其响应形状全部以协议 fixture 为基线——场景引擎产出的每一类响应形状，在协议 fixture 中都有对应的形态实例。

### fixture 结构

每个 fixture 是一个 JSON 文件，顶层字段与集成测试的供应商 fixture 共用同一套场景文件格式（需求 F3：场景格式在集成测试与黑盒端到端测试间复用）：`protocol` / `streaming` / `scenario` / `model` / `expect` / `request` / `response`（流式场景为同目录 `.txt` 原始 SSE 文本 + `-meta.json` 请求元数据），工具调用场景附 `tools_sent`。fixture 中的 model 统一使用中性占位模型 ID，不含任何真实供应商痕迹。

### 覆盖矩阵

双协议 × 协议面的完整组合：

| 协议面 | OpenAI | Anthropic |
|--------|--------|-----------|
| 基础文本响应 | `simple` | `anthropic-simple` |
| 推理/思考 | `reasoning`（`reasoning_content` 字段） | `anthropic-thinking`（thinking 块 + signature） |
| 工具调用 | `tool-use`（`tool_calls` + `finish_reason=tool_calls`） | `anthropic-tool-use`（`tool_use` 块） |
| 工具调用流式 | `tool-use-streaming`（`delta.tool_calls` 增量） | `anthropic-tool-use-streaming`（`input_json_delta`） |
| 流式文本 | `streaming`（chunk 序列 + `[DONE]`） | `anthropic-streaming`（事件序列） |
| 缓存用量 | `cache`（`cached_tokens`） | `anthropic-cache`（read + creation） |
| 错误响应 | `error-auth` / `error-rate-limit` / `error-server` | `anthropic-error` |

### 双端单元测试

同一套 fixture 驱动两端的单元测试：

- **Fake LLM Server 端**：给定场景声明，Fake LLM Server 生成的响应与 fixture 逐字段一致——验证生成器的协议正确性
- **CloseClaw 端**：给定 fixture 作为输入，CloseClaw 的协议解析与归一化产出符合 [llm/protocol-mapping](../llm/protocol-mapping.md) 映射的统一内容块/流式事件——验证消费方的解析正确性

双端共享同一基线是协议契约的双向锁定：Fake LLM Server 不会发出协议外形态，CloseClaw 的解析能力以 fixture 全集为验收范围。用量字段的字段形状表（输入/输出/推理/缓存命中/缓存写入的双协议原生位置）以 [llm/protocol-mapping](../llm/protocol-mapping.md) 为权威，fixture 是该表的实例化。

### 场景扩展

覆盖矩阵之上的扩展场景按需补充，同受 fixture 格式约束：多轮对话（`turns`）、工具结果回传（`tool-result`）、流式思考、并行工具调用、max_tokens 截断等。扩展不改变结构定义，只增加矩阵条目。

## 数据流

```
协议 fixture 集（JSON / SSE 基线）
  ├─▶ Fake LLM Server 单元测试：场景声明 → 生成响应 ≡ fixture
  ├─▶ CloseClaw 单元测试：fixture 输入 → 解析结果 ≡ protocol-mapping 映射
  └─▶ Fake LLM Server 运行时：场景响应形状的基线来源
        （[scenario-engine](scenario-engine.md) 产出 → [endpoints](endpoints.md) 协议层序列化）
```

## 模块关系

- **模块内上游**：无（静态数据基线，无直接上游）
- **模块内下游**：[scenario-engine](scenario-engine.md)（响应形状以本基线为准）、[delivery](delivery.md)（错误结构基线）、[endpoints](endpoints.md)（协议层序列化以基线为准）
- **跨模块**：双协议映射权威 → [llm/protocol-mapping](../llm/protocol-mapping.md)；与 `tests/fixtures/llm/` 供应商 fixture 体系互补（见架构节的分工表）
