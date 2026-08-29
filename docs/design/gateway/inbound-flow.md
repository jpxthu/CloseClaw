# 入站流程

## 概述

入站流程处理从 IM 平台到 LLM 的完整消息链路。消息先进入 Gateway 入站消息队列缓冲，出队列后依次经过 IM 插件格式解析 → Processor Chain 消息变换 → Gateway 路由决策。

## 架构

1. 多个 IM 平台事件到达 Gateway，进入入站消息队列（有界持久化缓冲，默认 256）。
   - 队列满 → 拒收 + 回复"服务繁忙，请稍后重试"。
   - 消息入队即持久化，重启时重放未完成消息（详见 [Gateway README](README.md#消息队列与排队语义)）。
2. IM 插件解析平台格式（媒体已落盘为引用，不可得媒体记入 unavailable_media）→ NormalizedMessage { platform, sender_id, peer_id, reply_ref?, account_id, content, message_type, media_refs, unavailable_media, timestamp }。
3. Processor Chain 入站按 priority 升序执行：
   - RawLog（priority 10）：日志记录，透传（仅在 raw_log_dir 配置时注册）。
   - SessionRouter（priority 20）：计算 session_key = {timestamp_ms}-{hash}（算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session_key-算法)），写入 metadata，不创建 session。
   - ContentNormalizer（priority 30）：文本标准化（去除控制字符和 ANSI 转义序列、压缩连续空行、去行尾空格）。
