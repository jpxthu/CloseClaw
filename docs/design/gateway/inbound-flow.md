# 入站流程

## 概述

入站流程处理从 IM 平台到 LLM 的完整消息链路。消息先进入 Gateway 入站消息队列缓冲，出队列后依次经过 IM 插件格式解析 → Processor Chain 消息变换 → Gateway 路由决策。

## 架构

```
webhook → webhook → webhook → ...（高并发）
  ↓
[Gateway 入站消息队列]
  有界缓冲（默认 256）→ 满则拒 + 回复"服务繁忙，请稍后重试"。重启清空，消息由 IM 平台 webhook 重试补偿
  ↓
[IM 插件]
  平台格式解析 → NormalizedMessage { platform, sender_id, peer_id, thread_id?, account_id, content, message_type, media_refs, timestamp }
  ↓
[Processor Chain 入站]
  RawLog（priority 10）        → 日志记录 → 透传（仅在 raw_log_dir 配置时注册）
    ↓
  SessionRouter（priority 20） → session_key = {timestamp_ms}-{hash}（算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session_key-算法)）
                               → 写入 metadata，不创建 session
    ↓
  ContentNormalizer（priority 30） → 文本标准化（去除控制字符和 ANSI 转义序列、压缩连续空行、去行尾空格）
  ↓
[ProcessedMessage](../common/shared-types.md#processedmessage)（content_blocks + metadata { session_key, message_type }）
  ↓
[Gateway]
  → message_type 非 text（image/file/audio）？
    ├─ 是 → 构造错误回复 ContentBlock[] → 简化出站（详见 [Gateway README](README.md#入站路径) 非文本消息处理）
    └─ 否（text）→ 从 metadata 取出 session_key
                    → session_key 为空？
                      ├─ 是 → 记录 warning 日志，仍通过路由字段（platform, sender_id, peer_id, account_id）继续
                      └─ 否 → 正常流转
                    → SessionManager 执行 resolve（传入 session_key 和消息路由字段；SessionManager 内部提取稳定路由键做查找）→ 获得 session_id  ← 日志：session 查找结果
                    → content 以 / 开头？  ← 日志：路由决策结果
                        ├─ 是 → 先拦截 /approve-once、/approve-whitelist、/deny（不进 SlashDispatcher，非 Owner 调用直接回复"权限不足"）
                        │       其余斜杠 → SlashDispatcher（不进入 LLM）
                        └─ 否 → Session → LLM
                                           ↓
                                      ContentBlock[]（进入出站）
```

### 关键设计

