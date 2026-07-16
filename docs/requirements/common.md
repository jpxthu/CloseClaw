# Common 需求

## 概述

Common 模块定义跨模块共享的纯数据结构和接口契约，是业务模块间的依赖基础层。这些结构和契约本身不对应用户可感知的功能——例如 NormalizedMessage 是模块间传递的数据结构，它的用户可感知功能是"多平台消息归一化"，由 im_adapter 模块承载。

因此，Common 不定义独立的用户功能需求。它的每个概念的用户需求，均在各自归属的业务模块需求文档中完整定义。Common 的需求文档仅作说明用途，不包含功能域。

## 功能需求

Common 无独立的用户功能需求。其定义的概念按性质分为两类：

- **共享类型**（shared-types）：跨模块传递的纯数据结构。用户需求由消费这些数据的业务模块承载——消息格式归一化归属 im_adapter，消息路由归属 gateway，System Prompt 构建归属 system_prompt，工具发现归属 tools 等。
- **核心 trait**（core-traits）：跨模块依赖注入的接口契约。用户需求由实现这些 trait 的业务模块承载——平台适配归属 im_adapter，工具注册归属 tools，System Prompt 片段提供归属 system_prompt 等。

各概念与业务模块的归属关系，以设计文档中的模块职责划分为准。

## 关联设计文档

- [✓] common/README.md
- [✓] common/shared-types.md
- [✓] common/core-traits.md
- [✓] common/data-flow.md

## 非功能需求

Common 是纯定义层，不承载运行时行为，无非功能需求。
