# 产品文档写作标准

## 定位

设计文档是需求与代码之间的桥梁。它将 `docs/requirements/` 中的用户需求翻译为实现层的架构蓝图——从产品角度描述模块的职责、架构、运转方式、模块间交互。讲清楚产品**应该是什么样**，不讲历史、不讲现状问题、不讲设计取舍过程。

设计文档**必须遵从需求文档**。若发现更优的架构方案与需求文档期望行为冲突，必须提给 owner 裁决，不得自行修改需求文档或偏离需求设计。

本标准管设计文档。需求文档（`docs/requirements/`）、API 设计、测试用例、开发记录另立文档，不混入设计文档。

## 术语

- **上游**：调用本模块，或本模块消费其产出数据。含经 Gateway、消息队列等中介传递的数据流依赖
- **下游**：本模块调用，或本模块产出数据供其消费。含经中介传递的数据流依赖
- **数据流**：输入 → 处理 → 输出的完整路径

## 写作格式

善用 Markdown 富文本表达结构化信息，避免将多项并列数据写成长句平铺。

## 章节结构

固定 4 节，不增不减。**模块的所有子功能都写在独立的子功能文档中**，README.md 只保留模块级架构概览和目录索引。

### README.md — 模块索引 + 架构

```
# <模块名>

## 概述
- 关联需求文档：[requirements/xxx.md](../requirements/xxx.md)
- 一句话：这个模块做什么

## 架构
- 模块的整体框架结构、核心组件及其关系，用自然语言描述
- 可配 ASCII 图描述结构关系（用等宽字符，箭头用 -> / <-，层级缩进 2 空格）
- 列出所有子功能文档，作为目录索引（每个子功能一行，含一句话简述）

## 数据流
- 模块级别的输入 → 处理 → 输出完整路径
- 关键判断点和分支条件
- 可配 ASCII 流程图

## 模块关系
- 上游：谁调这个模块，或本模块消费谁产出的数据，传什么进来
- 下游：这个模块调谁，或谁消费本模块产出的数据，传什么出去
- 无关：无调用且无数据流关联、但名称或功能容易混淆的模块
- 共享类型：若传入/传出的数据被 2+ 模块共享，必须在 common 文档中定义，此处引用 `[common/xxx](common/xxx.md)`
```

### `<子功能>.md` — 子功能文档

与 README 同 4 节结构，聚焦子功能自身：

```
# <子功能名>

## 概述
- 一句话：这个子功能做什么

## 架构
- 子功能的核心机制和运转方式
- 可配 ASCII 图

## 数据流
- 子功能级别的输入 → 处理 → 输出路径

## 模块关系
- 与模块内其他子功能、跨模块组件的交互
```

## 文件组织

```
docs/design/
├── README.md                 ← 全库文档索引（每个模块一行，含一句话简述）
├── <模块名>/
│   ├── README.md             ← 模块索引 + 架构概览（4 节）
│   └── <子功能>.md           ← 子功能产品文档（4 节，每个功能独立成文）
```

规则：
- **所有子功能独立成文**，README.md 只做架构概览和目录索引，不内嵌子功能小节
- 目录结构尽量与 `src/` 下的模块目录对应
- 若产品名与目录名不一致，以产品名为文件夹名
- 跨模块主题放在主要受益模块下

## 跨模块类型与依赖设计

设计文档是代码的架构蓝图，文档描述的模块关系决定了 crate 间的依赖方向。

### 共享类型的定义地

被 2 个及以上模块共同消费的数据结构（如 `NormalizedMessage`、`ContentBlock`），必须在设计文档中指定**唯一的所有模块**。通常放在 `docs/design/common/` 下，其他模块文档引用之。

代码遵循文档：类型定义在哪个设计文档里，代码中就放在对应的 crate 里。

### 模块关系中的依赖方向

文档描述模块关系时，必须明确依赖方向：

- 上下游关系中，下游不应反过来定义上游消费的类型
- 业务模块之间避免直接依赖，跨模块交互通过 common 中定义的 trait 完成
- 若文档中发现两个业务模块相互依赖对方的类型定义，必须将共享类型提取到 common 文档中

### 依赖方向允许边表

「模块关系中的依赖方向」的约束落到 crate 层，即形成**依赖方向允许边表**，判定标准可脚本化。

