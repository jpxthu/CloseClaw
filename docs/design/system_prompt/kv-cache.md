# 边界标记与前缀缓存

## 概述

边界标记是 System Prompt 静态层和动态层之间的分隔锚点，同时作为 cache adapter 的程序输入——cache adapter 以其为切分点，对静态前缀注入缓存控制参数，为动态后缀和消息历史透传不做标记。

## 架构

### 边界标记

静态层和动态层之间通过 `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 标记分隔。标记的位置在 System Prompt 组装完成后的最终文本中，静态层之后、动态层之前。

标记是 cache adapter 的程序输入，而非仅供人类阅读的装饰文本——cache adapter 以标记为切分点，标记之前的内容作为可缓存前缀（cache adapter 对其标记显式缓存控制参数），标记之后的内容每次请求重新计算，不参与显式前缀缓存（但服务端自动前缀缓存的 provider 仍可因稳定前缀获益）。追加区位于动态层之后、对话历史之前。

### 前缀缓存

API 侧 KV Cache 通过 cache adapter 层实现。静态层和动态层通过边界标记分离后，Anthropic 适配在静态层上标记显式缓存控制参数（cache_control），使 Provider 复用静态前缀的 KV cache，仅对动态层和新增消息计费。DeepSeek / OpenAI 等供应商使用服务端自动前缀缓存，无需客户端标记，命中率由前缀的字节稳定性保证。

## 数据流

```
System Prompt 组装完成（静态层 + 边界标记 + 动态层 + 追加区）
  →
  cache adapter 读取边界标记位置
    → 标记之前：静态前缀（Anthropic 注入 cache_control 参数，其他供应商透传）
    → 标记之后：动态后缀 + 追加区（透传）
  →
  发送 LLM 请求
```

## 模块关系

### 上游

- **System Prompt 构建流程**：静态层构建完成后、动态层之前插入边界标记。

### 下游

- **Cache Adapter**：以边界标记为切分点，对静态层按供应商策略处理（Anthropic 注入 cache_control，DeepSeek/OpenAI 透传）。详见 [llm/cache-adapter](docs/design/llm/cache-adapter.md)。

### 无关

- **Compaction 模块**：边界标记不参与对话消息压缩。压缩后触发 system prompt 重建，静态层重建时边界标记随之重新定位。
- **追加区**：追加区位于动态层之后、边界标记之后，不参与前缀缓存。
