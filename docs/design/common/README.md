# Common

## 概述

- 关联需求文档：[requirements/common.md](../../requirements/common.md)
- 一句话：common 是跨模块共享概念的唯一定义地，包含跨模块传递的纯数据结构（共享类型）与依赖注入接口（核心 trait）；各模块文档通过引用指向此处，不在自身文档中重复定义。

## 架构

common 不是业务模块——它以跨模块共享的数据结构与接口契约为核心，不含业务逻辑。消费 common 的模块通过其共享类型和 trait 进行解耦交互，避免业务模块间的直接类型依赖和循环引用。common 不含业务执行逻辑——可执行工具函数、调度器、互斥机制等属领域逻辑，应在各自领域 crate 实现。

```
common/
├── shared-types.md        ← 跨模块传递的纯数据结构的完整定义（共享类型权威清单）
├── core-traits.md         ← 核心 trait 的接口定义（跨模块 DI trait 全集，按领域分组）
├── data-flow.md           ← 共享类型主链路（入站/出站/跨方向）方向级流动总览
```

## 数据流

common 本身不参与运行时数据流。它定义的数据结构在业务模块间传递，trait 接口在依赖注入时绑定实现。共享类型的主要入站/出站/跨方向流动路径总览见 [data-flow](data-flow.md)，详细流动路径（字段级、判断分支、渲染差异）见 [shared-types](shared-types.md)。

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

- **上游**：无（common 不依赖任何其他模块，是纯定义基底层）
- **下游**：所有消费 common 中类型或 trait 的模块（通过引用 common 中定义的类型和 trait 进行交互）——完整清单见 [core-traits §模块关系](core-traits.md#模块关系)（system_prompt / tools / session / skills / agent / tasks / memory / im_adapter / gateway / cli / slash / permission / processor_chain / daemon / config / llm）
- **无关**：无。platform、debug_log、fake_llm 不依赖 common（不作为下游），也无与「跨模块共享定义」名称/功能易混的关系
- **子文件**：[shared-types](shared-types.md)（共享类型的完整定义与数据流）、[core-traits](core-traits.md)（跨模块 DI trait 全集，按领域分组）、[data-flow](data-flow.md)（共享类型主链路方向级流动总览）。共享类型的权威清单见 `shared-types.md`

### 代码映射

设计文档中的 common 模块对应代码中的 `common` crate。

**边界规则**：common crate 中定义的 pub 类型（struct / enum / type 别名）与 pub trait 必须已在本文档对应的 `core-traits.md` 或 `shared-types.md` 中唯一定义（比对口径见 [STANDARDS.md](../STANDARDS.md)「common 准入清单」）。反之亦然——已在 common 设计文档中定义的类型和 trait，代码中必须位于 common crate（或其子 crate）。common 是共享概念的**唯一访问路径**，其他 crate 不得把 common 的项再导出到自己的公共 API（见 [STANDARDS.md](../STANDARDS.md)「禁止二次出口」）。

若代码中 common crate 存在未在设计文档中定义的类型或 trait：先按 [STANDARDS.md](../STANDARDS.md)「common 文档内容准入标准」判定归属——满足准入条件（被 2+ 模块消费的共享类型 / DI trait）属文档缺口，补进 common 文档；不满足准入条件（单模块类型/trait）才移至对应领域模块的 crate。完整规则见 [STANDARDS.md](../STANDARDS.md)「crate 结构跟随文档」节。
