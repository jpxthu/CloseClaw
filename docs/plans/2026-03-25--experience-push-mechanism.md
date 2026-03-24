# 经验推送机制设计方案（父→子下行）

> 状态：已定稿 | 作者：脑暴虾 | 日期：2026-03-25

---

## 背景与问题

### 现状

当前 Multi-Agent 架构（`docs/agent/MULTI_AGENT_ARCHITECTURE.md` 第 5 节）已定义：

- **经验分类**：项目经验（只与当前项目相关）vs 通用经验（可跨项目复用）
- **经验上报触发时机**：子 Agent 完成功能模块、代码 review、解决问题后主动上报
- **经验格式**：标准 JSON Schema（含 id、agent_id、timestamp、type、title、content、context、verified）

父 Agent 在接收到子 Agent 的经验上报后，需要将**通用经验**向下行传递给其他子 Agent。但具体的下行推送机制尚未定义。

### 为什么需要设计

没有下行推送机制，通用经验只能停留在父节点，无法发挥"能力复制"的设计目标：

- 子 Agent A 解决了某个通用问题，子 Agent B 无法自动获知，只能重新踩坑
- 父 Agent 每创建一个新子 Agent，都是"从零开始"，无法继承已有的通用经验
- 经验的价值随时间衰减，如果不能及时传递给需要的子 Agent，经验就失去了意义

### 设计目标

1. **通用经验能自动下行**：父节点的通用经验及时传递给所有子节点
2. **轻量实现**：不能为了"推送"引入复杂的异步消息队列或长连接
3. **优先级明确**：通用经验和项目经验的处理方式不同，边界清晰
4. **事件驱动而非轮询**：子 Agent 有实际需求时才触发，避免无用功

---

## 方案选型

### 方案 A：父 Agent 主动推送（Push）

父 Agent 在接收到子 Agent 的经验上报并判定为"通用经验"后，主动将经验推送给所有其他子 Agent。

**优点**：
- 实时性最强，经验产生后其他子 Agent 能立刻感知

**缺点**：
- 需要父 Agent 与所有子 Agent 维持通信通道（消息队列/长连接/WebSocket 等）
- 子 Agent 数量动态变化时，路由管理复杂
- 推送时子 Agent 可能正在执行其他任务，经验 push 过来会产生干扰或丢失
- 实现成本高，引入额外的通信中间件

### 方案 B：子 Agent 主动拉取（Pull）+ 父 Agent 缓存

子 Agent 在关键节点（如启动新任务、处理关键步骤前）主动向父 Agent 请求经验。父 Agent 维护一个经验缓存（Experience Cache），按类型和标签索引。

**优点**：
- 实现简单，无需消息队列或长连接，纯请求/响应即可
- 子 Agent 自身决定什么时候需要经验，上下文匹配更准确
- 父 Agent 的经验缓存天然支持多子 Agent 并发拉取
- 失败代价低：子 Agent 拉取不到只是少了一条经验，不影响主流程

**缺点**：
- 实时性依赖子 Agent 的拉取时机，如果拉取间隔长会有延迟
- 经验量增大时，子 Agent 每次都拉取全部经验会有带宽浪费

### 方案 C：混合模式（Push + Pull）

子 Agent 在关键节点拉取，父 Agent 在发现高价值经验时主动推送。适用于关键经验（如紧急 bugfix）需要立即触达的场景。

**优点**：
- 兼顾实时性和按需获取
- 高优先级经验可以绕过拉取等待

**缺点**：
- 实现复杂度最高，两套机制需要同时维护
- "高价值"的判定标准难以统一，容易产生歧义

---

### 方案对比

| 维度 | 方案 A（Push） | 方案 B（Pull） | 方案 C（混合） |
|------|--------------|--------------|--------------|
| 实现复杂度 | 高 | **低** | 高 |
| 实时性 | 强 | 依赖拉取时机 | 中 |
| 子 Agent 干扰 | 高 | **无** | 中 |
| 扩展性 | 差（通道管理复杂） | **好** | 中 |
| 失败容错 | 差（推送丢失） | **好（失败可重试）** | 中 |
| 按需匹配 | 差（全局推送） | **好（子 Agent 自决）** | 好 |

---

### 推荐方案：**方案 B（子拉取 + 父缓存）

理由：

1. **CloseClaw 是事件驱动架构**：子 Agent 的每次任务启动/关键步骤本身就是事件，在这些节点自然嵌入拉取调用，无需额外基础设施
2. **父 Agent 的全局视野是缓存层**：父节点天然知道所有子节点的经验，缓存统一管理，按类型/标签索引即可
3. **Pull 失败代价低**：子 Agent 拉不到经验时，不阻塞主流程，可以继续执行，只是少了一条参考
4. **与现有架构一致**：第 5.2 节"经验流转"中，父 Agent 已经有"更新自身 → 推送给所有子节点"的逻辑描述，但未实现。用 Pull 实现"推送"的语义，更简洁

