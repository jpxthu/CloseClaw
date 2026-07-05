# PendingMessage

## 概述

PendingMessage 是待发送消息的排队结构，用于 Gateway 出站链路中消息发送的排队管理。当 IM Adapter 发送受限或需要异步发送时，消息进入等待队列以 PendingMessage 形式暂存。

> **本文档定义的 PendingMessage 在 common crate 中实现。引用本模块的下游文档通过 [ProcessedMessage](processed-message.md)、[RenderedOutput](rendered-output.md) 等链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### PendingMessage

> **文档编写中** — PendingMessage 的具体字段定义待 Gateway 出站消息队列实现方案确定后细化。

## 数据流

> **文档编写中** — PendingMessage 的排队流转路径待消息队列实现方案确定后补充。

## 模块关系

- **生产者**：Gateway（出站链路中的消息发送队列管理）
- **消费者**：IM Adapter（从消息队列中取出并发送）
- **无关**：Processor Chain、LLM Provider、SlashDispatcher
