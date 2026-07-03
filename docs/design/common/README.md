# Common

## 概述

common 是跨模块共享概念的唯一定义地。包含两类内容：跨模块传递的纯数据结构（共享类型）和依赖注入接口（核心 trait）。各业务模块文档通过引用指向此处，不在自身文档中重复定义。

## 架构

common 不是业务模块——它不含可执行逻辑，只定义数据结构和接口契约。所有业务模块通过 common 中的共享类型和 trait 进行解耦交互，避免业务模块间的直接类型依赖。

```
common/
├── shared-types.md        ← 跨模块传递的纯数据结构的完整定义
├── core-traits.md         ← 核心 trait 的接口定义
├── data-flow.md           ← 共享类型在全系统中的流动路径总览
```

## 数据流

common 本身不参与运行时数据流。它定义的数据结构在业务模块间传递，trait 接口在依赖注入时绑定实现。各共享类型的全系统流动路径总览见 [data-flow](data-flow.md)，详细流动路径（字段级、判断分支、渲染差异）见 [shared-types](shared-types.md)。

## 模块关系

- **上游**：无（common 不依赖任何其他模块，是纯定义基底层）
- **下游**：所有业务模块（通过引用 common 中定义的类型和 trait 进行交互）
- **无关**：无（common 与所有模块都有关联，不存在无关关系）
- **子文件**：[shared-types](shared-types.md)（NormalizedMessage、ContentBlock、DslParseResult / DslInstruction、ProcessedMessage、SlashResult）、[core-traits](core-traits.md)（PromptFragmentProvider、ToolRegistrar）、[data-flow](data-flow.md)（共享类型全系统流动路径总览）
