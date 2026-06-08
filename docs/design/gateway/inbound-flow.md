# 入站流程

## 概述

入站流程处理从 IM 平台到 LLM 的完整消息链路。一条用户消息依次经过三个模块：IM 插件格式解析 → Processor Chain 消息变换 → Gateway 路由决策。

## 架构

```
webhook
  ↓
[IM 插件]
  平台格式解析 → NormalizedMessage { platform, sender_id, peer_id, thread_id?, account_id?, content, timestamp }
  ↓
[Processor Chain 入站]
  RawLog(10)        → 日志记录 → 透传
    ↓
  SessionRouter(20) → session_key = hash(platform, sender_id, peer_id[, account_id])
                    → 写入 metadata，不创建 session
    ↓
  ContentNormalizer(30) → 清洗平台残留 → 标准化 markdown 格式
  ↓
ProcessedMessage { content, metadata { session_key } }
  ↓
[Gateway]
  → SessionManager.resolve(session_key) → 获得 session_id
  → content 以 / 开头？
    ├─ 是 → SlashDispatcher（不进入 LLM）
    └─ 否 → Session → LLM → ContentBlock[]（进入出站）
```

### 关键设计

- **斜杠指令走完整的入站链**，但不进入 LLM。这样设计保证了：(a) 所有消息都有日志记录；(b) 斜杠指令也能拿到 session 上下文（`/stop`、`/compact`、`/mode` 都依赖 session_id）；(c) 路由逻辑简单——只在 Gateway 层做一次分支判断。
- **SessionRouter 不区分私聊和群聊**。会话粒度由插件控制——插件决定什么构成一个 `peer_id`。
- **SessionRouter 是纯变换**。只计算 session_key（确定性哈希），不创建 session、不查数据库。Session 的创建和查找由 Gateway 调用 SessionManager 完成。
- **Processor Chain 是纯变换**。每个处理器输入消息、输出消息，不做副作用（除了 RawLog 写日志）。链的设计遵循"变换和决策分离"原则——变换归链，决策归 Gateway。

## 数据流

### 第一步：IM 插件解析

各 IM 平台（飞书、Discord、Telegram 等）的 webhook 到达后，由对应平台的插件处理。插件把平台原生格式转成统一结构 `NormalizedMessage`：

| 字段 | 来源 | 说明 |
|------|------|------|
| `platform` | 插件内置 | 平台标识，如 `"feishu"` |
| `sender_id` | webhook payload | 发送者的平台内 ID |
| `peer_id` | webhook payload | 会话对端（群聊 chat_id 或私聊对方 ID） |
| `thread_id` | webhook payload | 话题 ID，可选。不参与 session key 计算，仅用于出站定向回复 |
| `account_id` | webhook context | 租户标识，可选。用于多租户 session 隔离 |
| `content` | webhook payload | 消息文本内容 |
| `timestamp` | webhook payload | 消息发送时间 |

插件屏蔽了平台差异。Gateway 和 Processor Chain 看到的是统一的 NormalizedMessage。

消息过滤：空 content 和非文本消息由插件在解析阶段过滤，不产 NormalizedMessage。

### 第二步：Processor Chain 处理

NormalizedMessage 进入入站 Processor Chain。链按 priority 升序依次执行三个处理器。

**RawLogProcessor（priority 10）**：将原始消息写入日志，用于审计和调试。消息内容不变，透传。

**SessionRouter（priority 20）**：计算 session 路由键。

- 输入：NormalizedMessage 的 `platform`、`sender_id`、`peer_id`，以及可选 `account_id`（由 DmScope 配置决定是否参与计算）
- 计算：`session_key = hash(platform, sender_id, peer_id[, account_id])`。相同输入产相同 key
- `thread_id` 不参与 session key 计算——话题回复与主会话共享同一 session。`thread_id` 仅用于出站时定向回复
- 输出：将 `session_key` 写入 metadata
- SessionRouter 不创建 session、不查 SessionManager——仅做哈希计算

**ContentNormalizer（priority 30）**：清洗并标准化消息内容。清洗平台特有的格式标记（飞书 at 语法、Discord mention 等），富文本展开为标准 markdown；标准化格式（压缩连续空行、去行尾空格、裸 URL 补 `https://` 前缀）。

链输出 `ProcessedMessage`（`content` 清洗后文本 + `metadata` 含 `session_key`）。

### 第三步：Gateway 路由

Gateway 从 metadata 取出 `session_key`，调用 `SessionManager.resolve(session_key)` 获得 `session_id`。

`resolve()` 内部逻辑：
- 查 key_registry 映射表
- 未命中 → 创建新 session → 写入映射 → 返回 session_id
- 命中且 active → 直接返回 session_id
- 命中且 archived → 触发 restore（Gateway 发送「正在恢复会话...」通知）→ 返回 session_id

key_registry 在 daemon 启动时由 SessionManager 遍历 SQLite 中所有 session（active + archived）重建。

若 session_key 为空（SessionRouter 计算失败），Gateway 回复"会话路由失败"，消息不进入 LLM。

获得 session_id 后，Gateway 检查 content 第一个字符：

**以 `/` 开头 → 斜杠指令**：消息不进入 LLM。Gateway 交给 SlashDispatcher。Immediate 指令（如 `/stop`、`/status`、`/help` 等）可绕过消息队列立即执行；非 Immediate 指令走 Handler → SlashResult → Gateway 执行副作用。

**不以 `/` 开头 → 普通对话消息**：Gateway 通过 `session_id` 找到 Session（状态已在 `resolve()` 中处理完毕），消息追加到对话历史。Session 构建完整 LLM 请求（system prompt + 消息历史 + 工具列表 + skill 列表）。LLM 返回 `ContentBlock[]`，进入出站链路。

## 模块关系

- **上游**：IM 插件（各平台 Adapter，产 NormalizedMessage）
- **下游**：Processor Chain 入站（调度链执行）、SessionManager（session 查找/创建/恢复）、SlashDispatcher（斜杠指令分派）
- **间接关联**：Session（通过 SessionManager 获取和操作）、Permission（斜杠指令高危操作执行前校验）
- **无关**：LLM Provider（不直接调用）、System Prompt（不参与构建）、Tools（不注册和执行工具调用）
