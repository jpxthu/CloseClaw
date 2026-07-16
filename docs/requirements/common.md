# Common 需求

## 概述

Common 是跨模块共享的纯数据结构与接口契约定义层。Common 中定义的共享类型（如 NormalizedMessage、ContentBlock）和核心 trait（如 IMPlugin、ToolRegistry）均为模块间传递的中间结构或依赖注入接口，无独立功能。

Common 涉及的用户功能需求均在各归属业务模块的需求文档中完整定义。本文档仅作说明性记录。

以下说明供设计 Common 时参考：设计文档作者应识别 Common 中每个概念所归属的业务模块，并据此撰写业务模块引用说明；在设计 Common 中该概念时，需确保该概念的定义与归属的业务模块的需求文档和设计文档保持一致。

## 功能需求

Common 无独立的用户功能需求。

## 关联设计文档

- [✓] common/README.md
- [✓] common/shared-types.md
- [✓] common/core-traits.md
- [✓] common/data-flow.md

## 非功能需求

Common 是纯定义层，不承载运行时行为，无非功能需求。
