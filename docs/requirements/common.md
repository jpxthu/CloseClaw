# Common 需求

## 概述

Common 是跨模块共享的纯数据结构和接口契约定义层。模块内定义的共享类型（如 NormalizedMessage、ContentBlock）和核心 trait（如 IMPlugin、ToolRegistry）均为模块间传递的中间结构或依赖注入接口，不对应用户可感知的独立功能。

Common 的用户需求均在各自归属的业务模块需求文档中完整定义。本模块需求文档仅作说明用途。

Common 的设计文档作者应关注：common 中每个概念的用户需求归属到哪个业务模块，并在设计时确保该概念的定义与归属模块的需求文档和设计文档保持一致。

## 功能需求

Common 无独立的用户功能需求。

## 关联设计文档

- [✓] common/README.md
- [✓] common/shared-types.md
- [✓] common/core-traits.md
- [✓] common/data-flow.md

## 非功能需求

Common 是纯定义层，不承载运行时行为，无非功能需求。
