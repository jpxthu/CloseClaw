# Spawn 树形拓扑

## 概述

- 一句话：spawn_tree 是 Session 模块内部的运行时子组件，维护父 session 与子 session 的父子关系，供并发检查、steer/kill、级联清理、生命周期联动和重启恢复使用，并对已完成子节点执行回收，保证树的大小不随历史子任务数增长。

## 架构

### 定位与存储结构

spawn_tree 维护一张内存查找表，记录 session 之间的父子关系。每棵 spawn 树的根节点是顶层 session（由用户或外部事件直接创建，非 spawn 产生），子节点在 sessions_spawn 创建时注册。spawn_tree 是纯内存结构，不持久化——重启后依赖 session checkpoint 重建（见重启恢复节）。

内存查找表以父 session ID 为键，子 session 列表为值。每个节点记录：

| 字段 | 含义 |
|------|------|
| session_id | 子 session 唯一标识 |
| parent_session_id | 父 session 标识（顶层 session 为空） |
| agent_id | 目标 agent ID |
| depth | 当前层级（根节点为 0） |
| mode | spawn 模式（run / session），描述子 session 的持久化策略 |

> `mode` 描述 spawn 的持久化策略（run 一次性 / session 持久线程），与 SessionCheckpoint 的 `session_mode` 字段（对话模式 normal/plan/auto）含义不同，二者作用于不同数据结构。

spawn 成功时注册新节点；子 session 完成时回收节点（见节点回收节）；kill 时移除节点（见级联 Kill 节）。已完成子任务的持久化记录（checkpoint + transcript）不受回收影响，结果仍保留可查，其归档与清理由 ArchiveSweeper 独立负责（见 session-lifecycle.md）。

### 查询接口

spawn_tree 提供三类只读查询，供 Session 模块内部使用：

- **list_children**：查询某 session 的所有直接子节点。depth 检查、并发检查、steer/kill 操作依赖此查询
- **list_descendants**：递归查询某 session 的所有后代节点（子树遍历）。级联 kill 和父 session 结束时自动清理依赖此查询
- **get_parent**：查询某 session 的父节点。用于层级完整性校验

### 节点回收

spawn_tree 只保留活跃（未完成）子节点，已完成节点在完成通知入队后立即回收，不长期占用内存。节点运行时状态：**活跃**（执行中）与**完成待回收**（已完成但完成通知入队失败，等待 AnnounceSweeper 补推）；正常路径下入队成功即回收。

子 session 完成（四维执行状态归零且已产出最终 assistant 消息）时，announce 按 session-execution.md「子 session 完成注入」流程入队父 session 消息队列（next 优先级，带去重保护）；入队成功后，立即从 spawn_tree 回收该节点。

入队失败（如父 session 已归档）时，节点保留为「完成待回收」状态，由 AnnounceSweeper 补推重试；父 session 已归档则跳过补推并回收节点（见 run-health.md）。

spawn_tree 节点回收与四维执行状态的 child_active 维度独立：child_active 在子 session 完成时清零（见 session-execution.md），spawn_tree 节点在 announce 入队成功后回收。

回收只移除内存节点，不删除持久化数据。该子 session 的对话历史与 checkpoint 仍在持久化存储中保留，供查阅与恢复，其归档与清理由 ArchiveSweeper 独立负责（见 session-lifecycle.md）。完成通知的送达保证由三层兜底：

- **正常路径**：完成时同步入队 + 去重保护
- **运行时异常路径**：入队失败由 AnnounceSweeper 补推（见 run-health.md）
- **崩溃路径**：SubSessionSpawn 记录在 pending_operations 中持久化，崩溃残留由启动恢复扫描兜底（见 session-recovery.md）

### 回收守护（GC 兜底）

除完成即回收外，spawn_tree 维护守护扫描作为安全网，回收漏回收的节点：

- 子 session 四维执行状态已归零，但长期滞留于「完成待回收」状态（如补推持续失败）→ 回收
- 父 session 已结束（或已被归档清理）但节点残留 → 回收

守护扫描与 AnnounceSweeper 的僵死检测职责不同：僵死检测判定"未完成的子 session 是否僵死并终止"（见 run-health.md），守护扫描清理"已完成但未回收"的残留节点。

### 级联 Kill

sessions_kill 终止指定 session 及其所有后代（子树）。kill 操作始终级联——不存在仅杀单个 session 而不杀其子孙的模式。

级联 kill 的执行顺序：

```
kill session A
  ↓
递归遍历 A 的子树，找出所有后代节点
  ↓
从最深层向浅层逐级终止（先杀叶子，再杀父）：
  每步执行：取消子 session → 清理子 session 数据 → 从 spawn_tree 移除节点
  ↓
最后终止 A 自身
```

