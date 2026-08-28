# 推理深度控制

## 概述

`/reasoning` 指令用于查询或设置当前会话的推理深度档位，或请求关闭推理输出。推理深度控制 LLM 在生成回复前的内部推理量，四个档位适用于不同的任务复杂度。

## 架构

推理深度有两个生效入口：config 全局默认值 + `/reasoning` 运行时覆盖。运行时覆盖优先级高于 config 默认值。

**四个档位**：Low、Medium、High、Max。High 为默认档位。不支持的档位由 Provider 侧自动降级（如 Max 在不支持的模型上降为 High）。

**关闭推理请求**：`off` 不是档位，是关闭推理输出的请求，实际效果取决于供应商能力——支持关闭推理的 provider 真正关闭推理输出；不支持的 provider 不视为错误，仅将推理强度降至最低可用档位。完整语义与 provider 映射见 [LLM 会话增强](../session/llm-session-enhancements.md)。

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

- **`/reasoning`**（无参数）：读取 session 当前实际生效档位（含 provider 降级后的值；关闭请求在支持关闭的 provider 上显示为已关闭，在不支持的 provider 上显示为降级后的最低可用档位）→ Reply
- **`/reasoning low|medium|high|max|off`**：解析档位或关闭请求 → SetReasoning(level|off) → Gateway 写入 session，回复实际生效结果——档位变更回复实际生效档位（含 provider 降级后的值）；关闭请求按供应商能力回复已关闭或已降至最低可用档位。off 的 provider 侧映射见 [LLM 会话增强](../session/llm-session-enhancements.md)。

## 模块关系

- **上游**：Gateway → Dispatcher → ReasoningHandler
- **下游**：Session 模块（`reasoning_level` 字段读写）；LLM 模块（读取 reasoning_level 映射为原生参数）
- **无关**：Processor 链（指令在 Gateway 层处理完毕，不进入 LLM）、Verbosity（`/verbose` 控制展示多少信息给用户，`/reasoning` 控制 LLM 内部推理量，两轴独立）
