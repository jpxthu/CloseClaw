# 缓存适配器

## 概述

缓存适配器（API 缓存优化层，独立于旧 Provider 桥接）为不同 LLM 供应商统一缓存策略，最大化 system prompt 静态区前缀缓存的命中率，降低 Token 计费成本。不同供应商的缓存机制差异巨大，适配器层的职责是让上游业务代码无需感知供应商差异。

## 架构

缓存适配器采用 Adapter 模式，为每个供应商提供独立的缓存实现。缓存适配器在 Plugin Pipeline 的 `before_request` 之前执行——它是请求发送前的独立前置处理层，不是 Plugin 的一部分。缓存适配器仅处理静态区内容（标记缓存控制参数等），动态区内容透传不做修改。

```
System Prompt 构建
  → 加载静态区内容
  → 组装动态区内容
  → 缓存适配器（请求前置处理，在 Plugin Pipeline 之前）
    ├─ Anthropic：静态区标记 cache_control（显式前缀缓存）
    └─ DeepSeek / OpenAI / 其他：无显式缓存参数（依赖前缀稳定性触发服务端自动前缀缓存）
  → Plugin Pipeline（before_request → 后续 LLM Client 标准链路）
  → 发送到 LLM API
```

### Anthropic 适配

Anthropic 支持显式前缀缓存。在系统提示的静态区内容块上标记缓存控制参数，服务端对相同前缀的后续请求自动命中缓存。首次请求以全价建立缓存，后续命中仅收取约 10% 的费用。缓存有效期为 5 分钟，可付费延长。

### 其他供应商

DeepSeek 和 OpenAI 使用服务端自动前缀缓存，无需客户端显式标记缓存控制参数。缓存命中依赖请求前缀的字节稳定性——system prompt 模块保证静态层和动态层在 session 生命周期内内容不变，消息历史只追加不重写，使自动前缀缓存在连续请求中持续命中。缓存适配器对这两家不做额外处理，命中率由前缀稳定性保证。

> **注意**：缓存策略取决于供应商 API 的缓存特性，与协议格式选择无关。DeepSeek 在对话调用中使用 Anthropic 协议格式（便于签名追溯），但 DeepSeek 的 API 不支持 Anthropic 的显式前缀缓存（cache_control）。缓存适配器按供应商 API 的能力选择策略，不因协议格式而改变。

### 边界标记

缓存适配器依赖 system prompt 模块的 `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 边界标记来切分可缓存前缀和不可缓存后缀。标记是缓存适配器的程序输入，而非仅供人类阅读的装饰文本。标记之前的内容作为静态区，标记之后的内容作为动态区；缓存适配器只对静态区标记缓存控制参数（Anthropic），对 DeepSeek / OpenAI 不做额外处理（service 端自动前缀缓存不依赖客户端标记），动态区内容统一透传不做修改。

### 消息历史缓存

缓存适配器仅处理静态区内容（system blocks）的缓存标记。messages 数组的 cache_control 标记由 Protocol 层在请求序列化时完成——Anthropic 协议在 messages 数组尾部标记 cache_control，使下一轮请求的完整前缀（上轮的 system prompt + 全部 messages）命中缓存。每轮请求只在消息序列的最尾部打一个 cache_control，指向前缀匹配点。

DeepSeek 和 OpenAI 对消息历史部分按前缀匹配生效——消息数组只追加、前缀不变，历史消息的前缀部分与 system prompt 共同被服务端自动前缀缓存覆盖。每轮请求只需计费新增的消息尾部。

### 工具 Schema 缓存

当工具定义通过 API 的 `tools` 参数传递时，所有工具 Schema 统一标记 cache_control，使工具定义在整个 session 内被前缀缓存覆盖。额外加载的工具定义追加在 base Schema 之后，切换时仅损失追加部分的缓存，不影响 base Schema 的命中。

当工具定义以文本形式嵌入 system prompt 时，它作为静态层的一部分自然被前缀缓存覆盖，无需在 tools 参数上单独标记。

## 数据流

```
system_prompt 模块构建 system prompt
  → 拆分静态区（bootstrap 文件 + ToolsSection + SkillListingSection + MemorySection）与动态区（ChannelContext + WorkingDirectory + GitStatus）
  → 缓存适配器接收（供应商 ID、静态区内容、动态区内容、会话 ID）
    → Anthropic：静态区标记缓存控制 → 标准协议请求
    → DeepSeek / OpenAI：无额外处理（前缀稳定性由 system prompt 模块保证）→ 标准协议请求
  → 进入 LLM Client 标准调用链路
  → 发送请求
  → 从用量字段监控缓存命中（cached_tokens 计数）
```

## 模块关系

- **上游**：system_prompt 模块（提供静态区和动态区内容）、Session 层（提供会话 ID）
- **下游**：LLM Client（缓存适配器由 LLM Client 持有并作为第一步调用，处理后的请求进入 Plugin Pipeline 和后续标准链路）
- **无关**：provider-config-wizard、model-discovery（缓存适配器运行在对话请求热路径上，与配置阶段工具无关）
