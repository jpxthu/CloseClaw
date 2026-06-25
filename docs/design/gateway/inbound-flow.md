# 入站流程

## 概述

入站流程处理从 IM 平台到 LLM 的完整消息链路。一条用户消息依次经过三个模块：IM 插件格式解析 → Processor Chain 消息变换 → Gateway 路由决策。

## 架构

```
webhook → webhook → webhook → ...（高并发）
  ↓
[Gateway 入站消息队列]
  有界缓冲 → 满则拒 + 回复"服务繁忙"。重启清空，消息由 IM 平台 webhook 重试补偿
  ↓
[IM 插件]
  平台格式解析 → NormalizedMessage { platform, sender_id, peer_id, thread_id?, account_id, content, timestamp }
  ↓
[Processor Chain 入站]
  RawLog(10)        → 日志记录 → 透传
    ↓
  SessionRouter(20) → session_key = {timestamp}-{hash}（算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session-key-算法)）
                    → 写入 metadata，不创建 session
    ↓
  ContentNormalizer(30) → 文本标准化（去除控制字符和 ANSI 转义序列、压缩空行、去尾空格）
  ↓
ProcessedMessage { content, metadata { session_key } }
  ↓
[Gateway]
  → SessionManager.resolve(session_key) → 获得 session_id
  → content 以 / 开头？
    ├─ 是 → 先拦截 /approve、/deny（不进 SlashDispatcher）
    │       其余斜杠 → SlashDispatcher（不进入 LLM）
    └─ 否 → Session → LLM
                       ↓
                  ContentBlock[]（进入出站）
```

### 关键设计

- **斜杠指令在 Gateway 层统一拦截**，不进入 LLM 对话循环。斜杠指令走完整的入站链（保证日志记录和 session_id 获取），在 Gateway 层被识别后先拦截 `/approve`、`/deny`（owner 专用，走审批流程验证），其余斜杠分派给 SlashDispatcher。斜杠指令消息不追加到对话历史。
- **SessionRouter 不区分私聊和群聊**。会话粒度由插件控制——插件决定什么构成一个 `peer_id`。
- **SessionRouter 是纯变换**。只计算 session_key，不创建 session、不查数据库。session_key 算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session-key-算法)。Session 的创建和查找由 Gateway 调用 SessionManager 完成。
- **Processor Chain 是纯变换**。每个处理器输入消息、输出消息，不做副作用（除了 RawLog 写日志）。链的设计遵循"变换和决策分离"原则——变换归链，决策归 Gateway。

## 数据流

### 第零步：入站消息队列

高并发入站时 webhook 消息先进入 Gateway 的入站消息队列。队列属性（边界、持久化、满行为、重启行为）见 [Gateway README](README.md#消息队列与排队语义)。

### 第一步：IM 插件解析

IM 平台（飞书、Discord、Telegram 等）的 webhook 消息出队列后，由对应平台的插件处理。插件把平台原生格式转成统一结构 `NormalizedMessage`：

| 字段 | 来源 | 说明 |
|------|------|------|
| `platform` | 插件内置 | 平台标识，如 `"feishu"` |
| `sender_id` | webhook payload | 发送者的平台内 ID |
| `peer_id` | webhook payload | 会话对端（群聊 chat_id 或私聊对方 ID） |
| `thread_id` | webhook payload | 话题 ID，可选。不参与 session key 计算，仅用于出站定向回复 |
| `account_id` | 身份映射 | CloseClaw 本地账号标识，由 sender_id 通过身份映射得到。身份映射由 IM 插件在解析阶段完成 |
| `content` | webhook payload | 消息文本内容 |
| `timestamp` | IM 插件设置 | 消息到达时间（毫秒级 Unix 时间戳），用于 session_key 计算 |

插件屏蔽了平台差异。Gateway 和 Processor Chain 看到的是统一的 NormalizedMessage。

> NormalizedMessage 的完整字段契约由 [IM Adapter 模块](../im_adapter/README.md) 定义（含 message_type、media_refs、quoted_message 等），上表仅列出入站链路中参与处理的关键字段。

消息过滤：空 content 和非文本消息由插件在解析阶段过滤，不产 NormalizedMessage。

### 第二步：Processor Chain 处理

NormalizedMessage 进入入站 Processor Chain。链按 priority 升序依次执行三个处理器。

**RawLogProcessor（priority 10）**：将原始消息写入日志，用于审计和调试。消息内容不变，透传。

**SessionRouter（priority 20）**：计算 session 路由键。

- 输入：NormalizedMessage 的 `platform`、`sender_id`、`peer_id`、`account_id`、`timestamp`
- 计算：session_key 算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session-key-算法)
- 输出：将 `session_key` 写入 metadata
- SessionRouter 不创建 session、不查 SessionManager——仅计算 session key

**ContentNormalizer（priority 30）**：对消息内容做平台无关的文本标准化。去除控制字符和 ANSI 转义序列，压缩连续空行，去行尾空格。不负责 Markdown 格式处理——URL 补全、代码块语言标签、富文本展开等均由各 IM 插件在解析阶段完成。

链输出入站 `ProcessedMessage`（`content` 标准化后文本 + `metadata` 含 `session_key`）。ContentNormalizer 保留 metadata 不变，下游 Gateway 从 metadata 取出 session_key。

### 第三步：Gateway 路由

Gateway 从 metadata 取出 `session_key`。若 session_key 为空（SessionRouter 计算失败），Gateway 回复"会话路由失败"，消息不进入 LLM。

非空时，调用 `SessionManager.resolve(session_key)` 获得 `session_id`。

获得 session_id 后，Gateway 检查 content 第一个字符：

**以 `/` 开头 → 斜杠指令**：消息不进入 LLM，不追加到对话历史。Gateway 将 session_id 传给 SlashDispatcher 作为执行上下文（权限校验依赖）。先拦截 `/approve`、`/deny`（owner 专用，走审批流程验证），其余斜杠指令分派给 SlashDispatcher。SlashDispatcher 匹配指令 → 执行对应 Handler → 返回 SlashResult → Gateway 执行副作用。

**不以 `/` 开头 → 普通对话消息**：Gateway 通过 `session_id` 找到 Session（状态已在 `resolve()` 中处理完毕），消息追加到对话历史。Session 构建完整 LLM 请求（system prompt + 消息历史 + 工具列表 + skill 列表）。LLM 返回 `ContentBlock[]`，进入出站链路。

## 模块关系

- **上游**：IM 插件（各平台 Adapter，产 NormalizedMessage）
- **下游**：Processor Chain 入站（调度链执行）、SessionManager（session 查找/创建/恢复）、SlashDispatcher（斜杠指令分派）
- **间接关联**：Session（通过 SessionManager 获取和操作）、LLM Provider（通过 Session 间接调用，入站流程不直接接触）、System Prompt（由 Session 构建，入站流程不参与）、Permission（斜杠指令高危操作执行前校验）、Tools（由 Session 注册和调用，入站流程不直接接触）
