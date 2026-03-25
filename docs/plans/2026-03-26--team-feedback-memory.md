# Team Feedback Memory — 从纠正中学习的共享机制

## 背景与问题

**现状**：
- Owner 纠正了某个 Agent 的行为（比如"这个设计不对"）
- Agent 道歉并修正，但没有持久化
- 下次同类问题 Agent 还是会犯

**OpenClaw 参考**：
- OpenClaw 有 `HEARTBEAT.md` 的 `.learnings/` 机制
- 但只在脑暴虾自己的心跳里，单 Agent 维度

**需求**：
- Owner 的纠正自动存入共享反馈库
- 所有相关 Agent 都能检索
- 在类似场景下主动想起、自我纠正

## 方案选型

### 方案 A：被动学习（Feedback-based Learning）

每次 Owner 纠正后，Agent 记录到共享反馈库。下次遇到类似场景时，主动检索。

```
Owner 纠正：Brainstormer，这个设计缺少错误分类
    ↓
记录到 ~/.closeclaw/feedback/
    ↓
下次 Brainstormer 开始设计前
    ↓
主动检索相关 feedback："最近有什么设计被纠正过？"
    ↓
类似场景 → 主动想起并避免
```

### 方案 B：主动规则注入（Rule Injection）

Owner 纠正后，自动生成规则注入到 Agent 的 system prompt。

```python
# 每次纠正后
rule = f"规则：设计文档必须包含错误分类章节"
append_to_agent_config(agent_id, rules=[rule])
```

| 优点 | 缺点 |
|------|------|
| 效果直接 | 规则越来越多，难以维护 |
| Agent 不会重复犯错 | 没有"场景上下文"，规则可能过度泛化 |
| | 难以区分"一次性纠正"和"永久规则" |

### 方案 C：向量检索 + 主动提醒

用 embedding 存储反馈，Agent 在关键节点主动检索相似反馈。

| 优点 | 缺点 |
|------|------|
| 不需要维护规则列表 | 需要 embedding 服务 |
| 可携带场景上下文 | 检索质量依赖 embedding 模型 |
| 区分一次性纠正和永久规则 |  |

**推荐方案 A + C 结合**：
- 反馈用 Markdown 存储（`YYYY-MM-DD--feedback--context.md`）
- 用 embedding 做 semantic search
- Agent 在关键节点（开始设计前、提交 review 前）主动检索
- 一次性纠正用 `repeat: false`，永久规则用 `repeat: true`

## 反馈库结构

```
~/.closeclaw/feedback/
├── rules/                    # 永久规则（repeat: true）
│   └── YYYY-MM-DD--rule-name.md
├── one-time/                 # 一次性纠正（repeat: false）
│   └── YYYY-MM-DD--context-name.md
└── index.json                # 索引（embedding 向量）
```

### 单条反馈格式

```yaml
feedback:
  id: "fb-2026-03-26-001"
  from: "owner"          # 谁纠正的
  to: "brainstormer"     # 被纠正的 Agent
  created_at: "2026-03-26T02:45:00+08:00"
  
  context:
    trigger: "提交了设计文档：LLM Fallback 策略"
    correction: "缺少错误分类章节"
    original: "设计文档没有对 LLM 错误进行分类"
  
  pattern:
    # 这次纠正属于哪个 pattern（用于归类）
    category: "design-doc-completeness"
    tags: ["设计文档", "完整性", "错误处理"]
  
  resolution:
    # 当时怎么解决的
    action: "补充了错误分类章节（Transient/Auth/Billing/InvalidRequest）"
  
  repeat: false  # true = 永久规则，false = 一次性
  status: "resolved"  # resolved / active
```

## 实现计划

### 步骤一：Feedback 收集

当 Owner 纠正 Agent 时，Agent 自动记录：

```python
def on_owner_correction(correction: str, context: str):
    feedback = Feedback(
        from="owner",
        to=self.agent_id,
        context=context,
        correction=correction,
        pattern=infer_pattern(correction),
    )
    feedback_store.save(feedback)
```

### 步骤二：Semantic Search

```python
def retrieve_relevant_feedback(context: str, agent_id: str):
    results = vector_search(
        query=context,
        index=feedback_index,
        filter={"to": agent_id, "status": "active"}
    )
    return results
```

### 步骤三：主动提醒（Agent 侧）

在关键节点插入：

```
在开始设计前：
"检索反馈库：最近有哪些类似设计被纠正过？"
→ 如果有 active 反馈，在 system prompt 里注入提醒

在提交设计 review 前：
"最终检索：有没有遗漏的常见问题？"
→ 类似 self-review checklist
```

### 步骤四：反馈生命周期

- `active`：还在影响 Agent 行为
- `resolved`：已确认修复完成
- `archived`：不再相关，保留存档

Owner 可手动升级：`one-time` → `rule`（一次性纠正变成永久规则）

## 与 Team Memory 的关系

Feedback Memory 是 Team Memory 的一部分：

```
team-memory/
├── DECISIONS/
├── CONTEXT/
├── LEARNINGS/         ← 这里放 Feedback Memory
│   ├── rules/
│   └── one-time/
└── index.json
```

## 扩展性

- 支持"团队学习"：A Agent 的反馈，B Agent 也能学到
- 支持反馈评分：Owner 可标记某个反馈"有用/没用"，调整检索权重
- 支持趋势分析：高频反馈说明有系统性问题，需要从流程上解决
