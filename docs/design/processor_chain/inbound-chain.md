# 入站链路

## 概述

入站 Processor Chain在所有 IM 平台的归一化之后运行。各 IM Adapter 的入站部分先将平台特定格式转为 NormalizedMessage，链再统一处理日志、会话路由计算和文本标准化。

Processor Chain是纯变换链——只做内容计算和 metadata 填充，不管理 session 生命周期、不做路由决策。链路不感知平台差异，平台特有的解析和适配逻辑由 IM Adapter 负责。

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
Processor Chain（按 priority 升序执行，纯变换）
  ├── RawLogProcessor（priority 10）
  │     → 原始消息写入日志，透传
  │
  ├── SessionRouter（priority 20）
  │     → 计算 session_key（公式见 [Session key 算法](#session-key-算法)）
  │     → session_key 写入 metadata
  │     → 不创建 session、不查 SessionManager
  │
  └── ContentNormalizer（priority 30）
        → 文本标准化：去除控制字符和 ANSI 转义序列、压缩连续空行、去行尾空格
        → 非文本消息（image/file/audio）跳过标准化，直接透传
  ↓
ProcessedMessage
  ↓
Gateway 路由
```

### session_key 算法

SessionRouter 计算 session_key 的方式：

- `session_key = {timestamp_ms}-{hash}`，hash 由 `platform:sender_id:peer_id:account_id:timestamp_ms` 拼接后计算
- `account_id` 由 IM Adapter 在产 NormalizedMessage 时通过身份映射填入（`sender_id` → CloseClaw 本地账号标识），SessionRouter 直接消费。一个 CloseClaw 账号可绑定多个平台的 sender_id
- `timestamp_ms` 为 SessionRouter 取当前系统时间（毫秒级 Unix 时间戳），独立于 NormalizedMessage.timestamp
- `thread_id` 不参与——仅用于出站时 Gateway 定向回复到正确的话题线

session_key 是消息级标识，用于日志追踪和调试。它不直接参与 session 路由——Gateway 调用 SessionManager 后，SessionManager 从消息路由字段中提取**稳定路由键**（platform + sender_id + peer_id + account_id）做 registry 查找。`/new` 指令在同稳定路由键下创建新 session 时覆盖映射，旧 session 自然脱离路由。

SessionRouter 不区分私聊和群聊。会话粒度由 IM Adapter 通过 peer_id 的构造方式控制——IM Adapter 决定什么构成一个"会话对端"，Session 机制本身是通用的。

**防撞机制**：`timestamp_ms` 的毫秒精度天然提供防撞能力。极罕见碰撞时（两条 `/new` 或首次消息同一毫秒到达且路由字段完全相同），SessionManager 等待 10ms 后重试——此时 SessionRouter 取新系统时间重算，session_key 必然不同。

### 异常处理

链路整体采用 fail-open 策略：任何 Processor 异常不阻塞消息流，回退到透传原文。

| 场景 | 处理 |
|------|------|
| RawLog 写入失败 | 记录错误日志，消息流程不受影响 |
| SessionRouter 计算 key 失败 | 记录告警日志，session_key 留空，消息继续流转 |
| ContentNormalizer 异常 | 记录告警日志，丢弃变换结果，透传原始 content |
| IM Adapter 解析失败 | 由 IM Adapter 自身处理（不产 NormalizedMessage 即丢弃消息）。非文本消息正常产 NormalizedMessage，不做丢弃处理 |

## 数据流

```
IM Adapter 产出 NormalizedMessage { platform, sender_id, peer_id, thread_id?, account_id, content, timestamp }
  → RawLogProcessor：记录原始内容到日志 → 透传（保留所有字段）
    → SessionRouter：计算 session_key（算法见上文 session_key 算法节）→ 写入 metadata.session_key
      → ContentNormalizer：文本标准化（去控制字符、压缩空行、去尾空格）。非文本消息跳过标准化，直接透传 content
        → ProcessedMessage { content, metadata { session_key, thread_id } }
          → Gateway
            → 调用 SessionManager.resolve()，SessionManager 从消息路由字段提取稳定路由键做查找 → 获得 session_id
            → 路由决策（/ 开头 → 斜杠指令；否则 → LLM 对话）
```

关键判断点：
- SessionRouter 计算失败时 session_key 留空，Gateway 收到无 key 消息时回复用户"会话路由失败，请重试"
- ContentNormalizer 异常时内容不变（丢回原文），消息继续流转
- 所有异常均记录日志，用于后续问题定位和改进



## 模块关系

- **上游**：IM Adapter（各平台提供适配器，产 NormalizedMessage）
- **下游**：Gateway（接收 ProcessedMessage，消费 metadata.session_key 用于消息追踪）、Session 模块（SessionRouter 计算的 session_key 随 metadata 经 Gateway 传递给 SessionManager；SessionManager 使用稳定路由键做 session 路由查找，属数据流下游依赖）
- **链内**：
  - RawLogProcessor — 审计日志（副作用），不改内容
  - SessionRouter — 计算 session_key（纯哈希计算），写 metadata
  - ContentNormalizer — 文本标准化（去控制字符、压缩空行、去尾空格）
- **无关**：出站 Processor Chain（独立链路，与入站互不干扰）
