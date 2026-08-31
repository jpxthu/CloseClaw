# Run Health & 运行快照（Runtime Snapshot）

## 概述

Run Health 和运行快照（Runtime Snapshot）构成 Session 执行层的运行时安全网——确保 session 的每一次 compact、LLM request、tool 调用、spawn 都不会静默失败。← 日志：健康检查与异常检测结果

- **Run Health**：每次 turn 结束后，系统用硬规则和可选的质量门禁判定 session 当前是否健康。
- **运行快照（Runtime Snapshot）**：对 transcript 的毁坏性操作前，创建可回滚的快照。检测到异常后，系统可回滚到上一个安全状态。与持久化层的 SessionCheckpoint（元数据 + transcript 持久化）是不同概念。

二者互补：Health 负责检测异常并触发自动响应，运行快照负责保全现场并提供独立的可选回滚能力。合在一起，session 在任何时刻都能回答两个问题——"我还健康吗"和"出事了能回去吗"。

## 架构

Session 执行循环嵌入 health check：

1. Session turn 执行  ← 日志：健康检查检测开始
2. 硬规则检测（超时、空响应、结构异常、重试耗尽）  ← 日志：健康检查与异常检测结果（硬规则命中项）
   - 命中 -> unhealthy，按失败类别分流（见下方「不健康时的处理分流」）
   - 通过 -> 继续步骤 3
3. 可选 Hook 审查（按 agent 配置挂载 0-N 个轻量 LLM 质量门禁）  ← 日志：健康检查与异常检测结果（hook 判定结果）
   - 无 hook 配置 -> healthy，turn 正常结束，结果出站
   - 有 hook -> 并行调用 -> 任一标记异常 -> unhealthy（按失败类别分流），全部通过 -> healthy（turn 正常结束）
4. 重试闭环：unhealthy 分流中可重试/响应无效类别的重试成功后，重新发起 LLM 调用（回到当前 turn 的 LLM 请求阶段，复用同一上下文，不回到用户输入步骤）。重走步骤 2-3 的硬规则+Hook 检测形成闭环。

核心组件：

- **硬规则检测器**：纯代码逻辑，不依赖 LLM。检测超时、空响应、结构异常、重试耗尽。检测到即判 unhealthy。其中「重试耗尽」指可重试/响应无效类别的重试次数达上限（区别于「上下文彻底耗尽」的容量语义）。
- **Hook 审查器**：可选组件，按 agent 配置决定挂载 0 到 N 个。每个 hook 是固定 prompt 的轻量 LLM 调用，审查当前 turn 的输出质量（如是否只计划未执行、是否陷入工具调用循环）。Hook 调用与主对话隔离，不进入 transcript。
- **运行快照管理器**：在毁坏性操作前自动创建 transcript 快照，提供回滚能力。每个 session 最多保留 25 个快照，旧的自动淘汰。与持久化层的 CheckpointManager（管理 SessionCheckpoint 的读写缓存和持久化）职责不同。每个快照携带一份**快照元数据**（快照 ID、触发原因、创建时间、所属 session、完整性状态），元数据经**可注入的持久化接口**存入会话存储供回溯——快照正文（transcript 副本）与快照元数据分离，正文按文件管理、元数据按结构存储。**与 SessionCheckpoint 的边界**：快照元数据是毁坏性操作前的快照私有现场资产（经可注入接口存取、随淘汰回收），其完整性状态属快照自身生命周期；SessionCheckpoint 是会话主链的持久化资产（元数据 + transcript 持久化），两者存储语义与生命周期互不共享、状态机独立。
- **转录修改分类器**：所有修改 transcript 的代码路径必须声明操作类型。Session 层根据类型决定是否触发运行快照。

### 转录修改归类

所有修改 transcript 的操作归为三类，由 session 层统一管理：

