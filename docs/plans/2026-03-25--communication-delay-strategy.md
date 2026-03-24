# Multi-Agent 通讯延迟处理策略设计方案

> 状态：已定稿 | 作者：脑暴虾 | 日期：2026-03-25

---

## 文档变更记录

| 日期 | 修改者 | 修改内容 |
|------|--------|----------|
| 2026-03-25 | Braino | 初次撰写。基于 MULTI_AGENT_ARCHITECTURE.md 第 4 节现有通讯机制，参考 #64 经验推送机制需求，明确推荐「拉取为主+内存队列」的混合策略。 |

---

## 1. 背景与问题

### 1.1 现状

`MULTI_AGENT_ARCHITECTURE.md` 第 4 节定义了通讯机制的基本框架：

- **通讯名单**：每个 Agent 的 `outbound` / `inbound` 名单
- **中央仲裁**：CloseClaw 路由层校验双向通讯权限
- **通讯方式**：子→父上报（主动发起）、父→子下行（子拉取父状态）

但**第 4.5 节未涉及**：
- 消息传递失败时的超时与重试
- 离线/不可达时的消息缓冲
- 死信（永久失败消息）的处理
- 延迟监控与告警

### 1.2 延迟场景

Multi-Agent 通讯中存在以下延迟场景：

| 场景 | 描述 | 典型延迟 |
|------|------|---------|
| **父子心跳** | 子 Agent 定期拉取父 Agent 状态 | 毫秒～秒级 |
| **经验推送**（#64） | 父向子推送新经验通知 | 秒～分钟级 |
| **任务下发** | 父向子分配任务，子返回结果 | 秒～分钟级 |
| **横向消息** | Agent A 向 Agent B 传递中间结果 | 毫秒～秒级 |
| **父不可达** | 父 Agent 下线/重启，子持续尝试重连 | 秒～小时级 |

### 1.3 核心问题

1. **通讯失败无重试**：当前无重试机制，失败即丢弃
2. **无死信处理**：永久失败的消息没有记录和告警
3. **延迟不可见**：无法监控消息端到端延迟
4. **与 #64 耦合**：经验推送机制依赖可靠的延迟处理基础设施

---

## 2. 方案选型

### 2.1 方案对比

#### 方案 A：消息队列（RabbitMQ / Redis Streams / Kafka）

**思路**：引入专用消息中间件，所有 Agent 间消息通过队列路由。

| 维度 | 评分 |
|------|------|
| 可靠性 | ⭐⭐⭐⭐⭐ 持久化、确认机制完善 |
| 延迟 | ⭐⭐⭐⭐ 毫秒级 |
| 运维复杂度 | ⭐ 需额外部署和维护消息中间件 |
| 成本 | ⭐ 需额外资源 |
| 适用规模 | 百级以上 Agent |

**结论**：过度设计。CloseClaw 预期 Agent 数量有限，引入 MQ 徒增运维负担。

---

#### 方案 B：长连接（WebSocket / gRPC Streaming）

**思路**：Agent 间维持永久连接，消息通过连接推送。

| 维度 | 评分 |
|------|------|
| 可靠性 | ⭐⭐⭐ 连接断开时消息丢失（需额外保障） |
| 延迟 | ⭐⭐⭐⭐⭐ 实时推送 |
| 运维复杂度 | ⭐⭐⭐ 连接管理、心跳保活、实现复杂 |
| 成本 | ⭐⭐⭐ 无额外资源 |
| 适用规模 | 十～百级 Agent |

**结论**：实现复杂，且连接管理（断连、重连、心跳）本身就是一套完整的工作量。在 CloseClaw 规模下，收益不足。

---

#### 方案 C：拉取为主 + 内存队列（推荐）

**思路**：

- **下行（父→子）**：父将消息放入子 Agent 的**内存队列**（Inbox），子下次拉取时一并返回
- **上行（子→父）**：子主动发起 HTTP 请求到父，父返回结果（已有设计）
- **横向**：同父子，通过父中转（符合架构约束）

| 维度 | 评分 |
|------|------|
| 可靠性 | ⭐⭐⭐⭐ 内存队列 + 重试机制，可持久化 Inbox |
| 延迟 | ⭐⭐⭐ 轮询间隔决定，最坏 RTT |
| 运维复杂度 | ⭐⭐⭐⭐ 无额外依赖，纯内存实现 |
| 成本 | ⭐⭐⭐⭐⭐ 零额外成本 |
| 适用规模 | 任何规模 |

**结论**：✅ **明确推荐方案 C**。与第 4.5 节「子拉取父状态」的设计完全兼容，复用现有架构，无需引入外部依赖，实现成本低。

---

### 2.2 方案 C 的扩展：持久化 Inbox

对于经验推送（#64）等需要**可靠传递**的场景，在方案 C 基础上增加持久化 Inbox：

```
父 Agent 想推送经验给子 Agent
    ↓
持久化到子 Agent 的 Inbox 存储（SQLite / JSON 文件）
    ↓
子下次拉取时，从 Inbox 读取未读消息
    ↓
子返回 ACK，父标记消息为已读
    ↓
（可选）超过 N 天未读，触发告警
```

