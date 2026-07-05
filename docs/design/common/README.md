# Common

## 概述

common 是跨模块共享概念的唯一定义地。包含两类内容：跨模块传递的纯数据结构（共享类型）和依赖注入接口（核心 trait）。各业务模块文档通过引用指向此处，不在自身文档中重复定义。

## 架构

common 不是业务模块——它不含可执行逻辑，只定义数据结构和接口契约。所有业务模块通过 common 中的共享类型和 trait 进行解耦交互，避免业务模块间的直接类型依赖和循环引用。

```
common/
├── shared-types/             ← 跨模块共享类型的完整定义（按语义群组拆分为12个子文件）
│   ├── README.md             ←   总览索引 + 数据流总览 + 模块关系总览
│   ├── inbound-message.md    ←   NormalizedMessage、MediaRef、MessageType
│   ├── content-block.md      ←   ContentBlock（7种变体）、ContentDelta、ContentBlockType
│   ├── dsl-parse-result.md   ←   DslParseResult、DslInstruction
│   ├── processed-message.md  ←   ProcessedMessage
│   ├── slash-result.md       ←   SlashResult（10种变体）、SideEffectContext
│   ├── llm-response.md       ←   UnifiedResponse、UnifiedUsage
│   ├── session-state.md      ←   PlanState、SessionCheckpoint、SessionStatus、PersistResult
│   ├── prompt-fragment.md    ←   FragmentContext、PromptFragment、BootstrapMode、SectionType
│   ├── runtime-config.md     ←   CompactConfig、ReasoningLevel、VerbosityLevel、PromptOverrides
│   ├── pending-message.md    ←   PendingMessage
│   └── rendered-output.md    ←   RenderedOutput、StreamingOutput
├── core-traits.md         ← 核心 trait 的接口定义（PromptFragmentProvider、ToolRegistrar、ToolRegistry、Tool trait、IMPlugin 等）
├── data-flow.md           ← 共享类型在全系统中的流动路径总览
```

## 数据流

common 本身不参与运行时数据流。它定义的数据结构在业务模块间传递，trait 接口在依赖注入时绑定实现。各共享类型的全系统流动路径总览见 [data-flow](data-flow.md)，各类型详细定义、数据流和模块关系见 [shared-types/README.md](shared-types/README.md) 及下属各子文件。

## 模块关系

- **上游**：无（common 不依赖任何其他模块，是纯定义基底层）
- **下游**：所有业务模块（通过引用 common 中定义的类型和 trait 进行交互）
- **无关**：无（common 与所有模块都有关联，不存在无关关系）
- **子文件**：[shared-types/](shared-types/README.md)（共享类型索引 + 12 个子类型文档）、[core-traits](core-traits.md)（PromptFragmentProvider、ToolRegistrar、ToolRegistry、Tool trait、IMPlugin 等）、[data-flow](data-flow.md)（共享类型全系统流动路径总览）

### 代码映射

设计文档中的 common 模块对应代码中的 `common` crate（未来拆分为 `common-types` + `common-traits`）。

**边界规则**：common crate 中定义的 pub trait 和 pub struct 必须已在本文档对应的 `core-traits.md` 或 `shared-types/` 下属文档中唯一定义。反之亦然——已在 common 设计文档中定义的类型和 trait，代码中必须位于 common crate（或其子 crate）。

若代码中 common crate 存在未在设计文档中定义的类型或 trait → 代码放错了，应移至对应领域模块的 crate，不是文档缺了。完整规则见 [STANDARDS.md](../STANDARDS.md)「crate 结构跟随文档」节。
