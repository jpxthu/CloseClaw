# Message Flow 需求

## 概述

本模块记录一条消息从用户发送到 Agent 回复送达的完整流转过程——覆盖入站（IM → Agent）与出站（Agent → IM）两个方向，串联 IM 平台接入、消息归一化、身份映射、消息路由、会话确定、LLM 对话、渲染与平台发送各环节，供理解完整产品流程时参考。

## 功能需求

### F1. 入站链路

一条用户消息从 IM 平台到达 Agent 的完整环节：

1. 用户在 IM 平台（飞书、Discord、Telegram 等）发送消息。
2. IM 平台事件到达平台插件，平台特有字段被解析。
3. 消息归一化为统一格式（平台标识、发送者、会话对端、账号标识、消息正文）。
4. **账号标识**由发送者经身份映射解析为 CloseClaw User，走「IM 用户 → User」入站方向。
5. 空消息丢弃、消息类型识别；不以 `/` 开头的文本消息进入 LLM 对话流程。
6. 消息路由到 Agent——走「机器人 → Agent」入站方向，由接收方 bot 解析出路由目标 Agent。
7. 在该 Agent 范围内按四项标识（平台标识、发送者、会话对端、账号标识）共同确定 Session：复用未归档 / 等待归档完成后恢复 / 恢复已归档 / 创建新。
8. LLM 对话：System Prompt 组装 + 历史 + 新消息 → 流式输出。

> **交叉引用**：步骤 2 的飞书实现详见 [im_adapter/feishu §F1](im_adapter/feishu.md)（飞书入站消息接收）。
> **交叉引用**：步骤 3 详见 [im_adapter §F2](im_adapter.md)（入站消息归一化）。
> **交叉引用**：步骤 5 详见 [gateway §F2](gateway.md)（入站消息预处理）、[gateway §F3](gateway.md)（消息类型识别与非文本处理）。
> **交叉引用**：步骤 6 详见 [gateway §F4](gateway.md)（普通消息路由到对话）。
> **交叉引用**：步骤 7 详见 [session §F1](session.md)（对话持久化与恢复）。
> **交叉引用**：步骤 4、6 所用绑定的定义详见 [config §F1](config.md)（多文件配置结构）。

### F2. 出站链路

Agent 回复从生成到送达用户的完整环节：

1. 流式输出按格式自动选择渲染：纯文本走轻量消息，含格式特征的内容走富格式（飞书渲染为卡片）。
2. 出站消息经统一处理（频率限制、敏感操作审计、错误降级），发送后追加到 Session 历史。
3. 选择承载发送的 bot——走「Agent → 机器人」出站方向。
4. 渲染与发送分离，经平台 API 发送。
5. 话题型消息携带话题标识，定向回复到对应 Session 的话题。

> **交叉引用**：步骤 1 详见 [im_adapter §F3](im_adapter.md)（出站消息格式自动选择）、[im_adapter §F4](im_adapter.md)（流式增量渲染）、[im_adapter/feishu §F4](im_adapter/feishu.md)（飞书卡片渲染）。
> **交叉引用**：步骤 2 详见 [gateway §F7](gateway.md)（出站消息统一处理）。
> **交叉引用**：步骤 3 所用绑定的定义详见 [config §F1](config.md)（多文件配置结构）。
> **交叉引用**：步骤 4 详见 [im_adapter §F6](im_adapter.md)（渲染与发送分离）、[im_adapter/feishu §F3](im_adapter/feishu.md)（飞书消息发送）。
> **交叉引用**：步骤 5 详见 [im_adapter/feishu §F7](im_adapter/feishu.md)（飞书话题与 Session 匹配）。

### F3. 跨 bot 双接收与身份归并

同一条群消息同时 @ 多个 bot 时，每个接收方 bot 各自收到一份平台事件；事件中的发送者标识与 @ 对象标识按接收方应用语境翻译，互不相同。这些标识必须由身份映射归并为同一个 CloseClaw User，否则同一用户会在不同 bot 下被识别为不同 User 并各自开启 Session。

跨应用身份映射是双向的：入站方向，任一接收方 bot 语境下的用户标识都解析回同一个 User；出站方向，同一个 User 可解析到各 bot 应用语境下的用户标识（用于选 bot、@ 解析）。

> **交叉引用**：身份映射与跨平台归并详见 [permission §F1](permission.md)（身份体系）。
> **交叉引用**：事件字段与跨应用 ID 隔离的实测语义见 `tests/fixtures/feishu/cli-poc/`——`group-mention-user-b1view.json` 与 `group-mention-user-b2view.json` 是同一条群消息被两个 bot 分别接收的事件（同 `message_id`，`sender_id` 与 `mentions[].id` 全部不同）；`p2p-top-text.json` 与 `p2p-thread-reply.json` 展示私聊顶层消息与话题回复的字段形态。

### F4. 绑定基数

单个 IM 平台下，机器人 ↔ Agent、IM 用户 ↔ User 各自一一对应；跨平台时，单个 CloseClaw Agent 可对应多个 IM 平台的机器人，单个 CloseClaw User 可对应多个 IM 平台的用户。

> **交叉引用**：双向绑定的权威定义、入站/出站方向划分、基数规则及其存储与生效机制详见 [config §F1](config.md)（多文件配置结构）。

## 非功能需求

各环节的可靠性、性能、安全等质量属性由对应模块需求文档定义，本模块不重复。
