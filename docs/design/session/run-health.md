# Run Health & 运行快照（Runtime Snapshot）

## 概述

Run Health 和 Checkpoint 构成 Session 执行层的运行时安全网——确保 session 的每一次 compact、LLM request、tool 调用、spawn 都不会静默失败。

- **Run Health**：每次 turn 结束后，系统用硬规则和可选的质量门禁判定 session 当前是否健康。
- **运行快照（Runtime Snapshot）**：对 transcript 的毁坏性操作前，创建可回滚的快照。检测到异常后，系统可回滚到上一个安全状态。与持久化层的 SessionCheckpoint（元数据 + transcript 持久化）是不同概念。

二者互补：Health 负责检测异常，运行快照负责保全现场。合在一起，session 在任何时刻都能回答两个问题——"我还健康吗"和"出事了能回去吗"。

## 架构

Run Health 和 Checkpoint 嵌入 session 执行循环，在 turn 边界工作：

```
Session turn 执行
  ↓
[硬规则检测]  ← 超时、空响应、结构异常、重试耗尽
  ↓
[可选 Hook 审查]  ← 按 agent 配置挂载的轻量 LLM 质量门禁
  ↓
判决：healthy / unhealthy
  ↓
unhealthy → 按失败类别处理（退避重试 / 通知用户 / 回滚快照）
```

核心组件：

- **硬规则检测器**：纯代码逻辑，不依赖 LLM。检测超时、空响应、结构异常、重试耗尽。检测到即判 unhealthy。
- **Hook 审查器**：可选组件，按 agent 配置决定挂载 0 到 N 个。每个 hook 是固定 prompt 的轻量 LLM 调用，审查当前 turn 的输出质量（如是否只计划未执行、是否陷入工具调用循环）。Hook 调用与主对话隔离，不进入 transcript。
- **运行快照管理器**：在毁坏性操作前自动创建 transcript 快照，提供回滚能力。每个 session 最多保留 25 个快照，旧的自动淘汰。与持久化层的 CheckpointManager（管理 SessionCheckpoint 的读写缓存和持久化）职责不同。
- **转录修改分类器**：所有修改 transcript 的代码路径必须声明操作类型。Session 层根据类型决定是否触发运行快照。

### 转录修改归类

所有修改 transcript 的操作归为三类，由 session 层统一管理：

| 操作 | 类型 | 触发快照 |
|------|------|----------------|
| 新增 user/assistant 消息 | 增量追加 | 否 |
| 新增 tool result 消息 | 增量追加 | 否 |
| Compaction（压缩对话历史） | 全量改写 | 是 |
| `/system` 指令修改 system prompt | 局部改写 | 是 |
| 从快照回滚 | 全量改写 | 是（回滚前自动打一个） |

Session 层暴露一个携带操作类型声明的 transcript 修改通道，强制所有调用方声明本次修改属于增量追加、全量改写还是局部改写。未来新增操作类型也逃不掉这个约束。

### 运行快照回滚方式

- **增量场景**：transcript 为 append-only JSONL。快照记录当时的 leaf entry id。回滚时截断该 id 之后的全部行。
- **改写场景**：compaction 或 `/system` 重写了 transcript 文件。快照保留改写前的完整文件副本。回滚时用副本覆盖当前文件。

### Hook 审查

Hook 是可选的轻量 LLM 质量门禁，按 agent 配置选择性启用：

- **挂载点**：session turn 结束、硬规则通过后
- **执行方式**：低温度、固定 prompt、1 turn 上限、0 工具
- **隔离**：不进入 transcript，不影响主对话的 system prompt
- **配置粒度**：agent 级别。agent 配置中定义启用的 hook 列表及其参数

| Hook 类型 | 检测目标 | 触发条件 |
|----------|---------|---------|
| `plan-check` | LLM 只输出了计划/承诺，没有执行 | turn 中无 tool call 且文本包含 promise 模式 |
| `loop-check` | 连续多 turn 调用同一工具且参数相似、无实质进展 | 工具调用历史模式匹配 |
| `progress-check` | 当前 turn 是否有可验证的推进 | 文件变化、tool result 差异 |

任何一个 hook 判定为异常 → session 判 unhealthy。

### Spawn 静默失败防护

子 agent spawn 场景有特殊的静默失败风险：子 agent 可能已完成但 announce 未成功投递、父 agent 可能未正确 yield 而继续空转。系统用三层防护应对：

**第一层：即时检测**。父 agent spawn 子 agent 后如果下一个 turn 没有调用 sessions_yield 而是继续做其他操作，系统注入提醒：「你有 N 个子 agent 仍在运行，建议 yield 等待结果」。这一层在 turn 边界触发，几乎零延迟。

