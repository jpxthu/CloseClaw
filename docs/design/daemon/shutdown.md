# Daemon 关闭

## 概述

Daemon 关闭流程在收到操作系统信号后触发，由 ShutdownHandle 统一协调，按 Phase 0（信号接收与模式判定）+ Phase 1–7（停止流程）有序关闭所有组件。支持 graceful（优雅等待）和 forceful（强制终止）两种模式，Owner 可在 graceful 期间随时升级为 forceful。关闭过程中保证已持久化的数据不丢失——未完成的操作通过 recovery 机制在下次启动时恢复（forceful 的已知代价见「双模关闭」）。

## 架构

### ShutdownHandle

ShutdownHandle 是关闭流程的中央协调器，在 Daemon 启动时创建，包含以下核心组成：

- **关闭门控标志**：关闭开始后置为拒绝状态，所有处理组件在开始处理新操作前检查，若已关闭则拒绝
- **活跃操作计数**：原子计数器。处理组件处理工作前递增，完成后递减
- **drain 等待**：阻塞等待活跃操作计数降至零或超时返回。用于 Phase 1 等待活跃操作（消息处理、工具执行、子 session）排空。超时时间可配置（默认 30s），超时后返回剩余活跃操作计数，调用方正常进入 Phase 2。超时不触发 forceful 升级——仅作为正常流转边界，剩余活跃操作由 Phase 2 的 session 停止流程处理
- **drain 状态查询**：查询当前活跃操作计数和剩余等待项

ShutdownHandle 实现跨模块关停接口 ShutdownSignal（定义见 [common/core-traits](../common/core-traits.md)）——llm 模块经由该接口查询关停状态、忙计数并接收 graceful→forceful 升级，无需依赖 Daemon 内部结构。

注册活跃操作计数的组件：

| 组件 | 递增时机 | 递减时机 |
|------|---------|---------|
| Gateway 消息处理循环 | 消息出队开始处理 | 响应发送完成 |
| 异步工具执行 | 工具进程创建 | 进程退出 |
| 子 session 处理 | 子 session 创建 | 子 session 结果注入父 session |

ShutdownHandle 不管理后台任务——这些有自己的停止接口，不属于活跃操作计数范畴。Daemon 级后台任务的完整清单见「架构 / Daemon 级后台任务清单」。

### 双模关闭

| 模式 | 触发条件 | 核心行为 |
|------|---------|---------|
| Graceful | 首次收到 SIGTERM 或 SIGINT | 等 LLM 流结束、等工具执行完，Owner 可见进度，可随时升级 |
| Forceful | 关闭进行中再次收到任一信号 | 立即终止工具进程和 LLM 请求，依赖 recovery 恢复 |

### 升级路径

任一情况下触发 graceful → forceful 升级：
- 关闭进行中再次收到 SIGTERM 或 SIGINT
- Owner 通过进度通知选择"强制关闭"

升级后：已有序停止的 session 直接持久化，未停止的切换为 forceful 模式继续。升级是单向迁移：已完成阶段的成果不受影响，进行中与未开始的阶段按 forceful 语义执行——Phase 1 的 drain 等待立即中断返回，后续阶段跳过一切等待直接快速终止。

### Daemon 级后台任务清单

后台任务清单以 [README.md](README.md) 启动依赖表为权威来源，当前共 5 个：ArchiveSweeper、AnnounceSweeper、PlanArchiveSweeper、DreamingScheduler、Config Hot Reload。新增后台任务时必须同时更新启动依赖表与本节。

各任务的用途与调度详见对应模块文档：会话归档见 [session/session-lifecycle.md](../session/session-lifecycle.md)，完成通知补推与僵死检测见 [session/run-health.md](../session/run-health.md)，plan 归档规则见 [mode/README.md](../mode/README.md)，记忆挖掘见 [memory/dreaming.md](../memory/dreaming.md)，配置监听见 [config/hot-reload.md](../config/hot-reload.md)。

### Session 停止策略

Daemon 不感知 session 树结构和停止顺序——全部委托 SessionManager 统一处理。SessionManager 构建 session 父子树，叶子→根顺序、并发停止同级 session。

Session 可能同时在多个活跃维度上处于活跃状态（四维定义见 [session 需求 §F11](../../requirements/session.md)，与 [requirements/daemon.md](../../requirements/daemon.md) §F2 的关闭等待条件对应）。Graceful 停止按「任一维度活跃则继续等待」处理，各维度独立完成，全部就绪后才执行该 session 的最终持久化。