持久化 Inbox 与内存队列的关系：

| 组件 | 用途 | 持久化 |
|------|------|--------|
| **内存队列** | 高频心跳、实时状态同步 | 否 |
| **持久化 Inbox** | 经验推送、任务下发等重要消息 | 是 |

---

## 3. 详细设计

### 3.1 超时配置

```json
{
  "communication": {
    "pull": {
      "interval_ms": 5000,
      "timeout_ms": 10000,
      "max_retry": 3
    },
    "request": {
      "connect_timeout_ms": 5000,
      "read_timeout_ms": 30000
    },
    "inbox": {
      "retention_days": 7,
      "max_size_mb": 100,
      "warning_threshold_mb": 80
    }
  }
}
```

| 配置项 | 说明 | 默认值 |
|--------|------|--------|
| `pull.interval_ms` | 子拉取父状态的轮询间隔 | 5000ms |
| `pull.timeout_ms` | 单次拉取请求超时 | 10000ms |
| `pull.max_retry` | 拉取失败最大重试次数 | 3 |
| `request.connect_timeout_ms` | HTTP 连接建立超时 | 5000ms |
| `request.read_timeout_ms` | HTTP 响应读取超时 | 30000ms |
| `inbox.retention_days` | Inbox 消息保留天数 | 7 天 |
| `inbox.max_size_mb` | Inbox 目录最大占用 | 100MB |

### 3.2 重试策略

**指数退避（Exponential Backoff）**：

```
第 N 次重试等待时间 = min(base * 2^N, max_backoff)

base = 1000ms（1秒）
max_backoff = 60000ms（1分钟）
max_retry = 3
```

| 重试次数 | 等待时间 |
|---------|---------|
| 1 | 1s |
| 2 | 2s |
| 3 | 4s |
| 放弃 | 标记死信，告警 |

**Jitter（抖动）**：在退避时间基础上加随机抖动 ±500ms，避免多子 Agent 同时重试造成惊群效应。

### 3.3 死信处理

#### 3.3.1 死信定义

满足以下任一条件的消息标记为死信（Dead Letter）：
- 重试次数超过 `max_retry` 仍失败
- 消息在 Inbox 中超过 `retention_days` 未被消费
- 目标 Agent 已被销毁但 Inbox 中仍有未读消息

#### 3.3.2 死信处理流程

```
消息重试失败
    ↓
标记为 dead_letter，移入 dead_letter_inbox/
    ↓
记录死信日志（包含：原始消息、失败原因、重试次数、时间戳）
    ↓
告警通知（可配置：飞书/日志/不告警）
    ↓
死信保留 30 天后自动清理
```

#### 3.3.3 死信存储结构

```
~/.closeclaw/agents/<agent_id>/inbox/
├── pending/           # 未读消息
│   └── <msg_id>.json
├── acked/             # 已确认消息（保留 N 天后清理）
│   └── <msg_id>.json
└── dead_letter/       # 死信目录
    └── <msg_id>.json
```

**死信 JSON 格式**：

```json
{
  "id": "msg-uuid",
  "from": "agent-parent-uuid",
  "to": "agent-child-uuid",
  "type": "experience_push|task|heartbeat",
  "payload": {},
  "created_at": "2026-03-25T10:00:00Z",
  "retry_count": 3,
  "last_error": "connection timeout",
  "dead_letter_at": "2026-03-25T10:05:00Z"
}
```

### 3.4 延迟监控

#### 3.4.1 指标定义

| 指标 | 计算方式 |
|------|---------|
| **enqueue_latency** | 消息入队到被消费的时间差 |
| **pull_rtt** | 拉取请求的往返时间 |
| **inbox_size** | pending 队列当前长度 |
| **dead_letter_rate** | 死信数量 / 总消息数量 |

#### 3.4.2 监控输出

通过 `/communication-stats` 指令输出：

```
=== Multi-Agent 通讯统计 ===
Agent: agent-child-uuid
Inbox: 3 条待读, 12 条已读, 0 条死信
平均拉取延迟: 234ms
平均入队到消费延迟: 1.2s
最后拉取时间: 2026-03-25T10:05:00Z
```

#### 3.4.3 告警规则

| 条件 | 级别 | 动作 |
|------|------|------|
| 死信数量 > 0 | 警告 | 记录日志 |
| 死信数量 > 10 | 严重 | 飞书通知 |
| Inbox 大小 > 80MB | 警告 | 记录日志 |
| 拉取延迟 > 30s | 警告 | 记录日志 |
| Agent 离线 > 5min | 信息 | 记录日志 |

---

## 4. 与 #64 经验推送机制的关系

### 4.1 协同关系

| Issue | 关注点 |
|-------|--------|
| **#64** | 推送什么内容（经验）、何时推送、推送给谁 |
| **#65（本文）** | 怎么可靠传递（传输层基础设施） |