**判定**：以 crate 为单位，其 workspace 内部依赖（依赖目标为本 workspace 成员 crate——根 crate `closeclaw` 或 `crates/` 下各 crate）必须落在下表对应行的允许集合内；表外任何内部依赖均为越界边，必须消除。本表只约束常规依赖声明（`dependencies`）；测试 / 构建期依赖（`dev-dependencies` / `build-dependencies`）不在此列（不影响发布产物的模块边界）。本表描述**目标态**。

| 层 | crate（目录名） | 允许依赖的 workspace crate |
|----|-------|--------------------------|
| 基础层 | `common`、`platform`、`debug_log` | 无 |
| 领域层 | `agent`、`config`、`im_adapter`、`llm`、`memory`、`permission`、`processor_chain`、`session`、`skills`、`slash`、`system_prompt`、`tools`、`workflow`、`execution`、`tasks` | `common`、`platform`、`debug_log` |
| 领域层（跨域桥接） | `gateway` | `common`、`platform`、`debug_log`、`session`、`llm`、`permission`、`config` |
| 组合根 | `daemon`、根 crate `closeclaw` | 全部 workspace crate |
| 入口客户端 | `cli` | `common`、`platform`、`debug_log`、`gateway`、`config`、`permission`、`llm`、`agent` |
| 测试基础设施 | `fake_llm` | 无（不引用任何 CloseClaw crate） |

判读：

- 模块关系中的「上游 / 下游」描述数据流与调用关系（含经 common trait 完成的调用），**不等于 crate 依赖**；crate 依赖以本表为准。
- 基础层（`common` 跨模块接口层、`platform` OS 抽象、`debug_log` 日志基础设施）不承载业务逻辑，任何其他层可依赖。
- 领域 crate（业务模块，含模块拆出的 `execution`、`tasks`）之间不直接互依，跨模块交互经 `common` 的 trait / 共享类型完成（依赖倒置，由组合根装配）。
- **`gateway` 的跨域桥接例外**：其文档登记为 `LlmCaller`、`SessionLookup`、`SlashSessionQuery`、`SlashEffectExecutor`、`PermissionChecker` 等 common trait 的具体提供方（桥接 session ↔ llm 的循环依赖、包装 PermissionEngine、读取配置绑定），故允许直接依赖 `session`、`llm`、`permission`、`config`；其余仅依赖基础层。
- 组合根运行时装配组件、注入依赖，可依赖全部 crate；其组件间调用不转化为组件所在 crate 的直接依赖（见 [daemon README](daemon/README.md) 组件依赖表）。依赖图（常规依赖）必须无环。
- `cli` 为客户端入口，除基础层外其允许的领域依赖见上表（取自 cli 模块各子文档「模块关系」登记的下游模块）；它经管理协议（socket）与 daemon 交互，不依赖 `daemon` crate。
- `fake_llm` 为黑盒测试基础设施，不引用任何 CloseClaw crate；产品领域 crate 不把它列入常规依赖，仅组合根（测试装配）使用。
- 未列入本表的 crate 默认适用领域层规则（仅基础层）。

### common 模块文档

`docs/design/common/` 是跨模块共享概念的唯一权威定义地：

```
docs/design/common/
├── README.md              ← 共享类型与核心 trait 的总索引
├── shared-types.md        ← 跨模块传递的纯数据结构的完整定义
├── core-traits.md         ← 核心 trait 的接口定义
└── data-flow.md           ← 共享类型在全系统中的流动路径
```

各业务模块文档引用 common 文档中的类型定义，不在自身文档中重复描述。

### common 文档内容准入标准

不是"被两个模块引用就放 common"。按跨模块消费范围分为三类：

| 类别 | 定义 | 放 common |
|------|------|-----------|
| 共享类型 | 被 2+ 模块消费的纯数据结构，无单一领域归属 | ✅ 完整定义在 common |
| DI trait | 被 2+ 模块实现或消费的依赖注入接口 | ✅ 完整定义在 common |
| 单模块类型/trait | 仅被一个模块定义和消费的配置/错误/内部类型 | ❌ 留在领域模块 |

**判断原则**：一个类型被哪个模块定义和理解。如果只有 gateway 模块理解 `HandleResult` 的含义，它就属于 gateway 文档——不应为了"共享"而入 common。

此标准同时约束代码层：文档中在 common 定义的类型和 trait，代码中位于 common crate；文档中在领域模块定义的，代码中位于对应领域 crate。无例外。

### common 准入清单（比对口径）

