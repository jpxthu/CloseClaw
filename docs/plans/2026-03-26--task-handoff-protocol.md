# Task Handoff Protocol — Agent 间结构化任务交接

## 背景与问题

**现状**：
- 脑暴虾完成设计后，在 GitHub comment 里 @ 计划虾
- 但 closed issue 的 comment 不会触发通知
- 计划虾不知道设计已经完成，需要主动去翻聊天记录

**需求**：
- Agent 之间交接任务时，有结构化的交接包
- 不靠 comment 漂流，有明确的"工单流转"机制

## 方案选型

### 方案 A：GitHub Issue 作为交接介质（现状问题版）

用 GitHub Issue 传递任务，但当前问题是没有通知机制。

- ✅ 人类也能看到
- ❌ closed issue @ 不触发通知
- ❌ 交接包格式不统一

### 方案 B：Session-based Handoff

源 Agent 直接在目标 Agent 的 session 里注入上下文。

```python
source_agent.handoff_to(
    target_agent="builder",
    session_context={
        "task": "implement llm fallback",
        "design_doc": "https://...",
        "acceptance_criteria": [...],
        "relevant_history": [...]
    }
)
```

| 优点 | 缺点 |
|------|------|
| 结构化，有类型定义 | 需要目标 Agent 在线才能即时送达 |
| 不依赖 GitHub 通知机制 | 如果是异步的，还是需要某种队列 |
| 上下文可携带完整设计文档 |  |

### 方案 C：Task Queue + Event Notification

任务交接到共享队列，目标 Agent 订阅队列事件。

```
Brainstormer 完成设计
    ↓
创建 Task 对象（包含完整交接包）
    ↓
放入 Task Queue（持久化）
    ↓
通知 Builder Agent（飞书消息 + @）
    ↓
Builder Agent 从 Queue 取任务
```

| 优点 | 缺点 |
|------|------|
| 可靠，任务不会丢失 | 需要持久化队列 |
| 解耦，Agent 可以异步处理 | 实现复杂度高 |
| 天然支持多 Builder 并行 |  |

**推荐方案 B**（Session-based）+ **持久化 backup**：
- 主要路径：源 Agent 直接在目标 Agent 的 session 注入上下文（类似 `sessions_send`）
- 备份路径：同时在 GitHub 创建 issue，确保即使目标 Agent 不在线也有记录
- 飞书通知：主动 push 消息给目标 Agent 的 Owner

## Task Handoff 包结构

```yaml
handoff:
  id: "ht-2026-03-26-001"
  from: "brainstormer"
  to: "builder"
  status: "pending"  # pending → accepted → done
  created_at: "2026-03-26T02:30:00+08:00"
  
  task:
    type: "implementation"
    title: "实现 LLM 调用失败 Fallback 策略"
    design_doc: "docs/plans/2026-03-26--llm-fallback-design.md"
    github_issue: 93
    acceptance_criteria:
      - "偶发 429/5xx 错误能在 3 次重试内恢复"
      - "主模型不可用时自动切换 fallback 模型"
      - "所有 failover 事件有结构化日志"
  
  context:
    summary: |
      需要在 LLM Client 层实现两阶段 Failover：
      1. Auth Profile 轮换 + 重试
      2. Model Fallback Chain 切换
    key_decisions:
      - "Cooldown 持久化到 ~/.closeclaw/llm_cooldowns.json"
      - "配置项：llm.primary + llm.fallbacks[]"
    related_docs:
      - "docs/plans/2026-03-26--llm-fallback-design.md"
    open_questions:
      - "多实例部署时 cooldown 如何共享？"
```

## 实现计划

### 步骤一：Handoff 包定义

```python
@dataclass
class TaskHandoff:
    id: str
    from_agent: str
    to_agent: str
    task: TaskSpec
    context: ContextSummary
    status: HandoffStatus
    created_at: datetime
```

### 步骤二：Handoff Service

```python
class HandoffService:
    def create(self, handoff: TaskHandoff):
        # 1. 持久化到 ~/.closeclaw/handoffs/
        self.persist(handoff)
        
        # 2. 通知目标 Agent 的 Owner（飞书消息）
        self.notify_owner(handoff)
        
        # 3. 在目标 Agent 的 session 注入上下文
        self.inject_to_target_session(handoff)
        
        # 4. 创建 GitHub issue 作为备份记录
        self.create_github_issue(handoff)
```

### 步骤三：Session 注入

```python
def inject_to_target_session(handoff: TaskHandoff):
    message = f"""
## 📋 新任务：{handoff.task.title}

{handoff.context.summary}

设计文档：{handoff.task.design_doc}
验收标准：
{chr(10).join(f'- {c}' for c in handoff.task.acceptance_criteria)}
"""
    sessions_send(
        session_key=f"agent:{handoff.to_agent}:main",
        message=message
    )
```

### 步骤四：飞书通知

```
🤖 [脑暴虾] 交接任务给你：

📋 实现 LLM 调用失败 Fallback 策略
🎯 设计文档：docs/plans/2026-03-26--llm-fallback-design.md
✅ 验收标准：3 条

[查看详情] [接受任务]
```

## 扩展性

- 支持拒绝：Builder 可拒绝任务并说明原因
- 支持状态跟踪：pending → in_progress → done / rejected
- 支持优先级：urgent 任务可加急通知
- 支持 handoff history：所有历史交接可查
