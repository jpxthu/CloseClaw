# 边界标记与切分合约

## 概述

边界标记是 System Prompt 静态层和动态层之间的分界标识。System Prompt 模块在向缓存适配层交付时，以边界标记为界确定静态区字段（静态层）和动态区字段（动态层 + 追加区）。本子功能定义切分契约；切分后各字段的缓存控制参数由缓存适配层按供应商策略注入，详见 [llm/cache-adapter](../llm/cache-adapter.md)。

## 架构

### 边界标记

静态层和动态层之间通过 `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` 标记分隔。标记在静态层之后、动态层之前，追加区位于动态层之后、对话历史之前。System Prompt 模块以标记为界确定两个字段（实现可将标记作为字面分隔符插入后切分，也可在分别产出两字段时作为逻辑分界）：

- **静态区字段** = 标记之前的静态层文本
- **动态区字段** = 标记之后的动态层 + 追加区，二者按顺序组合为单个字段

两个字段分别作为 [InternalRequest](../common/shared-types.md#internalrequest--internalmessage--systemblock--tooldefinition) 的 `system_static` / `system_dynamic` 传递。标记是 System Prompt 模块与缓存适配层之间的接口合约——System Prompt 模块负责切分并传递分离后的字段，缓存适配层负责按供应商策略处理各字段。

### 切分规则

- 标记缺失：全文作为静态区，动态区为空
- 出现多个标记：以第一个标记为切分点
- 静态层、动态层、追加区任一为空时，对应字段为空，切分仍成立

### 前缀缓存边界

静态区在 Session 生命周期内字节稳定，由静态层保证；动态区每次请求重新组装（动态层即时构建、追加区读取持久化条目）。本子功能只定义切分与字段语义，不保证供应商缓存命中——静态区的显式缓存标记、动态区的非缓存处理由缓存适配层按供应商策略完成，详见 [llm/cache-adapter](../llm/cache-adapter.md)。

## 数据流

```
1. System Prompt 组装阶段确定静态层、动态层与追加区内容（实现可分别产出，不要求先拼接为单串）
2. System Prompt 模块以边界标记为界，确定静态区字段（静态层）和动态区字段（动态层 + 追加区）
3. 两个字段写入 InternalRequest 的 system_static / system_dynamic，传给缓存适配层
4. 缓存适配层按供应商策略处理（详见 llm/cache-adapter.md）
```

## 模块关系

### 上游

- **System Prompt 组装路径（模块内）**：在向缓存适配层交付时，以边界标记为界确定静态区与动态区两个字段。

### 下游

- **缓存适配层（Cache Adapter）**：接收 System Prompt 模块切分好的静态区和动态区字段，按供应商策略处理。缓存控制参数的具体注入策略见 [llm/cache-adapter](../llm/cache-adapter.md)。

### 无关

- **LLM Provider**：不解析边界标记，消费缓存适配层处理后的请求。
