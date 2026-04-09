# Multi-Agent 通讯延迟处理策略 Spec

> Issue: GitHub #138  
> Status: Spec v1.0  
> Created: 2026-04-09

## 1. 概述

### 1.1 背景

`MULTI_AGENT_ARCHITECTURE.md` 第 4 节定义了通讯机制的基本框架，但未涉及：
- 消息传递失败时的超时与重试
- 离线/不可达时的消息缓冲
- 死信（永久失败消息）的处理

**核心问题**：通讯失败无重试、无死信处理、延迟不可见。

### 1.2 设计目标

1. **可靠性**：消息传递失败时自动重试，提高最终成功率
2. **可观测性**：延迟、pending、dead_letter 数量可见可告警
3. **可恢复性**：父 Agent 重启后 Inbox 状态可恢复
4. **无惊群效应**：多子 Agent 重试不集中，避免负载突刺

---

## 2. 通讯模型

### 2.1 消息分类

| 消息类型 | 传输方式 | 重试策略 | 持久化 |
|---------|---------|---------|--------|
| 状态同步（心跳） | 拉取，不持久化 | 不重试 | 否 |
| 任务下发 | 持久化 Inbox | 指数退避重试 | 是 |
| 横向中间结果 | 内存队列 | 简单重试 1 次 | 否 |

### 2.2 上下行定义

| 方向 | 定义 | 实现 |
|------|------|------|
| **下行（父→子）** | 父将消息放入子 Agent 的 Inbox | InboxManager 持久化 |
| **上行（子→父）** | 子主动发起请求到父，父返回结果 | HTTP 请求响应 |

---

## 3. Inbox 基础设施

### 3.1 目录结构

```
~/.closeclaw/agents/<agent_id>/inbox/
├── pending/           # 未读消息
├── acked/             # 已确认消息（保留 7 天）
└── dead_letter/       # 死信（保留 30 天）
```

### 3.2 消息格式

```json
{
  "id": "msg-uuid",
  "from": "parent-agent-uuid",
  "to": "child-agent-uuid",
  "type": "task|heartbeat|lateral",
  "payload": {},
  "status": "pending|acked|dead_letter",
  "retry_count": 0,
  "max_retry": 3,
  "created_at": "2026-04-09T00:00:00Z",
  "acked_at": null,
  "dead_letter_at": null,
  "next_retry_at": null
}
```

### 3.3 InboxManager API

| 操作 | 说明 |
|------|------|
| `push(msg)` | 添加消息到 pending |
| `pull(agent_id)` | 拉取所有 pending 消息，返回后标记为 acked |
| `ack(msg_id)` | 确认消息 |
| `get_stats()` | 返回 pending/dead_letter 数量和平均延迟 |
| `gc()` | 清理过期消息（acked > 7 天，dead_letter > 30 天） |

### 3.4 持久化

- 存储格式：JSON 文件，每条消息一个文件
- 路径：`~/.closeclaw/agents/<agent_id>/inbox/pending/<msg_id>.json`
- 加载：Agent 启动时读取 pending 目录，恢复未处理消息

---

## 4. 重试策略

### 4.1 指数退避

```
第 N 次重试等待时间 = min(60000ms, 1000ms × 2^N)
```

| 重试次数 | 等待时间 |
|---------|---------|
| 1 | 1s |
| 2 | 2s |
| 3 | 4s |
| >3 | 不再重试 |

### 4.2 Jitter

在指数退避基础上加随机抖动 ±500ms，避免多子 Agent 同时重试：

```
actual_wait = calculated_wait + random(-500ms, +500ms)
```

### 4.3 重试触发条件

- 消息发送超时（默认 10s）
- 收到 5xx 错误
- 网络不可达

### 4.4 不重试条件

- 收到 4xx 错误（客户端错误，无需重试）
- 消息已明确标记为 dead_letter

---

## 5. 死信处理

### 5.1 死信定义

满足以下任一条件即为死信：
1. 重试次数超过 `max_retry`（默认 3 次）仍失败
2. 消息在 Inbox 中超过 `retention_days`（默认 30 天）未被消费