**第二层：定时巡检**。Run Health 模块内置 AnnounceSweeper，每 60 秒扫描所有活跃子 agent session。扫描逻辑：检查子 agent session 是否已结束——session 结束的判定标准是三维执行状态全部归零（LLM 状态 Idle、无前台工具、无后台工具、子孙 session 全部完成），与 session-execution.md 的整体状态判定一致。

AnnounceSweeper 只负责投递，不判断任务质量：session 结束即产生 announce，Sweeper 确保它送达父 session。子 agent 任务是否满意、是否需要重试——由父 agent 收到 announce 后自主决策。这一层兜底第一层遗漏的投递失败。与 session-lifecycle 的 ArchiveSweeper（负责归档/清理，可配置间隔）是独立组件。

**第三层：启动恢复**。系统重启后扫描 pending_operations 中未完成的操作（spawn、工具调用、出站消息）。出站消息自动重投递；其余操作注入恢复通知，由 Agent 自行决策处理。详细机制见 session-recovery.md。这一层兜底进程崩溃导致的状态丢失。

### 失败类别与处理

unhealthy 不细分状态名，处理方式由失败类别决定：

| 失败类别 | 判定条件 | 处理方式 |
|---------|---------|---------|
| 可重试 | LLM API 瞬时错误、超时 | 退避重试，耗尽后升级为不可重试 |
| 响应无效 | 空响应、纯推理无文本、纯计划不执行 | 给 LLM retry instruction（有限次），耗尽后通知用户 |
| 不可重试 | auth 失效、模型不存在、上下文彻底耗尽 | 立即通知用户，保留 session 状态 |

## 数据流

### Turn 边界健康检测

```
用户输入 → LLM 调用 → 解析响应 → 执行工具 → 更新 transcript
                                                      ↓
                                             [turn 结束]
                                                      ↓
                                          硬规则检测 ──→ 命中？ → unhealthy → 按类别处理
                                              │
                                              ↓ 通过
                                          有 hook 配置？
                                              │
                                       ┌──────┴──────┐
                                       │ 无           │ 有
                                       ↓              ↓
                                    healthy      [并行调用 hook]
                                                    ↓
                                         任一 hook flag？
                                          │         │
                                         是         否
                                          ↓         ↓
                                      unhealthy  healthy
```

不健康时的处理分流：

```
unhealthy
  ├─ 可重试 → 退避计数器递增 → 重试
  │   ├─ 重试成功 → healthy → 继续
  │   └─ 耗尽 → 通知用户 → 停止
  │
  ├─ 响应无效 → retry instruction 注入 → 重试
  │   └─ 耗尽 → 通知用户 → 停止
  │
  └─ 不可重试 → 通知用户（含原因）→ 停止
```

### 运行快照创建与回滚

```
毁坏性操作触发（compact、/system、回滚本身）
  ↓
[创建快照]：copy transcript / 记录 leaf id → 标记触发原因和时间
  ↓
执行操作
  ↓
操作成功 → 快照标记为 complete
操作失败 → 系统检测到 unhealthy → 可回滚到快照恢复 transcript
  ↓
可选操作：load 快照 → 原子性替换 transcript → 记录回滚 audit
```

### 回滚流程

```
用户选择回滚（或系统自动触发）
  ↓
[创建 pre-rollback 快照]：保留回滚前的现场（可 undo 回滚）
  ↓
加载目标快照
  ├─ 增量快照 → 截断 transcript 到快照 leaf id
  └─ 改写快照 → 用备份文件替换 transcript
  ↓
transcript 恢复完成 → session 回到 healthy
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Session 执行循环 | 每次 turn 结束后触发 hard rule 检测和 hook 审查 |
| Compaction 流程 | 压缩前触发运行快照创建；压缩异常触发 unhealthy |
| Slash Command | `/system` 指令触发运行快照创建 |
| Gateway | 外部用户中断（`/stop`）可能触发运行快照保存 |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Transcript 存储 | 运行快照创建和回滚直接操作 transcript 文件 |
| Persistence Service | 运行快照元信息（id、原因、时间）存入 session store |
| LLM Provider | Hook 审查调用轻量 LLM（独立于主对话） |

### 无关

| 模块 | 说明 |
|------|------|
| Agent 配置 | Agent 是纯配置档案，不持有运行时健康状态。Hook 列表由 agent 配置决定，但健康状态本身由 session 运行时维护 |
| Permission 模块 | 健康检测和回滚不涉及权限判断 |
| Processor Chain / Renderer | 健康状态判定在出站渲染之前完成 |
| IM Adapter | 健康状态不通过消息路由传递 |
