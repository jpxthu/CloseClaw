# ProcessorChain 模块规格书

## ① 模块概述

`processor_chain` 是消息处理的基础设施模块，为入站（inbound）和出站（outbound）处理器提供链式编排框架。

Inbound 链接收平台原始消息（`RawMessage`），经多个处理器顺序处理后输出 `ProcessedMessage` 送至 LLM；Outbound 链接收 LLM 输出，经处理后送至下游发送逻辑。处理器按 `priority` 升序执行，链中任何一个处理器均可短路后续执行（skip / suppress）。空链直接 bypass。

本模块是纯基础设施，不引用 Gateway 内部字段，可独立被任何层引用。

---

## ② 公开接口

### 类型

- **`RawMessage`** — 入站原始消息，携带 platform / sender_id / content / timestamp / message_id
- **`MessageContext`** — 链内传递的上下文，持有 content、raw_message_log、metadata、skip
- **`ProcessedMessage`** — 链执行结果，持有 final content、metadata、suppress 标志
- **`RawMessageLog`** — 快照，记录链中每步的原始消息副本及来源 processor
- **`ProcessError`** — 错误枚举：`ProcessorFailed` / `InvalidMessage` / `ChainFailed`

### Trait

- **`MessageProcessor`** — 处理器接口，`name / phase / priority / process`

### 枚举

- **`ProcessPhase`** — `Inbound` / `Outbound`，决定处理器加入哪条链

### 配置与加载器

- **`ProcessorChainConfig`** — 入站和出站 processor 列表配置，支持 serde 反序列化，含 `inbound: Vec<ProcessorConfig>` 和 `outbound: Vec<ProcessorConfig>` 字段（均 `#[serde(default)]`）
- **`ProcessorConfig`** — 单个 processor 配置枚举，通过 `type` 字段区分 RawLog / MessageCleaner / MarkdownNormalizer / DslParser / MarkdownToCard
- **`ProcessorChainLoader::load(config)`** — 从 `ProcessorChainConfig` 构造 `ProcessorRegistry`（按 priority 排序注册）

### 核心 API

- **`ProcessorRegistry::new()`** — 创建空 registry
- **`ProcessorRegistry::register(processor)`** — 注册 processor，自动按 phase 加入对应链，返回 `&mut Self`
- **`ProcessorRegistry::inbound_len()`** — 已注册 inbound processor 数量
- **`ProcessorRegistry::outbound_len()`** — 已注册 outbound processor 数量
- **`ProcessorRegistry::process_inbound(raw)`** — 按 priority 升序驱动 inbound 链；空链 bypass（RawMessage → 默认 ProcessedMessage）
- **`ProcessorRegistry::process_outbound(llm_output)`** — 按 priority 升序驱动 outbound 链；空链 bypass（直接返回输入的 ProcessedMessage，不经 from_raw 转换）

---

## ③ 架构 / 结构

### 子模块

| 文件 | 职责 |
|------|------|
| `context.rs` | `RawMessage / MessageContext / ProcessedMessage / RawMessageLog` + 单元测试 |
| `dsl_parser.rs` | `DslParser`（出站处理器，priority=10），解析 `::button[...]` DSL，从 markdown 中移除 DSL 行并将解析结果存入 metadata 的 `dsl_result` 键 |
| `error.rs` | `ProcessError` 枚举及构造函数 |
| `loader.rs` | `ProcessorChainLoader`，根据 YAML/TOML 配置构造 processor 实例并注册到 `ProcessorRegistry` |
| `markdown_normalizer.rs` | `MarkdownNormalizer`（入站处理器，priority=40），标准化 markdown 内容后再送 LLM：压缩连续空行、去除每行尾随空格、为裸 URL 补全 https:// 前缀、为无语言标识的代码块补全 ` ```text` 标记 |
| `markdown_to_card.rs` | `MarkdownToCard`（出站处理器，priority=20），判断输出使用 text 还是 interactive card，将 markdown 格式渲染为飞书卡片元素，从 metadata `dsl_result` 读取 DSL 按钮渲染为卡片 action 按钮 |
| `message_cleaner.rs` | `MessageCleaner`（入站处理器，priority=30），提取纯文本内容（text 类型直接取 text 字段，post 类型展开为 markdown），将 thread_id/root_id/parent_id 写入 metadata 的 `feishu_thread_id` 字段 |
| `processor.rs` | `ProcessPhase` 枚举 + `MessageProcessor` trait |
| `raw_log_processor.rs` | `RawLogProcessor`（入站处理器），Debug 模式或 enabled=true 时将 `RawMessage` 写入 JSON 日志文件，启动时清理超过 `retention_days` 的旧日志 |
| `registry.rs` | `ProcessorRegistry` 实现 + 单元测试 |
| `registry_tests.rs` | `ProcessorRegistry` 的集成测试（通过 `#[cfg(test)] mod tests` 间接测试） |