---

## 详细设计

### 1. 经验缓存结构（Experience Cache）

父 Agent 维护一个内存缓存，按经验类型和上下文标签组织：

```json
{
  "general_experiences": [
    {
      "id": "exp-uuid-1",
      "agent_id": "agent-a",
      "timestamp": "2026-03-25T10:00:00Z",
      "type": "general",
      "title": "Rust 错误处理最佳实践",
      "content": "使用 thiserror 替代手动实现 Error trait...",
      "context": {
        "tags": ["rust", "error-handling", "backend"],
        "applicable_scenarios": ["模块间错误传递", "API 错误封装"]
      },
      "verified": true,
      "children_notified": ["agent-b", "agent-c"]
    }
  ],
  "project_experiences": {
    "project-a": [
      {
        "id": "exp-uuid-2",
        "agent_id": "agent-a",
        "timestamp": "2026-03-25T11:00:00Z",
        "type": "project",
        "project_id": "project-a",
        "title": "React 组件拆分策略",
        "content": "...",
        "context": {
          "tags": ["react", "frontend", "project-a"]
        },
        "verified": true
      }
    ]
  }
}
```

**存储位置**：`~/.closeclaw/agents/<parent_id>/experience_cache.json`

**索引**：
- `general_experiences`：无项目绑定，全局共享
- `project_experiences`：按 project_id 分组，仅同项目子 Agent 需要拉取
- `children_notified`：记录哪些子 Agent 已经收到了这条经验（用于去重）

### 2. 拉取触发条件（Event-Driven）

子 Agent 在以下时机主动拉取父节点经验：

| 触发时机 | 说明 | 拉取范围 |
|---------|------|---------|
| **子 Agent 启动时** | 新任务开始前，先获取父节点已有的通用经验 | 全部 general_experiences |
| **关键步骤执行前** | 进入关键技术决策点前（如"开始代码 review"、"开始写测试"） | 匹配 context.tags 的经验 |
| **遇到已知错误码** | 捕获到错误时，先查父节点是否有相关解决方案 | 匹配 error_code 或 error_message 的经验 |
| **每日定时（可选）** | 通过配置开启，每日子 Agent 向父节点同步一次 | 增量新经验（按 timestamp 过滤） |

**触发实现**：在子 Agent 的任务循环（Task Loop）中嵌入经验拉取调用，不需要额外线程或定时器。

### 3. 拉取流程

```
子 Agent 触发拉取（启动/关键步骤/错误）
    │
    ▼
构造 ExperiencePullRequest
{
  child_agent_id: "agent-b",
  trigger: "task_start" | "key_step" | "error_encountered",
  current_context: {
    project_id: "project-a",
    tags: ["rust", "api"],
    error_code: null,
    error_message: null
  },
  last_pull_timestamp: "2026-03-25T08:00:00Z"  // 可选，用于增量拉取
}
    │
    ▼
发送给父 Agent（通过已有的 parent 通信通道）
    │
    ▼
父 Agent ExperienceCache 查询
    │
    ├─▶ 过滤 general_experiences（全部返回）
    │
    ├─▶ 过滤 project_experiences（仅同 project_id）
    │
    ├─▶ 匹配 context.tags（按相关度排序）
    │
    └─▶ 排除 children_notified 中已记录的 child_agent_id
    │
    ▼
返回 ExperiencePullResponse
{
  experiences: [...],
  cache_version: "v1.2"
}
    │
    ▼
子 Agent 消费经验（注入到当前任务上下文，或用于错误恢复）
    │
    ▼
更新本地经验缓存（去重合并）
```

### 4. 优先级机制

**通用经验 vs 项目经验的区分标准**（第 5.1 节待细化，本设计明确化）：

| 经验类型 | 判断规则 | 下行方式 |
|---------|---------|---------|
| **通用经验** | 满足以下任一条件：① 不绑定特定项目；② 绑定的项目与父 Agent 的项目集合无直接关系；③ 上报时 agent 标记 `applicable_to_all: true` | 拉取时全量返回给所有子 Agent |
| **项目经验** | 绑定特定 project_id，且该经验仅对该项目有参考价值 | 仅同 project_id 的子 Agent 拉取时返回 |

**优先级排序**（子 Agent 收到多条经验时）：

1. `verified: true` > `verified: false`（未验证经验仅作参考）
2. 按 `context.relevance_score` 降序（如果提供了相关度分数）
3. 按 `timestamp` 降序（最新经验优先）

### 5. 父 Agent 经验上报处理（配套）

当子 Agent 上报经验到父 Agent 时，父 Agent 执行：