Graceful 模式下各维度的等待行为（SessionManager 侧的状态机详见 [session-execution.md](../session/session-execution.md)）：

| 活跃维度 | Graceful 行为 |
|---------|--------------|
| LLM 流式输出中 | 等待流结束。结束后 assistant 消息若含工具调用请求，将工具调用写入待完成操作记录并持久化会话检查点，不执行工具（重启后的衔接见「Recovery 衔接」） |
| 同步工具执行中 | 等待工具完成。完成后工具结果写入对话记录、清除待完成操作记录、持久化会话检查点。不触发新一轮 LLM turn——后续对话推进由下次 User 消息到来时自然衔接（若期间经历重启，则为重启恢复后的首条 User 消息） |
| 后台任务（Agent 异步调用的工具） | 工具启动即登记进活跃操作计数（见 ShutdownHandle），drain 阶段统一等待；进入 Phase 2 仍在运行的，与其他维度并行等待其自然结束，结果照常写入对话记录。不设硬超时——长耗时后台任务是否继续等待由 Owner 经进度通知决策 |
| 子 session 未完成 | 由树序保证：叶子先于父节点停止，父 session 待其全部直接子 session 完成停止且产出已注入后才进入自身收尾 |
| 已就绪（四维均否） | 直接持久化 |

Daemon 不依赖 session 停止流程的硬超时自动升级——工具执行时间不可预测。Daemon 可从 SessionManager 查询当前停止进度，汇总为进度通知展示给 Owner，由 Owner 决定继续等待还是升级为 forceful。

## 数据流

### 关闭全流程

关闭门控标志在 Phase 0 置为拒绝状态，两种模式通用。后续各阶段按序执行。

1. **Phase 0：信号接收 & 模式判定**
   - 首次收到信号（SIGTERM 或 SIGINT）→ Graceful 模式，发送关闭启动通知告知 Owner 系统正在关闭（drain 结束后将展示 session 进度详情）
   - 关闭进行中再次收到任一信号 → Forceful 模式

2. **Phase 1：入站停摆 + Drain**
   - IM Adapters 关闭入站（websocket 断开、webhook 退订）
   - 调用 drain 等待在途消息处理完毕
     - 全部排空：进入 Phase 2
     - 超时（可配，默认 30s）：记录剩余活跃操作计数，进入 Phase 2

3. **Phase 2：Session 停止**
   委托 SessionManager 统一关闭所有 session：
   - 构建 session 父子树
   - 叶子→根顺序，同级并发停止
   - Graceful：按 session 当前状态分别处理（见架构节）
   - Forceful：立即终止工具进程、取消 LLM 请求。LLM 流被中断后当前 assistant 消息片段丢弃，不写入对话记录。会话检查点中待完成操作记录残留，下次启动由恢复扫描处理

4. **Phase 3：后台任务停止**

   按「架构 / Daemon 级后台任务清单」停止全部 5 个后台任务。

   四个定时器型扫描任务（ArchiveSweeper、AnnounceSweeper、PlanArchiveSweeper、DreamingScheduler）使用统一停止机制：取消定时器，给当前扫描迭代短 grace period（等待当前迭代完成，最长 10 秒），超时强制停止。

   | 后台任务 | 副作用处理 |
   |---------|-----------|
   | ArchiveSweeper | 强制停止时若残留未完成的归档操作，由启动恢复扫描处理，不产生持久副作用 |
   | AnnounceSweeper | 补推与僵死检测均为幂等巡检，中断的扫描周期不影响正确性；未完成的补推由下次启动后的定时巡检继续收敛 |
   | PlanArchiveSweeper | 归档是单文件移动操作，中断即视为未发生，无持久副作用 |
   | DreamingScheduler | 强制停止时当前迭代中止。产出只写入 MEMORY.md 与 Dream Diary 文件、不回写 SQLite，源 event 保留未消费，中断的迭代在下次调度时安全重跑并经语义去重收敛 |
   | Config Hot Reload | 取消文件监听即停。重载为内存操作（校验 → 更新内存 → 事件通知），中断后停留在最近一次有效配置，无持久副作用 |

   确认全部后台任务的同步停止流程执行完毕、遗留项已按上表移交启动恢复或下次巡检后进入下一阶段。