### 5.2 死信处理流程

```
消息重试失败（retry_count > max_retry）
    │
    ▼
将消息移入 dead_letter/
    │
    ▼
记录死信日志（包含失败原因、最后一次错误）
    │
    ▼
发送告警通知（如果配置了告警 Webhook）
    │
    ▼
死信保留 30 天后自动清理
```

### 5.3 死信日志格式

```json
{
  "msg_id": "msg-uuid",
  "original_msg": {},
  "failure_reason": "max_retries_exceeded|retention_expired",
  "last_error": "Connection timeout after 10000ms",
  "retry_count": 3,
  "dead_letter_at": "2026-04-09T00:00:00Z"
}
```

---

## 6. 监控接口

### 6.1 GET /communication-stats

返回格式：

```json
{
  "agent_id": "xxx",
  "pending_count": 5,
  "acked_count": 123,
  "dead_letter_count": 2,
  "avg_latency_ms": 1250,
  "max_latency_ms": 8500
}
```

### 6.2 延迟定义

- `latency` = 消息创建时间 → 消息被 ACK 的时间差
- `avg_latency_ms` = 过去 1 小时内所有消息的平均延迟
- `max_latency_ms` = 过去 1 小时内的最大延迟

### 6.3 告警规则

| 条件 | 级别 | 动作 |
|------|------|------|
| dead_letter_count > 0 | WARN | 记录日志 |
| pending_count > 100 | WARN | 记录日志 |
| avg_latency_ms > 30000 | WARN | 记录日志 |

---

## 7. 实现任务（Phase 1-4）

### Phase 1：Inbox 基础设施
- [ ] InboxManager 核心实现（push/pull/ack）
- [ ] 持久化存储（JSON 文件）
- [ ] 目录结构管理（pending/acked/dead_letter）
- [ ] Agent 启动时恢复 Inbox 状态

### Phase 2：拉取逻辑
- [ ] 子 Agent 定时拉取（轮询间隔可配置，默认 5s）
- [ ] 父 Agent 提供 GET /inbox 接口
- [ ] 每次拉取后自动 ACK

### Phase 3：重试与死信
- [ ] 指数退避重试实现
- [ ] Jitter 实现
- [ ] 死信判定与移动
- [ ] 死信日志记录
- [ ] GC 后台任务（清理过期消息）

### Phase 4：监控
- [ ] GET /communication-stats 接口
- [ ] 告警通知机制
- [ ] 延迟统计

---

## 8. 配置项

| 配置项 | 默认值 | 说明 |
|--------|-------|------|
| `inbox.poll_interval_secs` | 5 | 子 Agent 拉取间隔 |
| `inbox.max_retry` | 3 | 最大重试次数 |
| `inbox.base_delay_ms` | 1000 | 基础退避延迟 |
| `inbox.max_delay_ms` | 60000 | 最大延迟上限 |
| `inbox.jitter_ms` | 500 | Jitter 范围 |
| `inbox.timeout_ms` | 10000 | 消息发送超时 |
| `inbox.acked_ttl_days` | 7 | acked 消息保留天数 |
| `inbox.dead_letter_ttl_days` | 30 | 死信保留天数 |
| `inbox.alert_webhook` | null | 告警 Webhook URL |

---

## 9. 验收标准

1. ✅ 消息传递失败时指数退避重试（最大 3 次）
2. ✅ 超过重试次数进入 dead_letter/
3. ✅ 死信保留 30 天后自动清理
4. ✅ 父 Agent 重启后 Inbox 状态可恢复
5. ✅ /communication-stats 正确返回 pending/dead_letter 数量和延迟指标
6. ✅ 子 Agent 拉取间隔可配置，默认 5s
7. ✅ 多子 Agent 重试不集中（Jitter 起作用）

---

## 10. 依赖

- 无外部依赖
- 使用标准库 JSON 处理
- 使用 tokio 异步运行时（如已引入）

---

## 11. 参考文档

- `docs/agent/MULTI_AGENT_ARCHITECTURE.md` — 多智能体层级架构
- GitHub Issue #138 — 原始需求