```rust
fn process_child_experience_report(report: ExperienceReport) -> ExperienceCacheUpdate {
    match classify_experience(&report) {
        ExperienceType::General => {
            // 添加到 general_experiences
            // 等待子 Agent 下次拉取时自然获取
        }
        ExperienceType::Project => {
            // 添加到 project_experiences[project_id]
            // 等待同项目子 Agent 拉取
        }
        ExperienceType::UserDecision => {
            // 转发给用户，等待用户判定类型
        }
    }
}
```

**注意**：父 Agent **不需要主动推送**，子 Agent 会在下次关键节点自然拉取到。这与"方案 B"的 Pull 模型一致。

### 6. 缓存淘汰策略

| 条件 | 动作 |
|------|------|
| 经验数量超过 `max_cache_size`（默认 500） | 按 LRU 淘汰最旧、`verified: false` 的经验 |
| 经验超过 `max_age`（默认 90 天）且 `verified: false` | 自动删除 |
| 经验超过 `max_age`（默认 180 天）且 `verified: true` | 降级为"历史经验"，不主动返回，但仍可被拉取 |
| 子 Agent 下次拉取时 | 返回自 `last_pull_timestamp` 以来的增量 |

---

## 实现计划

| Phase | 内容 | 依赖 | 预计工作量 |
|-------|------|------|----------|
| **1** | 经验缓存数据结构和存储：`ExperienceCache` 结构体 + JSON 持久化到 `experience_cache.json` | 无 | 小 |
| **2** | 经验拉取接口：父 Agent 注册 `experience_pull` handler，支持过滤和去重 | Phase 1 | 小 |
| **3** | 子 Agent 拉取调用：在 Task Loop 的启动和关键步骤处嵌入 `pull_experiences()` 调用 | Phase 2 | 小 |
| **4** | 优先级和排序逻辑：实现 verified 过滤、tag 匹配、LRU 淘汰 | Phase 1 | 小 |
| **5** | 经验上报时自动分类：`classify_experience()` 自动判定 general/project | Phase 1 | 小 |
| **6** | 配置项：支持在 `config.json` 中配置触发时机、缓存大小上限、淘汰策略 | Phase 1-4 | 小 |

**不包含在本设计内**（后续迭代）：
- 经验的全文搜索（Phase 3 可以后加）
- 跨父节点的全局经验聚合（需要多级父 Agent 的经验上传机制）
- 经验的编辑/删除接口

---

## 扩展性

### 未来可能的变化方向

1. **跨父节点经验聚合**：多级父 Agent（如根 → 子 → 孙）时，孙节点的通用经验逐级上传到根节点，实现全局共享
2. **经验质量评分**：子 Agent 消费经验后反馈"有用/无用"，父节点按质量分数排序
3. **经验版本化**：同一 context 的经验更新时，保留历史版本，子 Agent 可以拉取特定版本
4. **推送混合**：对 `priority: high` 的经验（如紧急 bugfix），父 Agent 可以主动推送到子 Agent 的待处理队列（需要 Phase 1-6 基础上的扩展）

### 当前设计如何应对

- 缓存结构预留了 `metadata` 字段，可扩展
- 拉取接口的 `filter` 参数支持扩展新的过滤维度
- 淘汰策略可配置，新策略可以独立实现而不影响核心流程

---

## 风险与注意事项

1. **经验过载**：子 Agent 拉取到大量经验时，如果全部注入上下文会超限。子 Agent 应只选取与当前任务最相关的经验（按 tag 匹配 + 排序），其他经验作为背景参考而非显式注入

2. **经验时效性**：Rust 代码规范可能随版本更新而变化，父节点应定期清理过期经验（由 Phase 4 的淘汰策略处理）

3. **通信失败容错**：拉取请求失败时，子 Agent 应记录 `last_pull_timestamp`，下次启动时增量拉取，不阻塞主流程

4. **隐私隔离**：项目经验只在同项目子 Agent 间共享，不同项目的子 Agent 拉取时应被正确过滤

5. **父 Agent 单点**：如果父 Agent 崩溃，经验缓存需要从持久化文件恢复。`experience_cache.json` 应在每次更新后同步落盘（使用 write-ahead log 或直接覆写）

6. **循环上报**：如果经验上报 → 父分类 → 子拉取形成循环，可能导致经验在父子间反复传递。`children_notified` 字段和 `last_pull_timestamp` 配合去重，防止同一经验重复传递

---

## 关键设计决策记录

| 日期 | 决策 | 理由 |
|------|------|------|
| 2026-03-25 | 采用子拉取（Pull）而非父推送（Push） | 实现简单、子 Agent 自决按需获取、失败容错好 |
| 2026-03-25 | 通用经验全量返回，项目经验按 project_id 过滤 | 保持隐私隔离，避免项目间经验泄漏 |
| 2026-03-25 | 不实现主动推送，只实现拉取 + 父缓存 | 与事件驱动架构一致，避免引入消息队列等复杂中间件 |
| 2026-03-25 | 经验按 verified + relevance_score + timestamp 排序 | verified 优先保证质量，新的经验优先保证时效性 |