5. **Phase 4：最终持久化**
   通过 SessionManager 执行全局 fsync 同步，确保 Phase 2 所有 session 的持久化写入已安全落盘（forceful 模式中未持久化的 session 在此阶段兜底持久化）

6. **Phase 5：出站关闭**
   - IM Adapters 关闭出站连接
   - Gateway 清理路由表、processor 注册表

7. **Phase 6：存储关闭**
   关闭存储连接，释放文件句柄

8. **Phase 7：退出**
   - 异常 session → 日志告警
   - 进程退出

### Owner 进度通知

Graceful 关闭期间，向 Owner 发送实时状态，收集各组件状态汇总输出：

```
⏳ 正在优雅关闭...

活跃 Session：
  • session-1 — LLM 流式输出中，已等待 3s
  • session-2 — 工具执行中：make build 编译任务，已运行 12s
  • session-3 — 子任务进行中（2 个子 session 未完成），已等待 5s

[继续等待] [强制关闭]
```

Owner 可选择等待或升级为 forceful。关闭启动时（Phase 0 模式判定后）立即发送首次通知，告知 Owner 系统正在关闭。进入 Session 停止阶段后切换为进度详情通知，有状态变化时更新（session 完成、新 session 开始停止等）。每个活跃 session 的展示文案取其当前活跃维度：LLM 流式输出中 / 工具执行中 / 子任务进行中（仅有子 session 维度活跃时）；就绪的 session 完成持久化后即从列表移除。

会话停止期间若长时间无状态变化，系统每 30 秒发送一次心跳通知，让 Owner 确认系统仍在关闭中而非卡死。心跳内容为简化格式，不逐条列出 session 详情：

```
⏳ 仍在关闭中，已等待 27s

[继续等待] [强制关闭]
```

心跳仅在 Phase 2（Session 停止）阶段生效，所有 session 停止完毕后自动停止。

进度通知通过 IM Adapters 出站通道发送——Phase 1 仅关闭入站，出站在 Phase 5 才关闭，Phase 2-4 期间出站通道可用。

### Recovery 衔接

关闭流程与 [session-recovery.md](../session/session-recovery.md) 的衔接点：

**Graceful 关闭后重启**：
- LLM 流结束后工具调用未执行：会话检查点中已写入待完成操作记录。重启时恢复机制扫描到待完成操作非空 → 标记为异常 → 注入恢复通知（系统消息，列出未完成任务摘要）和工具失败结果到对话流 → LLM 自行决策重试
- 工具执行完毕、未做新一轮 LLM：工具结果已写入对话记录，待完成操作已清除。会话检查点干净，下次 User 消息触发 LLM turn 时 LLM 自然看到此前工具结果继续处理

**Forceful 关闭后重启**：
- 工具被终止：会话检查点中待完成操作记录残留。重启时恢复机制扫描到待完成操作非空 → 标记为异常 → 注入恢复通知和工具失败结果到对话流 → LLM 自行决策。工具副作用不可控（编译到一半等），这是 Owner 选择 forceful 时已知的代价

## 模块关系

### 上游

- **操作系统**：通过信号（SIGTERM、SIGINT）触发关闭
- **Owner**：通过进度通知选择升级为 forceful

### 下游

- **ShutdownHandle**：Daemon 创建并持有，调用门控设置、drain 等待、状态查询
- **SessionManager**：委托统一关闭所有 session（含最终持久化），传入模式参数
- **IM Adapters**：关闭入站/出站连接
- **Gateway**：清理路由表和注册表
- **llm 模块**：经 ShutdownSignal 接口向其暴露关停状态查询、忙计数与 graceful→forceful 升级（见 [common/core-traits](../common/core-traits.md)）
- **SqliteStorage**：关闭存储连接
- **后台任务**：逐一停止（完整清单见「架构 / Daemon 级后台任务清单」，共 5 个：ArchiveSweeper、AnnounceSweeper、PlanArchiveSweeper、DreamingScheduler、Config Hot Reload）

### 无关

- **LLM Provider**（不直接调用 Provider API）：对话内的 LLM 请求经由 SessionManager 等待或取消；关停状态感知走上方 ShutdownSignal 接口
- **Processor Chain**（无调用关系）：处理器链由 Gateway 管理，关闭时随 Gateway 清理
