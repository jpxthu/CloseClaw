# 协议 fixture

## 概述

协议 fixture 记录 OpenAI 与 Anthropic 协议的响应形态：真实采集响应是字段、取值与流式形态的唯一事实依据，合成样本用于生成方编码回归与双端单元测试的确定性输入。

## 架构

### 证据分级与权威

两套 fixture 的差别是证据等级不同：真实采集记录供应商实际返回，合成样本记录本服务按声明应产出的形态。

| | 真实采集（`tests/fixtures/llm/`） | 合成 fixture（`tests/fixtures/fake_llm/`） |
|--|--|--|
| 内容 | 供应商 API 的实际请求与响应 | 按场景声明构造、供测试对照的样本 |
| 作用 | 字段存在性、取值与语义的唯一事实依据 | 生成方编码回归、消费方确定性输入 |
| 不承担 | 不提供确定性边界与故障注入 | 不证明任何供应商行为，不替代真实采集 |

涉及 API 事实的判断——字段是否存在、字段含义、某分项能否在响应中报告、流式用量何时到达——一律以真实采集响应的原文为准；合成样本是测试机制的输入，不能反过来定义协议事实，也不能把合成取值当作供应商实测值。供应商 README、`expect` 标注与 API summary 是加工结论，不是原始响应，冲突时以原文为准。

尚未取得实采依据的形态在合成样本中构造时必须标注为构造值：429 限流、5xx 与 504、403 权限不足、404、空响应体、并行工具调用即属此类。它们可用于测试被测方的分支，但不得作为供应商行为的证据。

### 覆盖边界与空白登记

真实采集未出现的字段或形态是明确空白，逐项登记，不用合成样本冒充已覆盖；也不因当前样本没有某项证据，判定协议不提供该能力。字段取值分三态：缺失（字段未出现、为 `null` 或细节对象为空）、显式零、实采有正值；三态在 [llm/protocol-mapping](../llm/protocol-mapping.md) 的用量表中逐项记录。

已登记的空白包括：缓存写入无正取值样本；OpenAI 格式未见独立缓存写入字段；Anthropic 格式未见独立推理用量字段；缓存过期无真实等待采样；错误族无 429、5xx、504、403 与空响应体样本；未知模型在实采中返回 400，不是 404；实采无并行工具调用样本（单响应只有单个工具调用/单工具块）。空白随采集更新，不属于本模块的行为契约。

### fixture 记录格式

协议 fixture 以 JSON 记录，顶层字段为 `protocol` / `streaming` / `scenario` / `model` / `expect` / `request` / `response`，并按场景附 `turns`（逐轮 messages 与响应）、`rounds`（工具调用多轮）、`tools_sent`、`max_tokens_sent`、`system_sent`、`extra_body_sent`。该格式由本文件定义，与 [scenario-engine](scenario-engine.md) 的场景声明是两种文件：场景声明写匹配条件与响应步骤，fixture 记录采集或构造出的协议报文。

采集记录的 `request` 只保留 `model` 与 `messages`，流式开关、工具与采样参数另行记录在旁路字段；读取方不能把 `request` 当作完整请求原文。流式报文以同目录 `.txt` 保存、请求元数据存于 `-meta.json`；采集可能存在 Python 字面量包装（形如 `{'_raw': 'data: ...'}`），读取方须先安全解包再按 SSE 解析，不得假定为裸 SSE。合成样本的 `model` 使用中性占位标识，不冒充真实供应商。

### 覆盖矩阵

矩阵表示生成方必须支持的协议面类别，不表示每一类都有实采样本：

| 协议面 | OpenAI | Anthropic | 实采依据 |
|--------|--------|-----------|----------|
| 基础文本响应 | `simple` | `anthropic-simple` | 有 |
| 推理/思考 | `reasoning`（`reasoning_content`） | `anthropic-thinking`（thinking 块 + signature） | 有（供应商特有场景） |
| 工具调用 | `tool-use` | `anthropic-tool-use` | 有 |
| 工具调用流式 | `tool-use-streaming` | `anthropic-tool-use-streaming` | 有 |
| 并行工具调用 | 多个 `tool_calls` | 多个 `tool_use` 块 | 无（构造值） |
| 流式文本 | `streaming` | `anthropic-streaming` | 有 |
| 多轮对话 | `multi-turn` | `anthropic-*` 多轮 | 有 |
| 多轮缓存增量 | `cache`（`turns`） | `anthropic-cache`（`turns`） | 有 |
| 工具结果回传 | `tool-result`（`rounds`） | `anthropic-tool-result` | 有 |
| 上下文压力 | `context-pressure`（`turns`） | `anthropic-context-pressure` | 有 |
| 用量 | 输入、输出、推理、命中；写入无落点（按缺失） | 输入、输出、命中、写入；推理无独立落点 | 见 protocol-mapping 落点表 |
| 错误族 | 认证、非法请求（含空 messages、工具格式、未知模型 400）、限流、服务端、超时 | 同左 | 认证与非法请求有；429/5xx/504 无 |
| 模型发现 | `provider/model-list.json` | 同左 | 有 |

生成方缺少某类条目时按本表补条目；消费方验收不得只依赖合成条目，必须以实采样本覆盖字段三态、流式最终值覆盖初值与供应商差异。

### 双端单元测试

- **生成方**：给定场景声明，核对服务产出的响应符合协议落点与合成 fixture 记录的形态；对当前实现尚未产出的字段按实现缺口登记，不放宽断言掩盖。
- **消费方**：以同一套合成 fixture 为输入验证解析与归一化（需求 F11 的双向契约），并以真实采集响应验收字段三态、流式最终值覆盖、多轮增量与供应商差异；实采是 API 事实依据，合成样本不能替代。
- 两端共用同一套合成 fixture 与协议落点表，但验证方向不同：生成方验「声明 -> 响应」编码，消费方验「响应 -> 统一模型」归一化；生成方的自洽不能作为消费方正确性的证据。

### 场景扩展

覆盖矩阵之上的扩展场景按需补充，同受记录格式约束：流式思考、并行工具调用、max_tokens 截断、空响应体等。扩展只增加矩阵条目，不改变记录格式。

## 数据流

```text
真实采集响应（tests/fixtures/llm/，事实来源）
  ├─▶ 归纳入协议落点表（llm/protocol-mapping）：字段位置、三态语义、流式到达时机
  └─▶ 消费方验收：实采响应 -> 解析归一化 ≡ 落点表

合成 fixture（tests/fixtures/fake_llm/）
  ├─▶ 生成方单元测试：场景声明 -> 响应 ≡ fixture（编码与确定性）
  └─▶ 消费方单元测试：合成 fixture -> 解析归一化（双向契约，F11）
```

Fake LLM Server 运行时只装载场景声明，不读取协议 fixture；fixture 是测试资产，运行时产物应与它同形。

## 模块关系

- **模块外上游**：真实采集响应是只读事实来源，位于测试数据目录，不属于本模块产物。
- **模块内下游**：[scenario-engine](scenario-engine.md) 按落点表产出响应与用量，[endpoints](endpoints.md) 协议层序列化，[delivery](delivery.md) 的流式与错误形态以实采为准。
- **跨模块**：[llm/protocol-mapping](../llm/protocol-mapping.md) 承载协议映射与用量落点表，落点由实采归纳；记录格式由本文件唯一所有，场景声明格式见 [scenario-engine](scenario-engine.md)。