从深层向浅层逐级执行，确保每层数据清理完毕后再向上走，不留下孤儿节点。已完成或已终止的 session 跳过终止步骤，但仍从 spawn_tree 移除。

### 生命周期联动

除显式 kill 外，以下场景自动触发级联清理（子 session 对话历史均保留供查阅）：

- **父 session 正常结束**：父 session 完成时，所有仍活跃的子 session 被自动级联终止。否则子 session 失去父节点后无法被 steer
- **父 session 被终止**：父 session 被终止（如 `/stop` 级联、硬超时）时，同上级联终止所有子 session

父 session 归档（ArchiveSweeper 闲置归档）不级联终止子 session：child_active 为 true 时父 session 不被归档（见 session-lifecycle.md）；若因系统错误被归档，子 session 的完成通知被丢弃、残留节点由回收守护清理（见 session-lifecycle.md 与回收守护节）。

子 session 的结束不影响父 session 或其他兄弟节点。

### 重启恢复

spawn_tree 的运行时数据（内存查找表）随网关重启丢失。恢复依赖 session checkpoint 持久化：

**Checkpoint 字段**：SessionCheckpoint 包含以下字段用于记录 spawn 关系：

| 字段 | 含义 |
|------|------|
| parent_session_id | 谁 spawn 了我（顶层 session 为空） |
| depth | 当前层级（根节点为 0） |

spawn 子 session 时写入这两个字段。根 session（非 spawn 创建）没有 parent_session_id，depth 为 0。spawn_tree 节点的 mode（run/session）与 agent_id 不写入 checkpoint——重启重建时 agent_id 取自恢复 session 自身的 checkpoint，mode 标记不重建。

**恢复流程**：

```
网关启动
  ↓
Session 模块逐个恢复活跃 session（现有恢复流程，从 checkpoint 重建 session）
  ↓
spawn_tree 重建：
  遍历所有已恢复的 session 的 checkpoint
  → 有 parent_session_id 且父 session 也已恢复 → 在 spawn_tree 中注册父子关系
  → 有 parent_session_id 但父 session 未恢复（已被 sweep）→ 子 session 降级为根节点，depth 重置为 0
  → 无 parent_session_id → 确认为根节点
```

降级策略：父 session 已不存在时，子 session 降级为独立根节点而非级联清理。恢复是被动行为——重启不应主动删除已持久化的 session 数据。降级后的 session 仍可正常服务用户请求。Announce 队列不持久化——若子 session 恰好在重启前完成但父 session 还来不及消费 announce，该 announce 丢失，由 pending_operations 恢复扫描兜底（向父 session 注入恢复通知，由 LLM 自主决定处理，见 session-recovery.md）。

降级恢复仅在系统重启重建层级关系时生效；运行时的级联终止（steer/kill/父 session 结束）在级联 Kill 与生命周期联动节处理，两者触发时间点不同。

## 数据流

### 节点注册 → 完成回收

```
父 session 调用 sessions_spawn(mode="run", ...)
  ↓
前置检查 + 权限检查通过 → 创建 child session
  ↓
spawn_tree 注册新节点（session_id / parent_session_id / agent_id / depth / mode）
  ↓
子 agent 执行 task（可能多轮 turn）
  ↓
子 session 完成（四维执行状态归零 + 产出最终 assistant 消息）
  ↓
提取最后一条 assistant 消息 → announce 入队父 session 消息队列（next 优先级，带去重）
  ↓
入队成功 → 从 spawn_tree 回收该节点
  ↓
子 session 持久化记录（checkpoint + transcript）保留，由 ArchiveSweeper 独立归档/清理
```

### 级联 Kill、父 Session 结束清理、重启恢复

级联 Kill、父 Session 结束时的级联清理、重启恢复的流程见架构节对应小节（级联 Kill / 生命周期联动 / 重启恢复），此处不重复。

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Session（sessions_spawn / sessions_steer / sessions_kill 工具） | 创建子 session 时注册节点，steer/kill 时查询和移除节点 |
| SessionManager | 重启时遍历已恢复 session 的 checkpoint 重建 spawn_tree |

### 下游

| 模块 | 消费方式 |
|------|---------|
| Session 并发检查 / depth 检查 | 通过 list_children 查询直接子节点数 |
| Session 级联停止 / 生命周期联动 | 通过 list_descendants 查询所有后代节点 |
| Session 层级完整性校验 | 通过 get_parent 查询父节点 |

### 无关

| 模块 | 说明 |
|------|------|
| Agent 模块（纯配置层） | spawn_tree 由 Session 模块持有和维护，Agent 模块不接触运行时父子关系 |
| Permission 模块 | spawn_tree 不参与权限评估；spawn 时的权限检查由 tools 模块触发 PermissionEngine 完成 |
| ArchiveSweeper（session-lifecycle） | spawn_tree 回收只清内存节点，不影响持久化 session 的归档/清理；两者是不同层面 |
