# SlashResult

## 概述

SlashResult 是斜杠指令 Handler 返回的执行结果类型。每个变体封装一种指令的副作用逻辑。Handler 返回 SlashResult 后，由 Gateway 构造 SideEffectContext 并触发 SlashResult 执行，各变体自行完成对应的 session 操作和消息回复。

> **本文档定义的 SlashResult、SideEffectContext 在 common crate 中实现。引用本模块的下游文档通过 [ContentBlock](content-block.md)、[ProcessedMessage](processed-message.md) 等链接引用这些类型定义，不在自身模块的文档或代码中重复实现。**

## 架构

### SlashResult

SlashResult 共 10 种变体：

| 变体 | 用途 | 产出 |
|------|------|------|
| SetMode | 设置会话运行模式（Normal/Plan） | ContentBlock::Text（确认信息） |
| SetReasoning | 设置推理深度 | ContentBlock::Text（确认信息） |
| SetVerbosity | 设置信息展示等级 | ContentBlock::Text（确认信息） |
| Reply | 纯文本回复，用于 /help、/status 等仅需回复文本的指令 | ContentBlock::Text（回复文本） |
| NewSession | 创建新会话 | ContentBlock::Text（确认信息） |
| Stop | 终止当前运行（含级联终止子 session） | ContentBlock::Text（确认信息） |
| Compact | 触发对话历史压缩 | ContentBlock::Text（压缩结果） |
| SystemAppend | 向 system prompt 追加内容 | ContentBlock::Text（确认信息） |
| Exec | 执行系统命令（高危操作，执行前经 Permission 模块校验） | ContentBlock[]（命令输出经出站 Processor Chain） |
| Unknown | 未知指令回退 | ContentBlock::Text（提示信息） |

**执行模型**：Gateway 不感知具体 SlashResult 变体。Handler 返回 SlashResult 后，Gateway 统一调用执行方法，由各变体自行完成副作用。新增指令只需新增 SlashResult 变体及其执行实现，Gateway 无需改动。

### SideEffectContext

SideEffectContext 是 Gateway 在收到 SlashResult 后构造的执行上下文。携带当前 Session 的操作能力（用于模式切换、会话创建/停止、压缩等操作）和回复通道（用于产出回复内容）。SideEffectContext 由 Gateway 管理，SlashResult 不持有其引用。

**与 ContentBlock[] 的关系**：SlashResult 各变体在执行中通过 SideEffectContext 的回复通道产出 ContentBlock[]，进入出站 Processor Chain——与 LLM 的 UnifiedResponse 走同一条出站处理路径（VerbosityFilter → DslParser → OutboundRawLog → IM Adapter 渲染发送）。

## 数据流

SlashResult 的执行流程：

1. Gateway 将 / 开头的消息路由到 SlashDispatcher
2. SlashDispatcher 解析指令名和参数，查找对应 Handler
3. Handler 处理完成后返回 SlashResult 变体
4. Gateway 构造 SideEffectContext，触发 SlashResult 执行
5. Exec 变体：Gateway 调用 Permission 模块校验命令权限（校验通过方继续执行，拒绝则返回权限错误）
6. SlashResult 变体通过 SideEffectContext 完成副作用，分两条路径：
   - 回复路径：产出 ContentBlock[] → 出站 Processor Chain → IM Adapter 渲染发送
   - 会话路径：执行 Session 操作（模式切换、创建、停止、压缩等）

SlashResult 的生命周期：Handler 返回 → Gateway 构造 SideEffectContext 并触发执行 → 各变体通过 SideEffectContext 完成副作用后销毁。

## 模块关系

- **生产者**：SlashDispatcher（各 Handler 返回 SlashResult 变体）
- **消费者**：Gateway（构造 SideEffectContext 并触发 SlashResult 执行，回复内容进入出站 Processor Chain）
- **间接消费者**：Permission 模块（Exec 变体执行前校验）、CLI（通过 Gateway 间接消费斜杠指令回复）
- **无关**：LLM Provider（不参与斜杠指令，不接触 SlashResult）、Processor Chain 入站（斜杠指令不进入站 Processor Chain）、Session（SlashResult 通过 SideEffectContext 操作 Session，但 Session 不直接消费 SlashResult 结构）