4. 产出 [ProcessedMessage](../common/shared-types.md#processedmessage)（content_blocks + metadata { session_key, message_type, unavailable_media }——session_key 由 SessionRouter 写入，message_type 与 unavailable_media 由链调度环节在进链时从 NormalizedMessage 复制）。
5. Gateway 处理：
   - 含媒体消息 → 媒体可得性校验：不可得 → 提示「该消息内容无法获取」→ 简化出站，流程结束；可得 → 按类型构造上下文形态（图片进内容、文件音频以媒体引用）后与文本消息同链路继续（形态规则见 [im_adapter media-store](../im_adapter/media-store.md)）
   - 对话消息（文本消息及媒体可得消息）→ 从 metadata 取出 session_key；session_key 为空 → 记录 warning 日志，仍通过路由字段（platform, sender_id, peer_id, account_id）继续。
   - Gateway 根据配置定义的机器人→Agent 绑定确定对应的 Agent，得到 agent_id。
   - SessionManager 执行 session 查找/创建（传入 agent_id + session_key 和消息路由字段；SessionManager 内部提取稳定路由键做查找）→ 获得 session_id。
   - 按 content_blocks[0] 标准化文本前缀判断：
     - 以 `/` 开头 → 先拦截 `/approve-once`、`/approve-whitelist`、`/deny`（不进 SlashDispatcher，非 Owner 调用直接回复"权限不足"）；其余斜杠 → SlashDispatcher（不进入 LLM）。
     - 不以 `/` 开头 → Session → LLM → ContentBlock[]（进入出站）。

### 关键设计

- **斜杠指令在 Gateway 层统一拦截**，不进入 LLM 对话循环。拦截与路由逻辑见 [Gateway README](README.md#入站路径) 路由决策节。斜杠指令消息不追加到对话历史。
- **SessionRouter 不区分私聊和群聊**。会话粒度由插件控制——插件决定什么构成一个 `peer_id`。
- **SessionRouter 是纯变换**。只计算 session_key，不创建 session、不查 SessionManager。session_key 算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session_key-算法)。Session 的创建和查找由 Gateway 调用 SessionManager 完成。
- **session_key 为追踪标识，不参与路由**。详见 [Gateway README](README.md#数据流) 入站路径 Session 解析节。
- **Processor Chain 是纯变换**。每个处理器输入消息、输出消息，不做副作用（除了 RawLog 写日志）。链的设计遵循"变换和决策分离"原则——变换归链，决策归 Gateway。

## 数据流

### 前置：入站消息队列

入站消息先进入 Gateway 的入站消息队列。队列属性（边界、持久化、满行为、重启行为）见 [Gateway README](README.md#消息队列与排队语义)。

### IM 插件解析

IM 平台（飞书、Discord、Telegram 等）的平台事件出队列后，由对应平台的插件处理。插件把平台原生格式转成统一结构 `NormalizedMessage`（完整字段定义见 [common 共享类型](../common/shared-types.md)）。插件屏蔽了平台差异，Gateway 和 Processor Chain 看到的是统一的 NormalizedMessage。入站链路中参与处理的关键字段为：platform、sender_id、peer_id、account_id、content、message_type。`reply_ref?` 为可选字段（出站定向引用），入站仅透传、不参与路由计算，经 Session 上下文存储后供出站定向投递。message_type 由 IM 插件从平台消息类型映射并写入 NormalizedMessage（链调度环节将 message_type 复制到 ProcessedMessage 的 metadata，unavailable_media 同样复制，供 Gateway 做媒体可得性判断）。ContentNormalizer 对非 text 消息跳过标准化；Gateway 用 message_type 做分型路由。media_refs 在入站链路仅透传（媒体已在插件解析阶段落盘为本地引用，见 [im_adapter media-store](../im_adapter/media-store.md)），上下文形态决策由 Gateway 在路由阶段完成。

消息过滤：按 [common 共享类型](../common/shared-types.md) 的消息过滤规则执行——text 类型空 content 消息在解析阶段丢弃；post 类型 content 与 media_refs 均为空时同样丢弃；其余消息正常产 NormalizedMessage，由 Gateway 分型路由处理。

### Processor Chain 处理

NormalizedMessage 进入入站 Processor Chain。链按 priority 升序依次执行处理器（RawLog 仅在 `raw_log_dir` 配置时注册，未配置时链仅含 SessionRouter 和 ContentNormalizer 两个处理器）。

**RawLogProcessor（priority 10）**：将原始消息写入日志，用于审计和调试。消息内容不变，透传。

**SessionRouter（priority 20）**：计算 session 路由键。

- 输入：NormalizedMessage 的 `platform`、`sender_id`、`peer_id`、`account_id`，以及 SessionRouter 取的当前系统时间 `timestamp_ms`（毫秒）
- 计算：session_key 算法详见 [processor_chain 入站链路](../processor_chain/inbound-chain.md#session_key-算法)
- 输出：将 `session_key` 写入 metadata
- SessionRouter 不创建 session、不查 SessionManager——仅计算 session key

**ContentNormalizer（priority 30）**：对消息内容做平台无关的文本标准化。去除控制字符和 ANSI 转义序列，压缩连续空行，去行尾空格。不负责 Markdown 格式处理——URL 补全、代码块语言标签、富文本展开等均由各 IM 插件在解析阶段完成。非 text 消息（image/file/audio/post）跳过标准化，直接透传。

链输出 [ProcessedMessage](../common/shared-types.md#processedmessage)（`content_blocks` 含标准化后文本 + `metadata` 含 `session_key` 和 `message_type`）。ContentNormalizer 保留 metadata 不变，下游 Gateway 从 metadata 取出 session_key 和 message_type。

### Gateway 路由

Gateway 先按 message_type 做媒体可得性校验——含媒体消息（image/file/audio 或含内嵌媒体的 post）且媒体不可得（unavailable_media 非空，即下载失败或超出大小上限）时，向用户提示「该消息内容无法获取」，经简化出站路径发送（详见 [Gateway README](README.md#入站路径) 消息分型路由），不过 Session 和 LLM，流程结束。媒体可得时按类型构造上下文形态（图片进对话内容、文件音频以媒体引用，形态规则见 [im_adapter media-store](../im_adapter/media-store.md)），与文本消息同链路继续。

若 message_type 为 text，Gateway 从 content_blocks[0] 取标准化文本做前缀判断，同时从 metadata 取出 `session_key`。session_key 的降级处理（为空时仍通过路由字段继续）与路由语义（不参与 session 路由）见 [Gateway README](README.md#数据流) Session 解析节。Gateway 根据配置定义的机器人→Agent 绑定确定对应的 Agent，得到 agent_id，将 agent_id 连同 session_key、路由字段传给 SessionManager 做 session 查找/创建，获得 `session_id`。

Gateway 检查 content 第一个字符：

**以 `/` 开头 → 斜杠指令**：消息不进入 LLM，不追加到对话历史。先拦截 `/approve-once`、`/approve-whitelist`、`/deny`（拦截逻辑与权限校验见 [Gateway README](README.md#入站路径) 路由决策节）。其余斜杠指令分派给 SlashDispatcher，匹配指令 → 执行对应 Handler → 返回 [SlashResult](../common/shared-types.md#slashresult) → Gateway 执行副作用。

**不以 `/` 开头 → 普通对话消息**：Gateway 通过 `session_id` 找到 Session。后续流程（忙碌队列、归档恢复、进入 LLM）详见 [Gateway README](README.md#入站路径) 路由决策节。

## 模块关系

- **上游**：IM 插件（各平台 Adapter，产 NormalizedMessage）
- **下游**：Processor Chain 入站（调度链执行）、SessionManager（session 查找/创建/恢复）、Session（普通消息通过 `session_id` 追加到对话历史，Session 由 SessionManager 管理，入站流程不直接创建/销毁）、SlashDispatcher（斜杠指令分派）、Permission（/approve-once、/approve-whitelist、/deny 审批流程验证）
- **无关**：LLM Provider（通过 Session 间接调用）、System Prompt（由 Session 构建，入站流程不参与）、Tools（由 Session 注册和调用）
