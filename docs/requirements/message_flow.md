# Message Flow 需求

## 概述

描述一次用户消息在 CloseClaw 中从发送到 Agent 回复的完整产品链路——覆盖入站（IM → Agent）与出站（Agent → IM）两个方向，串联 im_adapter、gateway、session、permission、config 等模块。

本文档为流程级概述，各环节的详细定义与权威说明见对应模块需求文档。

## 入站链路

1. 用户在 IM 平台（飞书等）发送消息。
2. IM 平台事件到达平台插件，平台特有字段被解析。
3. 消息被归一化为统一格式（平台、发送者、会话对端、账号、消息正文）。
4. **账号标识**通过发送者解析为 CloseClaw User——走「IM 用户 → User」入站方向。
5. 空消息丢弃，文本消息进入对话流程。
6. 按「机器人 → Agent」入站方向，由接收方 bot 解析出路由目标 Agent。
7. 在该 Agent 范围内按四项标识（平台 + 发送者 + 会话对端 + 账号）共同确定 Session：复用未归档 / 等待归档 / 恢复已归档 / 创建新。
8. LLM 对话：System Prompt 组装 + 历史 + 新消息 → 流式输出。

## 出站链路

9. 流式片段按格式选择渲染（纯文本 / 富格式，飞书渲染为卡片）。
10. 出站统一处理（频率限制、审计、错误降级），发完后追加到 Session 历史。
11. 按「Agent → 机器人」出站方向，由 Agent 解析出 bot，确定发送通道。
12. 消息经平台发送回到用户端；话题型消息携带话题标识定向回到对应 Session。

## 跨 bot 双接收场景

同一群消息被两个 bot 各自接收时，IM 平台按接收方应用语境给出不同的用户标识——必须由身份映射归并为同一 CloseClaw User，否则两边各开 Session。

跨应用身份映射的双向特性：

- **入站**：任一接收方 bot 的用户标识都能解析回同一个 User。
- **出站**：同一个 User 能解析到各 bot 应用语境下的用户标识（用于选 bot、@ 解析）。

实测证据见飞书 CLI fixture：`tests/fixtures/feishu/cli-poc/group-mention-user-b1view.json` 与 `group-mention-user-b2view.json`——同一群消息被两个 bot 各自接收，两份事件中 `sender_id` / `mentions[].id` 按接收方应用语境各自不同。

## 基数

- 单个 IM 平台下，IM 用户 ↔ User、机器人 ↔ Agent 各自一一对应。
- 单个 CloseClaw User 可对应多个 IM 平台的用户；单个 CloseClaw Agent 可对应多个 IM 平台的机器人。

双向绑定的权威定义、入站/出站方向划分与基数规则的存储与生效机制详见 [config §F1](config.md)（多文件配置结构）。

## 非功能需求

- 完整入站出站链路任意环节异常时，系统降级或回滚的边界由各模块文档负责说明；本文档不重复定义。

> **交叉引用**：本文档中各环节的权威定义详见 [im_adapter §F2](im_adapter.md)（入站消息归一化）、[im_adapter/feishu §F1](im_adapter/feishu.md)（飞书入站消息接收）、[gateway §F4](gateway.md)（普通消息路由到对话）、[gateway §F7](gateway.md)（出站消息统一处理）、[session §F1](session.md)（对话持久化与恢复）、[permission §F1](permission.md)（身份体系）、[config §F1](config.md)（多文件配置结构）。