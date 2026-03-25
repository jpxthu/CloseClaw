# Proactive Alert — Agent 主动通知机制

## 背景与问题

**现状**：
- Agent 完成后只能等用户下次来问才知道结果
- Owner 不知道设计什么时候完成、任务什么时候失败
- 类似 CI/CD 的"构建完成通知"，但目前完全缺失

**需求**：
- 关键事件发生时，主动 push 飞书消息给 Owner
- 不要打扰——只在真正需要关注时才发

## 方案选型

### 触发事件分级

| 级别 | 事件 | 通知方式 |
|------|------|---------|
| 🔴 高 | 设计完成，需 Owner review | 飞书消息 + @Owner |
| 🔴 高 | 任务失败，需人工介入 | 飞书消息 + @Owner |
| 🟡 中 | 任务超时（超过预估时间 2x） | 飞书消息 |
| 🟢 低 | 设计完成，自动交接给 builder | 日志记录，不发消息 |
| 🟢 低 | 心跳正常 | 不发消息 |

### 方案 A：事件驱动（Event-driven）

每个 Agent 的关键节点发出事件，Gateway 的 Alert Service 统一处理分发。

```
Agent 完成设计 → Event: design.completed → Alert Service → 判断是否需要通知 → 飞书 push
```

| 优点 | 缺点 |
|------|------|
| 解耦，扩展性好 | 需要 Gateway 支持事件总线 |
| 可以加更多订阅方（不止飞书） | 实现复杂度较高 |

### 方案 B：Agent 内置（Inline）

每个 Agent 在关键节点直接调用飞书 API 发消息。

```python
def complete_design():
    save_design_doc()
    if needs_owner_review():
        feishu.send_message(owner_id, f"设计已完成，请 review: {link}")
```

| 优点 | 缺点 |
|------|------|
| 实现简单，直接 | 侵入性强，耦合到 Agent 逻辑 |
| 不需要额外基础设施 | 难以统一管理通知策略 |

### 方案 C：心跳触发（Heartbeat-based）

沿用现有心跳机制，心跳时检查"有没有需要 Owner 知道的事"，统一推送。

| 优点 | 缺点 |
|------|------|
| 和现有架构一致 | 有最多 1 分钟延迟 |
| 通知可批量 | 不适合真正紧急的事件 |

**推荐方案 A**（事件驱动）+ **飞书渠道内置**：
- 事件总线用内存队列（轻量）
- 每个事件类型有对应 Handler
- 飞书 Handler 是默认实现，可替换

## 实现计划

### 步骤一：事件类型定义

```python
class Event(Enum):
    DESIGN_COMPLETED = "design.completed"       # 需要 review
    TASK_FAILED = "task.failed"                  # 需要介入
    TASK_TIMEOUT_WARNING = "task.timeout_warning" # 超时提醒
    BUILDER_STARTED = "builder.started"         # 交接给 builder
    RISK_DETECTED = "risk.detected"             # 发现风险
```

### 步骤二：Alert Service

```python
class AlertService:
    def handle(self, event: Event, payload: dict):
        if event.needs_notify:
            self.send_feishu(event, payload)
```

### 步骤三：Owner 配置

```yaml
alert:
  owner_id: "ou_xxx"
  rules:
    - event: "design.completed"
      notify: true
      message: "💡 设计已完成，请 review"
    - event: "task.failed"
      notify: true
      urgent: true
```

### 步骤四：飞书发送

- 使用 `feishu_im_user_message` 工具发送
- 支持 @Owner（通过 open_id）
- 消息内含快速链接（设计文档 / GitHub issue）

## 扩展性

- 支持多个订阅方（Owner 之外可以加 developer）
- 支持不同渠道（飞书 / 邮件 / Slack）
- 支持通知聚合（多个同类事件合并成一条）