| 操作 | 类型 | 触发快照 |
|------|------|----------------|
| 新增 user/assistant 消息 | 增量追加 | 否 |
| 新增 tool result 消息 | 增量追加 | 否 |
| Compaction（压缩对话历史） | 全量改写 | 是 |
| `/system` 指令修改 system prompt | 局部改写 | 是 |
| 从快照回滚 | 全量改写 | 是（回滚前自动创建 pre-rollback 快照，保留回滚前的现场） |

Session 层暴露一个携带操作类型声明的 transcript 修改通道，强制所有调用方声明本次修改属于增量追加、全量改写还是局部改写。未来新增操作类型也逃不掉这个约束。不修改 transcript 的操作（如 `/stop` 的纯运行时清理）不经过此通道，永不触发快照。

### 运行快照回滚方式

快照保留改写前的完整 transcript 文件副本。回滚时用副本覆盖当前文件。

### Hook 审查

Hook 是可选的轻量 LLM 质量门禁，按 agent 配置选择性启用：

- **挂载点**：session turn 结束、硬规则通过后
- **执行方式**：低温度、固定 prompt、1 turn 上限、0 工具
- **隔离**：不进入 transcript，不影响主对话的 system prompt
- **配置粒度**：agent 级别。agent 配置中定义启用的 hook 类型列表

| Hook 类型 | 检测目标 | 触发条件 |
|----------|---------|---------|
| `plan-check` | LLM 只输出了计划/承诺，没有执行 | turn 中无 tool call 且文本包含 promise 模式 |
| `loop-check` | 连续多 turn 调用同一工具且参数相似、无实质进展 | 工具调用历史模式匹配 |
| `progress-check` | 当前 turn 是否有可验证的推进 | 文件变化、tool result 差异 |

任何一个 hook 判定为异常 → session 判 unhealthy。

### Spawn 静默失败防护

子 agent spawn 场景有特殊的静默失败风险：子 agent 可能已完成但完成通知未成功投递、可能长时间挂起无产出。系统用三层防护应对：

**第一层：即时检测**。父 agent spawn 子 agent 后，每轮对话开始时系统注入当前活跃子 Session 摘要（详见 [session/README.md](README.md) Session 运行时）。父 agent 可据此判断子 agent 是否仍在执行中，是否继续等待。

**第二层：定时巡检**。Run Health 模块内置 AnnounceSweeper，定时扫描 spawn_tree 中的子 session 节点（含活跃与完成待回收，见 [spawn-tree.md](spawn-tree.md)），执行两类检查：  ← 日志：健康检查与异常检测结果（巡检发现：补推/僵死）

- **补推**：子 agent 已结束（四维执行状态全部归零且已产出最终 assistant 消息）但完成通知未成功送达父 Session → 补推完成通知，成功后回收节点。若父 Session 已归档则跳过补推并回收节点。
- **僵死检测**：子 agent 未结束且超过五分钟无新产出（无新 assistant 消息、无工具执行结果变化）→ 判定为僵死，自动终止该子 agent（级联终止其所有后代），向父 Session 注入僵死通知。若父 Session 已归档则跳过。

与 session-lifecycle 的 ArchiveSweeper（负责归档/清理，可配置间隔）是独立组件。

**第三层：启动恢复**。系统重启后扫描 pending_operations 中未完成的操作（spawn、工具调用、出站消息）。出站消息自动重投递；其余操作注入恢复通知，由 Agent 自行决策处理。详细机制见 session-recovery.md。这一层兜底进程崩溃导致的状态丢失。

### 失败类别与处理

unhealthy 不细分状态名，处理方式由失败类别决定：

| 失败类别 | 判定条件 | 处理方式 | 通知方式 |
|---------|---------|---------|---------|
| 可重试 | LLM API 瞬时错误、超时 | 退避重试，耗尽后升级为不可重试 | 耗尽时：注入 assistant 消息到 transcript |
| 响应无效 | 空响应、结构异常（响应解析失败、格式损坏）、纯推理无文本、纯计划不执行 | 给 LLM retry instruction（有限次），耗尽后通知用户 | 耗尽时：注入 assistant 消息到 transcript |
| 不可重试 | auth 失效、模型不存在、上下文彻底耗尽 | 立即通知用户，保留 session 状态 | 注入 assistant 消息到 transcript（含失败原因） |

