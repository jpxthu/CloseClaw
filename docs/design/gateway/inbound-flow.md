# 入站流程

## 概述

入站流程处理从 IM 平台到 LLM 的完整消息链路。一条用户消息依次经过三个模块：IM Adapter 的格式解析 → Processor Chain 的消息变换 → Gateway 的路由决策。

## 完整流程

### 第一步：IM Adapter 解析

各 IM 平台（飞书、Discord、Telegram 等）的 webhook 到达后，由对应平台的 Adapter 处理。Adapter 的唯一职责是把平台原生格式转成统一结构 `NormalizedMessage`：

| 字段 | 来源 | 说明 |
|------|------|------|
| `platform` | Adapter 内置 | 平台标识，如 `"feishu"` |
| `sender_id` | webhook payload | 发送者的平台内 ID |
| `peer_id` | webhook payload | 会话对端（群聊 chat_id 或私聊对方 ID） |
| `thread_id` | webhook payload | 话题 ID，可选。**不参与 session key 计算**，仅用于出站时定向回复到正确话题 |
| `account_id` | webhook context | 租户/账号标识（飞书的 tenant_key 等），可选。用于多租户 session 隔离 |
| `content` | webhook payload | 消息文本内容 |
| `timestamp` | webhook payload | 消息发送时间，用于日志和审计 |

Adapter 屏蔽了平台差异。Gateway 和 Processor Chain 看到的是统一的 NormalizedMessage，不需要知道消息来自哪个平台。

> **消息过滤**：空 content 和非文本消息（图片、文件、语音等）由 IM Adapter 在解析阶段过滤，不产 NormalizedMessage。非文本消息的后续处理方案留待 IM 模块设计时确定。

### 第二步：Processor Chain 处理

NormalizedMessage 进入入站 Processor Chain。链按 priority 升序依次执行四个处理器，每个接手前一个的输出，做变换后传给下一个。

**RawLogProcessor（priority 10）**：将原始消息写入日志，用于审计和调试。消息内容不变，直接透传给下一步。

**SessionRouter（priority 20）**：这是最关键的一步。它的工作是确定"这条消息属于哪个会话"。

- 输入：NormalizedMessage 的 `platform`、`sender_id`、`peer_id`，以及可选 `account_id`（由 DmScope 配置决定是否参与计算）
- 计算：`session_key = f(platform, sender_id, peer_id[, account_id])`，具体格式由 `DmScope` 配置决定（默认 `PerChannelPeer`）
- thread_id 不参与 session key 计算——话题归属由出站链路处理，与 session 路由是独立概念
- 查找或创建 session：去 SessionManager 问"这个 key 有没有活跃会话？"——有则复用，没有则新建（含 bootstrap 注入、system prompt 组装等）
- 输出：将 `session_id` 写入消息 metadata，传给下一步

SessionRouter 不关心消息内容是什么。它只看"谁、在哪个平台、在哪个会话"发来的，然后把它挂到正确的 session 上。这一步做完，后续所有处理都有 session 上下文可用。

**MessageCleaner（priority 30）**：清洗消息内容。Adapter 在解析时可能残留平台特有的格式标记（如飞书的 at 语法、Discord 的 mention 格式），这一步把它们清除。如果有富文本内容，展开为标准 markdown。输入和输出都是文本字符串。

**MarkdownNormalizer（priority 40）**：标准化 markdown 格式。做三件事——压缩连续空行为单个空行、去掉行尾空格、给裸 URL 补上 `https://` 前缀。确保进入 LLM 的文本格式干净统一。

链处理完毕后，返回 `ProcessedMessage`，包含两个字段：
- `content`：清洗并标准化后的纯文本
- `metadata`：至少包含 `session_id`（由 SessionRouter 写入）

### 第三步：Gateway 路由

ProcessedMessage 到达 Gateway。Gateway 检查 content 的第一个字符：

**以 `/` 开头 → 斜杠指令**：
- 消息不进入 LLM。Gateway 将指令名和参数交给 SlashDispatcher。
- 标记为 Immediate 的指令（`/stop`、`/status`、`/help`）可绕过消息队列立即执行。
- 非 Immediate 指令走 Handler → 返回 SlashResult → Gateway 执行副作用（模式切换、会话管理、权限检查等）。

**不以 `/` 开头 → 普通对话消息**：
- Gateway 通过 `metadata.session_id` 找到对应 Session
- 若 Session 处于 archived 状态，由 SessionManager 触发 restore 流程，Gateway 向用户发送「正在恢复会话...」通知
- Session 就绪后将消息内容追加到对话历史
- Session 构建完整的 LLM 请求（system prompt + 消息历史 + 工具列表 + skill 列表）
- LLM 返回响应，封装为 `ContentBlock[]`
- ContentBlock[] 进入出站链路，最终渲染发送回用户

## 关键设计决策

1. **斜杠指令走完整的入站链**，但不进入 LLM。这样设计保证了：(a) 所有消息都有日志记录；(b) 斜杠指令也能拿到 session 上下文（`/stop`、`/compact`、`/mode` 都依赖 session_id）；(c) 路由逻辑简单——只在 Gateway 层做一次分支判断。

2. **SessionRouter 不区分私聊和群聊**。会话粒度由 Adapter 控制——Adapter 决定什么构成一个 `peer_id`。Session 机制本身对公私聊无感。

3. **Processor Chain 是纯变换**。每个处理器输入消息、输出消息，不做副作用（除了 RawLog 写日志）。SessionRouter 创建 session 是找 SessionManager，不是自己管理。链的设计遵循"变换和决策分离"原则——变换归链，决策归 Gateway。

## 数据流向图

```
webhook
  ↓
[IM Adapter]
  平台格式解析 → NormalizedMessage { platform, sender_id, peer_id, content, ... }
  ↓
[Processor Chain 入站]
  RawLog(10)         → 日志记录 → 透传
    ↓
  SessionRouter(20)  → session_key = f(platform, sender_id, peer_id)
                    → 查找/创建 session → session_id 写入 metadata
    ↓
  MessageCleaner(30) → 清洗残留元数据，富文本 → markdown
    ↓
  MarkdownNormalizer(40) → 压缩空行、去行尾空格、补全 URL 前缀
  ↓
ProcessedMessage { content, metadata { session_id, ... } }
  ↓
[Gateway]
  content 以 / 开头？
    ├─ 是 → SlashDispatcher（不进入 LLM）
    └─ 否 → SessionManager → Session
                ├─ archived → restore + 发送恢复通知
                └─ active → 直接使用
              → LLM → ContentBlock[]（进入出站）
```
