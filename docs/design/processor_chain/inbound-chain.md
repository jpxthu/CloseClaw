# 入站链路

## 概述

入站 Processor 链在所有 IM 平台的归一化之后运行。各 IM Adapter 的入站部分先将平台特定格式转为 NormalizedMessage，链再统一处理日志、会话路由计算和内容清洗。

Processor 链是纯变换链——只做内容计算和 metadata 填充，不管理 session 生命周期、不做路由决策。链路不感知平台差异，平台特有的解析和适配逻辑由 IM Adapter 负责。

## 架构

```
IM 平台消息到达
  ↓
IM Adapter（入站）— 平台特定解析，产 NormalizedMessage
  ↓
NormalizedMessage（统一中间结构）
  ┌─────────────────────────────────┐
  │ platform    — 来源平台标识       │
  │ sender_id   — 发送者标识         │
  │ peer_id     — 会话对端标识       │
  │ thread_id   — 话题标识（可选）    │
  │ account_id  — CloseClaw 本地账号标识│
  │ content     — 消息文本内容        │
  │ timestamp   — 消息时间           │
  └─────────────────────────────────┘
  ↓
Processor 链（按 priority 升序执行，纯变换）
  ├── RawLogProcessor（priority 10）
  │     → 原始消息写入日志，透传
  │
  ├── SessionRouter（priority 20）
  │     → 从 (platform, sender_id, peer_id, account_id) 计算 session_key
  │     → session_key 写入 metadata
  │     → 不创建 session、不查 SessionManager
  │
  └── ContentNormalizer（priority 30）
        → 清洗平台残留元数据（飞书 at 语法、Discord mention 等）
        → 富文本展开为标准 markdown
        → 标准化格式：压缩连续空行、去行尾空格、裸 URL 补全 https:// 前缀
  ↓
ProcessedMessage → Gateway 路由
```

### Session key 算法

SessionRouter 计算 session_key 的方式：

- `session_key = hash(platform, sender_id, peer_id, account_id)`
- `account_id` 由 `sender_id` 通过身份映射得到，是 CloseClaw 本地的账号标识。一个 CloseClaw 账号可绑定多个平台的 sender_id
- `thread_id` 不参与——仅用于出站时 Gateway 定向回复到正确的话题线

session_key 是一个确定性哈希值，相同的输入永远产相同的 key。但 session_key 不直接等于 session_id——它只是一个查找键。

Gateway 拿到 session_key 后，调用 SessionManager 的 key registry 查表获得最新的 session_id。`/new` 指令在同输入下创建新 session 时，registry 覆盖为新的 session_id，旧 session 自然脱离路由。

SessionRouter 不区分私聊和群聊。会话粒度由 Adapter 通过 peer_id 的构造方式控制——Adapter 决定什么构成一个"会话对端"，Session 机制本身是通用的。

### 异常处理

链路整体采用 fail-open 策略：任何 Processor 异常不阻塞消息流，回退到透传原文。

| 场景 | 处理 |
|------|------|
| RawLog 写入失败 | 记录错误日志，消息流程不受影响 |
| SessionRouter 计算 key 失败 | 记录告警日志，session_key 留空，消息继续流转 |
| ContentNormalizer 异常 | 记录告警日志，丢弃变换结果，透传原始 content |
| IM Adapter 解析失败 | 由 Adapter 自身处理（不产 NormalizedMessage 即丢弃消息） |

## 数据流

```
IM Adapter 产出 NormalizedMessage { platform, sender_id, peer_id, thread_id?, account_id, content }
  → RawLogProcessor：记录原始内容到日志 → 透传
    → SessionRouter：计算 session_key = hash(platform, sender_id, peer_id, account_id) → 写入 metadata.session_key
      → ContentNormalizer：清洗平台残留 → 标准化 markdown 格式
        → ProcessedMessage { content, metadata { session_key } }
          → Gateway
            → 调用 SessionManager.resolve(session_key) 获得 session_id
            → 路由决策（/ 开头 → 斜杠指令；否则 → LLM 对话）
```

关键判断点：
- SessionRouter 计算失败时 session_key 留空，Gateway 收到无 key 消息时回复用户"会话路由失败，请重试"
- ContentNormalizer 异常时内容不变（丢回原文），消息继续流转
- 所有异常均记录日志，用于后续问题定位和改进

## 模块关系

- **上游**：IM Adapter（各平台提供适配器，产 NormalizedMessage）
- **下游**：Gateway（接收 ProcessedMessage，消费 metadata.session_key 做路由决策）、Session 模块（SessionRouter 计算的 session_key 经 Gateway 传递给 SessionManager 做 session 路由查找，属数据流下游依赖）
- **链内**：
  - RawLogProcessor — 审计日志（副作用），不改内容
  - SessionRouter — 计算 session_key（纯哈希计算），写 metadata
  - ContentNormalizer — 内容清洗 + 格式标准化（纯文本变换）
- **无关**：出站 Processor 链（独立链路，与入站互不干扰）
