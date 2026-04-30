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
| `error.rs` | `ProcessError` 枚举及构造函数 |
| `context.rs` | `RawMessage / MessageContext / ProcessedMessage / RawMessageLog` + 单元测试 |
| `processor.rs` | `ProcessPhase` 枚举 + `MessageProcessor` trait |
| `registry.rs` | `ProcessorRegistry` 实现 + 单元测试 |
| `raw_log_processor.rs` | `RawLogProcessor`（入站处理器），Debug 模式或 enabled=true 时将 `RawMessage` 写入 JSON 日志文件，启动时清理超过 `retention_days` 的旧日志 |
| `message_cleaner.rs` | `MessageCleaner`（入站处理器，priority=30），提取纯文本内容（text 类型直接取 text 字段，post 类型展开为 markdown），将 thread_id/root_id/parent_id 写入 metadata 的 `feishu_thread_id` 字段 |
| `markdown_normalizer.rs` | `MarkdownNormalizer`（入站处理器，priority=40），标准化 markdown 内容后再送 LLM：压缩连续空行、去除每行尾随空格、为裸 URL 补全 https:// 前缀、为无语言标识的代码块补全 ` ```text` 标记 |
| `mod.rs` | 模块入口，re-export 公开类型 |

### 数据流

```
Inbound:
  RawMessage → [message_cleaner (priority=30, extracts content, writes feishu_thread_id), ...other processors sorted by priority asc] → ProcessedMessage

Outbound:
  ProcessedMessage (from LLM)
    → synthetic RawMessage
    → [Processor sorted by priority asc]
    → ProcessedMessage
```

### 短路行为

- processor 返回 `Ok(None)` → 链中止（skip），后续 processor 不再执行
- processor 设置 `suppress: true` → 链中止，`suppress` 标志保留至输出
- processor 返回 `Err` → 错误立即向上传播

### Bypass

- inbound 链为空：直接将 `RawMessage` 转为默认 `ProcessedMessage`
- outbound 链为空：直接返回输入的 `ProcessedMessage`