「common 文档内容准入标准」与「crate 结构跟随文档」已要求 common 文档与 common crate 双向对应；本节给出可脚本比对的**清单口径**：

- **范围**：common 准入清单取自 [shared-types](common/shared-types.md) 与 [core-traits](common/core-traits.md) 全文中的类型 / trait 条目（`shared-types.md` 声明整篇文档为共享类型的权威清单）。
- **条目承载**：作为类型 / trait 名出现的**标题**（各级）与**粗体条目**（列表项 `- **名称**：…` 或行首 `**名称**：…`）；分类分组标题、过程描述、字段 / 语义标签等含说明文字的粗体不构成条目。
- **比对键**：条目中的标识符；` / ` 或空白分隔者分别计入；类别修饰词（如 `trait`）不计；同名去重。
- **比对范围**：common crate 的 `pub` 类型（struct / enum / type 别名）与 `pub` trait 与清单一一对照（含 common 子 crate，若有）；机械包装别名（`Arc<…>` 形式的 `Shared…` 别名）、函数、常量、模块、测试相关模块内的项、再导出声明不参与比对。
- **缺失即偏差**：清单缺名（common 有 `pub` 项而清单无条目）或清单多名（清单有条目而 common 无 `pub` 项）均为偏差，按「crate 结构跟随文档」的归属判定处置。

### crate 结构跟随文档

`docs/design/<模块>/` 与 crate 的映射为一对一或一对多（模块拆多 crate 时），不允许多对一——两个设计文档模块的定义不应混入同一个 crate。

**common 的边界尤其严格**：common crate 中定义的 pub trait 和 pub struct 必须已在 `docs/design/common/` 的设计文档中唯一定义。反之亦然——已在 common 设计文档中定义的类型和 trait，代码中必须位于 common crate（或其子 crate）。

若代码中 common crate 存在未在设计文档中定义的类型或 trait：先按「common 文档内容准入标准」判定归属——
- 满足准入条件（被 2+ 模块消费的共享类型 / 被 2+ 模块实现或消费的 DI trait）：属文档缺口，应补进 common 文档，代码留在 common crate；
- 不满足准入条件（仅被单一模块定义和消费的类型/trait）：代码放错了，应移至对应领域模块的 crate。

### 禁止二次出口

`common` 是共享概念的**唯一**访问路径。除 `common` 自身外，任何 crate 不得把 `common` 的项经自己的**公共路径**再暴露——否则会出现第二接口层，消费方可绕过 `common` 直接耦合到某个 crate。本节仅约束 `common`（基础层 `platform` / `debug_log` 不在此列）。

**判定**（针对 `common` 之外的每个 crate，逐个判定其**直接**再暴露 common 项的行为；不追溯经其它 crate 的传递链）：

- **构成二次出口（禁止）**：使 `common` 的某个 `pub` 项（类型 / trait / 函数 / 常量 / 模块 / 宏）可从该 crate 的公共路径（`<crate>::…`，即自 crate 根经全部 `pub` 项链可达）访问到的声明——包括再导出语句（`pub use`，含通配 `*` 与 `as` 别名、`pub extern crate`），以及把 common 项命名为本 crate 类型别名的声明（`pub type <名称> = <common 项>`）。
- **不属于二次出口（允许）**：
  - 不可公共抵达的再导出——位于私有模块（非 `pub` 声明，含 `pub(crate) mod`）内、且未被任何公共路径再导出的 common 项再导出；
  - 私有 / `pub(crate)` 方式的 `use`（不对外暴露）；
  - 再导出**本 crate 自身定义**的项（`crate::…` / `self::…` 来源，且最终定义地确为本 crate）；
  - **使用** common 类型而非为其新增公共名称——公开 API 的签名、公开字段以 common 类型为材料（如 `pub struct Z(pub T)`）属正常使用。
- **不因「被公开 API 使用」而豁免**：某 crate 的公开 API 若在签名中使用 `common` 类型，属正常使用，消费方直接经 `common` 引用该类型；但不得据此把 common 项再导出或另立公共别名。

## 红线

**不写**：
- 第 5 节或任何超出 4 节的内容
- 借鉴来源（"参考 Claude Code..."）
- 现状问题（"当前代码中..."）
- 自问自答（"为什么选 A 不选 B"）
- 编号前缀、版本号等历史标记
- 函数名、变量名、代码片段
- 未来计划、开发步骤
- 验收标准