**#64 依赖 #65**。经验推送机制必须建立在可靠的延迟处理基础设施之上，否则推送失败无法重试、死信无法处理。

### 4.2 协同设计

```
父 Agent 决定推送经验（#64 逻辑）
    ↓
调用 PushMessage(from, to, ExperiencePayload)（#65 基础设施）
    ↓
持久化到子的 Inbox（#65 Inbox）
    ↓
子下次拉取时获取经验消息（#65 Pull）
    ↓
子消费后 ACK（#65 ACK）
    ↓
（若失败）走 #65 重试和死信流程
```

### 4.3 优先级区分

| 消息类型 | 传输方式 | 重试策略 |
|---------|---------|---------|
| 状态同步（心跳） | 拉取，不持久化内存队列 | 不重试 |
| 经验推送（#64） | 持久化 Inbox | 指数退避重试 |
| 任务下发 | 持久化 Inbox | 指数退避重试 |
| 横向中间结果 | 内存队列 | 简单重试 1 次 |

---

## 5. 实现计划

### 5.1 整体计划

| Phase | 内容 | 依赖 | 预计工作量 |
|-------|------|------|----------|
| **Phase 1** | Inbox 基础设施：`InboxManager`（增删改查、持久化）+ ACK 机制 | 无 | 中 |
| **Phase 2** | 拉取逻辑：子 Agent 定时拉取父状态，复用第 4.5 节设计 | Phase 1 | 小 |
| **Phase 3** | 重试与死信：指数退避重试策略 + `dead_letter_inbox` + 告警 | Phase 1 | 中 |
| **Phase 4** | 监控：`/communication-stats` + 延迟指标埋点 | Phase 1-3 | 小 |
| **Phase 5** | 与 #64 集成：经验推送复用 Inbox 基础设施 | Phase 1 + #64 | 小 |

### 5.2 核心接口设计

```rust
// InboxManager：管理 Agent 的收件箱
struct InboxManager {
    agent_id: String,
    inbox_path: PathBuf,
}

impl InboxManager {
    // 写入消息（持久化到 pending/）
    async fn enqueue(&self, msg: &AgentMessage) -> Result<String, InboxError>;

    // 拉取待读消息，返回消息列表并标记为已分发
    async fn fetch_pending(&self, limit: usize) -> Result<Vec<AgentMessage>, InboxError>;

    // 确认消息（消费成功）
    async fn ack(&self, msg_id: &str) -> Result<(), InboxError>;

    // 获取统计信息
    async fn stats(&self) -> InboxStats;
}

// AgentMessage：跨 Agent 消息结构
struct AgentMessage {
    id: String,
    from: String,
    to: String,
    msg_type: MessageType, // heartbeat | experience_push | task | lateral
    payload: Value,
    created_at: DateTime<Utc>,
    retry_count: u8,
}
```

### 5.3 目录结构

```
~/.closeclaw/agents/<agent_id>/
├── config.json
├── permissions.json
└── inbox/
    ├── pending/          # 未读消息（JSON 文件）
    ├── acked/            # 已确认消息
    ├── dead_letter/      # 死信
    └── inbox.db          # SQLite 索引（可选，加速查询）
```

---

## 6. 风险与注意事项

### 6.1 已知风险

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|---------|
| **父 Agent 重启后 Inbox 丢失** | 低 | 高 | 持久化到磁盘，启动时恢复 |
| **Inbox 目录磁盘占满** | 中 | 中 | `max_size_mb` 限制 + 空间报警 |
| **拉取间隔太长导致实时性差** | 中 | 中 | `interval_ms` 可配置，建议 ≤5s |
| **死信堆积无人处理** | 低 | 低 | 告警机制 + 30 天自动清理 |
| **子 Agent 数量过多拉爆父** | 低 | 中 | 拉取间隔错峰（加 jitter）|

### 6.2 注意事项

1. **不要在心跳路径上做持久化**：心跳（状态同步）走纯内存队列，不写盘，避免影响性能
2. **ACK 时机要明确**：子收到消息后应立即 ACK，不要等到处理完再 ACK，避免父以为失败而重试
3. **Inbox 大小监控**：定期检查 `inbox/` 目录占用，超阈值时告警
4. **与第 4.5 节兼容**：本文设计是对第 4.5 节「子拉取父状态」的细化，不改变原有架构
5. **#64 经验推送优先实现**：经验推送是第一个依赖本基础设施的功能，实现顺序应先 #65 再 #64

---

## 7. 附录：与其他架构文档的关系

| 文档 | 关系 |
|------|------|
| `MULTI_AGENT_ARCHITECTURE.md` §4 | 本文细化了 §4.5「子拉取父状态」的设计 |
| `MULTI_AGENT_ARCHITECTURE.md` §5 | 经验共享机制依赖本文的传输基础设施 |
| `#64 经验推送机制` | #64 依赖本文作为传输层 |
| `2026-03-24--session-and-topic-design.md` | 平行设计，同属 Multi-Agent 架构子课题 |
