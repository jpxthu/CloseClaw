# 推理深度控制

## 概述

`/reasoning` 指令用于查询或设置当前会话的推理深度。推理深度控制 LLM 在生成回复前的内部推理量，四个等级适用于不同的任务复杂度。

## 架构

推理深度有两个生效入口：config 全局默认值 + `/reasoning` 运行时覆盖。运行时覆盖优先级高于 config 默认值。

**四个等级**：Low、Medium、High、Max。High 为默认值。`off` 是 Low 的别名。不支持的等级由 Provider 侧自动降级（如 Max 在不支持的模型上降为 High）。

```
/reasoning medium
  ↓
ReasoningHandler 返回 SetReasoning(Medium)
  ↓
Gateway 写入 session reasoning_level = Medium
  ↓
回复"推理深度已设为 Medium"
  ↓
下次 LLM 调用 → LLM 模块将 Medium 映射为各模型的原生参数
```

`/reasoning` 无参数时查询当前值，不改变设置。

## 数据流

- **`/reasoning`**（无参数）：读取 session 当前推理深度 → Reply("当前推理深度：Medium")
- **`/reasoning low|medium|high|max|off`**：解析等级 → SetReasoning(level) → Gateway 写入 session。`off` 映射为 Low。

## 模块关系

- **上游**：Gateway → Dispatcher → ReasoningHandler
- **下游**：Session 模块（`reasoning_level` 字段读写）；LLM 模块（读取 reasoning_level 映射为原生参数）
- **无关**：Processor 链（指令在 Gateway 层处理完毕，不进入 LLM）