### Outbound Fixtures

| 文件 | 场景 |
|------|------|
| `tests/fixtures/outbound/fixture_E1.json` | 纯文本 → text 消息 |
| `tests/fixtures/outbound/fixture_E2.json` | 加粗/斜体/行内代码 → interactive card |
| `tests/fixtures/outbound/fixture_E3.json` | DSL 按钮 → 卡片 action 按钮 |
| `tests/fixtures/outbound/fixture_E4.json` | 标题提取（`# 标题`）→ header.title |
| `tests/fixtures/outbound/fixture_E5.json` | 分割线 `---` → hr 元素 |
| `tests/fixtures/outbound/fixture_E6.json` | 代码块 → card code 元素 |
| `tests/fixtures/outbound/fixture_E7.json` | 混合格式（含标题+分割线+代码块）→ interactive card |
| `tests/fixtures/outbound/fixture_E8.json` | 无 markdown 纯文本 → text 消息（边界）|

### 数据流

```
Inbound:
  RawMessage → [message_cleaner (priority=30, extracts content, writes feishu_thread_id), ...other processors sorted by priority asc] → ProcessedMessage → LLM

Outbound:
  ProcessedMessage (from LLM)
    → synthetic RawMessage (via ProcessedMessage.clone)
    → [dsl_parser (priority=10, parses ::button[...] DSL, removes DSL lines, writes dsl_result to metadata),
       markdown_to_card (priority=20, renders markdown as Feishu card or text), ...other processors sorted by priority asc]
    → ProcessedMessage
```

### 短路行为

- processor 返回 `Ok(None)` → 链中止（skip），后续 processor 不再执行
- processor 设置 `suppress: true` → 链中止，`suppress` 标志保留至输出
- processor 返回 `Err` → 错误立即向上传播

### MarkdownToCard 判断逻辑

- 空内容 → bypass（返回 `None`，不输出任何消息）
- 纯文本无 markdown 格式 → 输出 `{"msg_type":"text","content":{"text":"..."}}`
- 含 markdown 格式（`**`/`*`/代码块/列表/`#`/`>`/链接等）或含换行符或含 DSL → 输出 `{"msg_type":"interactive","card":{...}}`
- 内容第一行是 `# 标题` → 提取为 header.title（模板 blue），该行不进入 elements；`##` 及以下 → MarkdownElement
- `---` → `{"tag":"hr"}` 元素
- 从 metadata `dsl_result` 读取 DslInstruction 按钮，第一个 `type:primary`，其余 `type:default`
- 无 `dsl_result` metadata → 不报错，正常渲染 markdown
- metadata 中 `dsl_result` JSON 解析失败 → 跳过按钮渲染

### Bypass

- inbound 链为空：直接将 `RawMessage` 转为默认 `ProcessedMessage`（调用 `ProcessedMessage::from_raw`）
- outbound 链为空：直接返回输入的 `ProcessedMessage`（不创建 synthetic RawMessage）
