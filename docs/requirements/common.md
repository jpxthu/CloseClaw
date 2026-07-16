# Common 需求

## 概述

Common 是跨模块共享的纯数据结构和接口契约层，不承载独立的用户需求。其定义的所有类型和 trait 的用户可感知价值，已在各自归属的业务模块需求文档中完整表达。

本文档作为反向索引——将 common 中定义的每个概念映射到其用户需求所在的模块，供设计文档和代码开发者查找。

## 功能需求

Common 无独立的用户功能需求。以下按 common 设计文档中的定义列出各概念的需求归属。

### 共享类型

common 定义的共享类型是模块间传递的数据结构。用户不直接接触这些类型——用户需求体现为这些类型所支撑的功能：

| 类型 | 用户可感知功能 | 需求归属 |
|------|--------------|---------|
| NormalizedMessage | 跨平台消息格式统一 | [im_adapter §F2](im_adapter.md)（入站消息归一化） |
| ContentBlock | Agent 输出的结构化内容渲染 | [im_adapter §F3](im_adapter.md)（出站消息格式自动选择）、[im_adapter §F4](im_adapter.md)（流式增量渲染） |
| ProcessedMessage | 消息出入站的统一处理 | [gateway §F2](gateway.md)（入站消息预处理）、[gateway §F7](gateway.md)（出站消息统一处理） |
| DslParseResult / DslInstruction | 消息中交互元素的渲染 | [im_adapter §F3](im_adapter.md)（出站消息格式自动选择） |
| RenderedOutput | 平台原生格式消息 | [im_adapter §F6](im_adapter.md)（渲染与发送分离） |
| SlashResult | 斜杠指令的执行结果 | [gateway §F5](gateway.md)（斜杠指令拦截与分派） |
| FragmentContext / PromptFragment | System Prompt 静态层的构建 | [system_prompt §F1](system_prompt.md)（身份与行为准则定义）、[system_prompt §F2](system_prompt.md)（工具与技能清单注入） |
| VerbosityLevel | Agent 回复的信息展示等级 | [session §F5](session.md)（LLM 交互控制） |
| PlanState | Plan Mode 下的规划状态 | mode（需求文档待创建） |

### 核心 trait

common 定义的核心 trait 是跨模块依赖注入的接口契约。用户需求体现为这些 trait 所支撑的模块能力：

| Trait | 用户可感知功能 | 需求归属 |
|------|--------------|---------|
| PromptFragmentProvider | System Prompt 各数据来源的统一抽象 | [system_prompt §F2](system_prompt.md)（工具与技能清单注入）、[system_prompt §F3](system_prompt.md)（长期记忆注入） |
| ToolRegistrar | 多模块向工具注册中心添加工具 | [tools §F9](tools.md)（工具扩展接入） |
| ToolRegistry | Agent 的工具发现与查询 | [tools §F1](tools.md)（工具注册与发现） |
| Tool trait | 工具的统一接口定义 | [tools §F1](tools.md)（工具注册与发现） |
| IMPlugin | 多平台插件化适配 | [im_adapter §F1](im_adapter.md)（多平台插件化适配） |

### 数据流

Common 定义的数据流是各共享类型在模块间的传递路径，属于内部实现约定，无独立的用户需求。相关流动方向已在各归属模块的需求中体现。

## 关联设计文档

- [✓] common/README.md
- [✓] common/shared-types.md
- [✓] common/core-traits.md
- [✓] common/data-flow.md

## 非功能需求

Common 是纯定义层，不承载运行时行为，无非功能需求。
