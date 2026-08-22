# Gateway 需求

## 概述

Gateway 是消息路由中枢。User 通过不同 IM 平台发送的消息由 Gateway 统一接入、识别类型、按规则路由到斜杠指令或 LLM 对话，并将所有回复经统一流程发回对应平台。

## 功能需求

### F1. 多平台消息统一接入

- User 可以通过飞书、Discord、Telegram、CLI 等多种平台发送消息，系统对不同平台的消息采用统一的处理方式，平台差异被透明消除

### F2. 入站消息预处理

- 空文本消息在接入阶段丢弃，不进入后续处理

> **交叉引用**：入站消息文本标准化由 Processor Chain 模块负责，详见 [processor_chain §F3](processor_chain.md)（文本内容标准化）。

### F3. 消息类型识别与非文本处理

- 系统能识别文本、图片、文件、音频等多种消息类型
- 系统对非文本消息回复“暂不支持该消息类型”提示后结束处理，不进入 LLM 对话

### F4. 普通消息路由到对话

- 经类型识别后且不以 `/` 开头的文本消息进入 LLM 对话流程
- 消息路由到接收该消息的机器人所绑定的 Agent——机器人与 Agent 的绑定关系由配置定义
- 在命中 Agent 后，Session 查找、创建与归档恢复由 Session 模块负责（含向 User 展示的提示语），Gateway 仅传入路由字段，命中 Session 后进入下一步路由决策

> **交叉引用**：会话查找、创建与归档恢复详见 [session §F1](session.md)（对话持久化与恢复）。
- Session 忙闲判定与消息排队行为由 Session 模块管理，详见 [session §F10](session.md)（消息排队）
- Session 查找异常或创建失败时，系统向 User 回复错误提示

### F5. 斜杠指令拦截与分派

- User 可以用 `/` 开头发送斜杠指令，指令在进入 LLM 对话之前被拦截，不追加到对话历史
- 非 Owner 调用 `/approve-once`、`/approve-whitelist`、`/deny` 时收到权限不足提示

> **交叉引用**：审批指令的 Owner 专用语义详见 [slash §F13](slash.md)（审批指令）。
- Immediate 类指令绕过排队条件，立即受理；审批指令的结果在 Owner 审批完成后送达

> **交叉引用**：各指令的 Immediate 标记由 Slash 模块逐指令定义，详见 [slash §F1](slash.md)（斜杠指令入口）。审批指令的等待与回调机制详见 [permission §F5](permission.md)（审批工作流）。
- 非 Immediate 斜杠指令在满足排队条件时进入该 Session 的待处理队列，排队提示详见 [session §F10](session.md)（消息排队）

排队条件定义详见 [session §F10](session.md)（消息排队），活跃维度详见 [session §F11](session.md)（Session 活跃维度）。

### F6. 入站消息队列与过载保护

- 高并发入站消息按到达顺序排队处理
- 队列满时拒绝新消息，立即向 User 回复“服务繁忙，请稍后重试”
- 系统重启后，未完成处理的消息不会丢失，被拒绝的消息由对应平台自动重发

### F7. 出站消息统一处理

- LLM 回复和斜杠指令回复均走同一条出站消息处理流程

> **交叉引用**：出站消息按目标平台的要求展示，格式自动选择由 IM Adapter 模块负责，详见 [im_adapter §F3](im_adapter.md)（出站消息格式自动选择）。
- 出站消息发送前经过频率限制、敏感操作审计等检查，被拦截的消息不发送
- 回复过程中出错时统一降级处理，不向 User 呈现不完整内容
- LLM 回复和斜杠指令回复发送后保存到 Session 历史记录；排队提示、错误提示等系统提示不保存
- 流式回复中断时，保存已送达 User 的部分

### F8. 调试日志

Gateway 在以下环节记录调试日志，用于运维 Agent 排查问题：
- 入站消息到达与队列操作（不含原始消息内容）
- 路由决策结果（斜杠指令识别 / 普通对话分发 / 排队状态）
- 频率限制、审计等拦截

> **交叉引用**：入站消息原始内容日志由 Processor Chain 模块负责，详见 [processor_chain §F6](processor_chain.md)（调试日志）。
> **交叉引用**：Session 查找与生命周期事件日志由 Session 模块负责，详见 [session §F12](session.md)（调试日志）。
> **交叉引用**：出站渲染与平台 API 发送结果日志由 IM Adapter 模块负责，详见 [im_adapter §F7](im_adapter.md)（调试日志）。

> **交叉引用**：日志框架定义（格式、级别、追踪标识、存储轮转、隐私脱敏）详见 [debug_log](debug_log.md)（调试日志）。

## 关联设计文档

- [✓] gateway/README.md
- [✓] gateway/inbound-flow.md
- [✓] gateway/outbound-flow.md

## 非功能需求

- 入站消息队列满时，"服务繁忙"拒绝响应必须在 2 秒内送达 User
- User 发送文本消息后应在 1 秒内收到系统响应或排队提示；等待 Owner 审批等异步等待的结果不适用此约束
- 系统重启后，重启前发送但未完成处理的消息不应丢失
