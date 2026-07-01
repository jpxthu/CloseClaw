# 核心 Trait

## 概述

核心 trait 是跨模块依赖注入的接口契约。每个 trait 在概念上归属一个具体领域模块（在对应模块的设计文档中完整定义），但在 common 中索引供多方消费，避免业务模块间的直接类型依赖。

## 架构

core-traits 不定义 trait 本身——trait 的完整定义在其归属的领域模块文档中。本文件是跨模块 DI trait 的总索引，每条记录标注 trait 名称、归属领域和定义位置。

### 已索引 Trait

| Trait | 归属领域 | 用途 | 完整定义 |
|-------|----------|------|----------|
| PromptFragmentProvider | system_prompt | 统一抽象 system prompt 静态层各数据来源（bootstrap、tools、skills、memory） | [system_prompt/fragment-provider](../system_prompt/fragment-provider.md) |
| ToolRegistrar | tools | 统一抽象各模块向 ToolRegistry 注册工具的能力，解耦 tools 对 skills/session/im_adapter 的直接调用 | [tools/tool-registrar](../tools/tool-registrar.md) |

## 数据流

core-traits 本身不参与运行时数据流。trait 接口在依赖注入时绑定实现，各业务模块通过 trait 接口交互而非直接依赖实现模块。

## 模块关系

- **上游**：无（core-traits 是纯索引，不产生数据）
- **下游**：所有引用这些 trait 的业务模块
- **无关**：无（本文件为纯索引，不参与运行时交互）
