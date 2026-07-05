# 运行时配置

## 概述

VerbosityLevel 是出站信息展示等级的枚举，控制 VerbosityFilter 对 ContentBlock 的过滤策略。CompactConfig、ReasoningLevel 和 PromptOverrides 是其他运行时配置相关的类型。

> **本文档定义的 CompactConfig、ReasoningLevel、VerbosityLevel、PromptOverrides 在 common crate 中实现。引用本模块的下游文档通过 [ContentBlock](content-block.md)、[ProcessedMessage](processed-message.md) 等链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### VerbosityLevel

VerbosityLevel 控制 VerbosityFilter 对 ContentBlock 的过滤策略。由 `/verbose` 指令设置，Session 存储，出站 Processor Chain 的第一道过滤（VerbosityFilter，priority 5）消费。

三个等级：

| 等级 | 值 | 过滤行为 |
|------|-----|----------|
| full | `"full"` | 展示全部：思考过程、工具调用、工具结果、最终回复 |
| normal | `"normal"` | 展示工具调用和结果作为进度提示，隐藏思考过程 |
| off | `"off"` | 仅展示最终回复，隐藏所有中间过程 |

**作用范围**：Verbosity 控制展示内容，不影响 LLM 推理深度和 Agent 行为模式。切换等级不影响当前正在输出的消息——仅对后续新消息生效。非文本媒体块（Image/Audio/File）属于最终回复的一部分，不受 VerbosityLevel 过滤——在所有等级下均展示。

### ReasoningLevel

ReasoningLevel 是推理深度的枚举，控制 LLM 的推理复杂度。

> **文档编写中** — ReasoningLevel 的具体枚举值和语义待 LLM Provider 配置方案确定后细化。当前在 [SlashResult SetReasoning](slash-result.md) 中引用，用于设置推理深度等级。

### CompactConfig

CompactConfig 是对话历史压缩的配置结构。

> **文档编写中** — CompactConfig 的具体字段定义待 Compaction 模块实现后确定。

### PromptOverrides

PromptOverrides 是提示词覆盖配置，用于在运行时临时替换或追加 system prompt 内容。

> **文档编写中** — PromptOverrides 的具体字段定义待 Session 配置管理方案确定后细化。

## 数据流

### VerbosityLevel

VerbosityLevel 的读写路径：

```
/verbose <等级> 指令
  ↓
VerboseHandler 设置等级
  ↓
Gateway 写入 Session 的 Verbosity 字段
  ↓
出站 Processor Chain 的第一道 Processor（VerbosityFilter，priority 5）读取
  ↓
按等级过滤 ContentBlock[] — 去除被隐藏的块类型
  ↓
过滤后的 ContentBlock[] 继续后续出站链路（DslParser → OutboundRawLog → Renderer）
```

### ReasoningLevel

ReasoningLevel 的设置和消费路径：

```
/reasoning <等级> 指令
  ↓
ReasoningHandler 设置等级
  ↓
Gateway 写入 Session 的 reasoning_level 字段
  ↓
Gateway 在构造 LLM 请求时读取 reasoning_level
  ↓
传递给 LLM Provider（通过 provider-specific 参数，如 reasoning_effort）
  ↓
LLM 按指定推理深度产出响应
```

### CompactConfig

CompactConfig 的配置和消费路径：

```
Config 模块加载 CompactConfig（来自配置文件）
  ↓
Session 在 compaction 触发时读取：
  ├── 自动触发：检查上下文 token 是否超过 `窗口 - auto_compact_buffer_tokens`
  │   └── 超过阈值 → 调用 Compaction 模块执行压缩
  ├── 手动触发：/compact 指令 → 直接调用 Compaction 模块
  └── 失败处理：连续失败超过 max_consecutive_failures → 断路器切断自动压缩
  ↓
Compaction 模块根据 chars_per_token 估算 token 用量
```

### PromptOverrides

PromptOverrides 在 system prompt 构建中的使用路径：

```
Agent 注册/配置时设置 PromptOverrides（各字段可选）
  ↓
Gateway 触发 system prompt 构建时传入 PromptOverrides
  ↓
SystemPromptBuilder 按优先级检查：
  1. override_prompt != None → 直接使用，跳过所有 section
  2. agent_prompt != None → 替换 agent 级 prompt
  3. custom_prompt != None → 在 agent 级 prompt 上叠加
  4. 全部 None → 正常 section 渲染流程
  ↓
产出最终 system prompt 字符串 → 写入 Session
```

## 模块关系

### VerbosityLevel

- **生产者**：slash 模块（VerboseHandler 处理 `/verbose` 指令，写入 Session）
- **消费者**：Processor Chain 出站（VerbosityFilter 读取并过滤 ContentBlock[]）；Session（存储当前等级，供下次出站过滤）
- **无关**：LLM Provider（Verbosity 不影响 LLM 推理，仅控制展示）、IM Adapter 入站（入站不涉及展示过滤）

### ReasoningLevel

- **生产者**：slash 模块（ReasoningHandler 处理 `/reasoning` 指令，写入 Session）
- **消费者**：Gateway（构造 LLM 请求时读取推理等级并传递给 LLM Provider）；Session（持久化存储当前等级）
- **无关**：IM Adapter（不感知推理深度）、Processor Chain（ReasoningLevel 在 LLM 调用前已消费，不出现在入站/出站处理链中）

### CompactConfig

- **生产者**：Config 模块（从配置文件加载 CompactConfig）
- **消费者**：Compaction 模块（读取配置确定压缩触发条件和估算参数）；Session（在 compaction 触发时通过管理器间接引用）
- **无关**：LLM Provider（压缩配置不影响 LLM 调用）、IM Adapter（不参与压缩决策）

### PromptOverrides

- **生产者**：Agent 注册/配置（各 Agent 设置自己的 PromptOverrides）
- **消费者**：system_prompt 模块（SystemPromptBuilder 接收 PromptOverrides，按优先级选择覆盖策略）
- **无关**：LLM Provider（不接触 PromptOverrides 结构，消费的是构建后的最终 prompt 文本）、Processor Chain（不参与 system prompt 构建）