## 数据流

### Turn 边界健康检测

```
1. 用户输入 -> LLM 调用 -> 解析响应 -> 执行工具 -> 更新 transcript
2. turn 结束后执行硬规则检测：超时、空响应、结构异常、重试耗尽
   - 命中 -> unhealthy -> 按下方的处理分流执行（见下方「不健康时的处理分流」）
   - 通过 -> 继续步骤 3
3. 检查 hook 配置
   - 无 hook -> healthy
   - 有 hook -> 并行调用各 hook
     - 任一 hook 标记异常 -> unhealthy（按失败类别分流，同硬规则命中）
     - 全部通过 -> healthy
```

不健康时的处理分流：

```
unhealthy
  - 可重试 -> 退避计数器递增 -> 重试
    - 重试成功 -> healthy -> 重新发起 LLM 调用（重走硬规则+Hook 检测形成闭环）
    - 耗尽 -> 升级为不可重试：注入 assistant 消息到 transcript -> 停止
  - 响应无效 -> retry instruction 注入 -> 重试
    - 重试成功 -> healthy -> 重新发起 LLM 调用（重走硬规则+Hook 检测形成闭环）
    - 耗尽 -> 注入 assistant 消息到 transcript -> 停止
  - 不可重试 -> 注入 assistant 消息到 transcript（含失败原因）-> 停止
```

### 运行快照创建与回滚

```
1. 毁坏性操作触发（compact、/system、回滚本身）
2. 创建快照：生成快照（含快照 ID、所属 session、触发原因、创建时间、初始完整性状态），保存 transcript 文件副本，并经可注入的持久化接口将快照元数据写入会话存储
3. 执行操作
   - 操作成功 -> 快照元数据标记为 complete
   - 操作失败 -> 系统检测到异常 -> 可触发回滚（见下方「回滚流程」）
```

### 回滚流程

触发来源：快照创建后操作失败的系统自动触发，或用户选择主动回滚。

```
1. 用户选择回滚（或系统自动触发）
2. 创建 pre-rollback 快照：保留回滚前的现场（回滚可撤销）
3. 加载目标快照，用备份文件替换 transcript
4. 记录回滚 audit（回滚目标快照、触发原因、时间）
5. Transcript 恢复完成 -> session 回到 healthy
```

## 模块关系

### 上游

| 模块 | 调用关系 |
|------|---------|
| Session 执行循环 | 每次 turn 结束后触发硬规则检测和 Hook 审查 |
| Compaction 流程 | 压缩前触发运行快照创建；压缩异常触发 unhealthy |
| Slash Command | `/system` 指令触发运行快照创建。`/stop` 不触发快照——它是纯运行时清理（cancel LLM、杀工具、清队列、级联停子 session），不修改 transcript，不属于毁坏性操作（见 [session-execution.md](session-execution.md) 停止流程） |

### 下游

| 模块 | 调用关系 |
|------|---------|
| Transcript 存储 | 运行快照创建和回滚直接操作 transcript 文件 |
| Persistence Service | 快照元数据（快照 ID、触发原因、创建时间、所属 session、完整性状态）经可注入的持久化接口存入会话存储 |
| LLM Provider | Hook 审查调用轻量 LLM（独立于主对话） |

### 无关

| 模块 | 说明 |
|------|------|
| Agent 配置 | Agent 是纯配置档案，不持有运行时健康状态。Hook 列表由 agent 配置决定，但健康状态由 session 运行时维护——"无关"指 Agent 自身不执行健康检测逻辑，而非"与健康检测的配置无关" |
| Permission 模块 | 健康检测和回滚不涉及权限判断 |
| Processor Chain / Renderer | 健康状态判定在出站渲染之前完成 |
| IM Adapter | 健康状态不通过消息路由传递 |