- **斜杠指令在 Gateway 层统一拦截**，不进入 LLM 对话循环。拦截与路由逻辑见 [Gateway README](README.md#入站路径) 路由决策节。斜杠指令消息不追加到对话历史。
- **SessionRouter 不区分私聊和群聊**。会话粒度由插件控制——插件决定什么构成一个 `peer_id`。
- **SessionRouter 是纯变换**。只计算 session_key，不创建 session、不查数据库。session_key 算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session_key-算法)。Session 的创建和查找由 Gateway 调用 SessionManager 完成。
- **session_key 为追踪标识，不参与路由**。详见 [Gateway README](README.md#数据流) 入站路径 Session 解析节。
- **Processor Chain 是纯变换**。每个处理器输入消息、输出消息，不做副作用（除了 RawLog 写日志）。链的设计遵循"变换和决策分离"原则——变换归链，决策归 Gateway。

## 数据流

### 前置：入站消息队列

入站消息先进入 Gateway 的入站消息队列。队列属性（边界、持久化、满行为、重启行为）见 [Gateway README](README.md#消息队列与排队语义)。

### IM 插件解析

IM 平台（飞书、Discord、Telegram 等）的 webhook 消息出队列后，由对应平台的插件处理。插件把平台原生格式转成统一结构 `NormalizedMessage`（完整字段定义见 [common 共享类型](../common/shared-types.md)）。插件屏蔽了平台差异，Gateway 和 Processor Chain 看到的是统一的 NormalizedMessage。入站链路中参与处理的关键字段为：platform、sender_id、peer_id、account_id、content、message_type、timestamp。`thread_id?` 为可选字段，入站仅透传、不参与路由计算。message_type 由 IM 插件从平台消息类型映射并写入 NormalizedMessage（链框架将其复制到 ProcessedMessage 的 metadata）。ContentNormalizer 对非文本消息跳过标准化，Gateway 用 message_type 做非文本拦截。media_refs 当前在入站链路无实际消费者，为多模态支持预留。

消息过滤：text 类型空 content 消息在解析阶段丢弃，不产 NormalizedMessage。非文本消息（image/file/audio）正常产 NormalizedMessage（message_type 标记类型，media_refs 存储引用，content 可为空），由下游 Gateway 统一处理。

### Processor Chain 处理

NormalizedMessage 进入入站 Processor Chain。链按 priority 升序依次执行处理器（RawLog 仅在 `raw_log_dir` 配置时注册，未配置时链仅含 SessionRouter 和 ContentNormalizer 两个处理器）。

**RawLogProcessor（priority 10）**：将原始消息写入日志，用于审计和调试。消息内容不变，透传。

**SessionRouter（priority 20）**：计算 session 路由键。

- 输入：NormalizedMessage 的 `platform`、`sender_id`、`peer_id`、`account_id`，以及 SessionRouter 取的当前系统时间 `timestamp_ms`（毫秒）
- 计算：session_key 算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session_key-算法)
- 输出：将 `session_key` 写入 metadata
- SessionRouter 不创建 session、不查 SessionManager——仅计算 session key

**ContentNormalizer（priority 30）**：对消息内容做平台无关的文本标准化。去除控制字符和 ANSI 转义序列，压缩连续空行，去行尾空格。不负责 Markdown 格式处理——URL 补全、代码块语言标签、富文本展开等均由各 IM 插件在解析阶段完成。非文本消息（image/file/audio）跳过标准化，直接透传。

链输出 [ProcessedMessage](../common/shared-types.md#processedmessage)（`content_blocks` 含标准化后文本 + `metadata` 含 `session_key` 和 `message_type`）。ContentNormalizer 保留 metadata 不变，下游 Gateway 从 metadata 取出 session_key 和 message_type。

### Gateway 路由

Gateway 先检查消息的 message_type——若为非文本（image/file/audio），直接构造"暂不支持该消息类型"的错误回复（ContentBlock[]），经简化出站路径发送（详见 [Gateway README](README.md#入站路径) 非文本消息处理），不过 Session 和 LLM。流程到此结束。

若 message_type 为 text，Gateway 从 content_blocks[0] 取标准化文本做前缀判断，同时从 metadata 取出 `session_key`。session_key 的降级处理（为空时仍通过路由字段继续）与路由语义（不参与 session 路由）见 [Gateway README](README.md#数据流) Session 解析节。Gateway 将 session_key 连同路由字段传给 SessionManager 做 session 查找/创建，获得 `session_id`。

Gateway 检查 content 第一个字符：

**以 `/` 开头 → 斜杠指令**：消息不进入 LLM，不追加到对话历史。先拦截 `/approve-once`、`/approve-whitelist`、`/deny`（拦截逻辑与权限校验见 [Gateway README](README.md#入站路径) 路由决策节）。其余斜杠指令分派给 SlashDispatcher，匹配指令 → 执行对应 Handler → 返回 [SlashResult](../common/shared-types.md#slashresult) → Gateway 执行副作用。

**不以 `/` 开头 → 普通对话消息**：Gateway 通过 `session_id` 找到 Session（状态已在 `resolve()` 中处理完毕），消息追加到对话历史。Session 构建完整 LLM 请求（system prompt + 消息历史 + 工具列表 + skill 列表）。LLM 返回 `ContentBlock[]`，进入出站链路。

## 模块关系

- **上游**：IM 插件（各平台 Adapter，产 NormalizedMessage）
- **下游**：Processor Chain 入站（调度链执行）、SessionManager（session 查找/创建/恢复）、Session（普通消息通过 `session_id` 追加到对话历史，Session 由 SessionManager 管理，入站流程不直接创建/销毁）、SlashDispatcher（斜杠指令分派）、Permission（/approve-once、/approve-whitelist、/deny 审批流程验证）
- **无关**：LLM Provider（通过 Session 间接调用）、System Prompt（由 Session 构建，入站流程不参与）、Tools（由 Session 注册和调用）
