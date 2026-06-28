# Daemon 关闭

## 概述

Daemon 关闭流程在收到操作系统信号后触发，由 ShutdownHandle 统一协调，按 Phase 0（信号接收与模式判定）+ Phase 1–7（停止流程）有序关闭所有组件。支持 graceful（优雅等待）和 forceful（强制终止）两种模式，用户可在 graceful 期间随时升级为 forceful。关闭过程中保证数据不丢失——未完成的操作通过 recovery 机制在下次启动时恢复。

## 架构

### ShutdownHandle

ShutdownHandle 是关闭流程的中央协调器，在 Daemon 启动时创建，持有以下核心状态：

- **关闭门控标志**：关闭开始后置为拒绝状态，所有处理组件在开始处理新操作前检查，若已关闭则拒绝
- **活跃操作计数**：原子计数器。处理组件处理工作前递增，完成后递减
- **drain 等待**：阻塞等待活跃操作计数降至零或超时返回。用于 Phase 1 等待活跃操作（消息处理、工具执行、子 session）排空。超时时间可配置（默认 30s），超时后返回剩余活跃操作计数，调用方正常进入 Phase 2。超时不触发 forceful 升级——仅作为正常流转边界，剩余活跃操作由 Phase 2 的 session 停止流程处理
- **drain 状态查询**：查询当前活跃操作计数和剩余等待项

注册活跃操作计数的组件：

| 组件 | 递增时机 | 递减时机 |
|------|---------|---------|
| Gateway 消息处理循环 | 消息出队开始处理 | 响应发送完成 |
| 异步工具执行 | 工具进程创建 | 进程退出 |
| 子 session 处理 | 子 session 创建 | 子 session 结果注入父 session |

ShutdownHandle 不管理后台任务（ArchiveSweeper、Skill Watcher、Config Hot Reload、DreamingScheduler 等）——这些有自己的停止接口，不属于活跃操作计数范畴。

### 双模关闭

| 模式 | 触发条件 | 核心行为 |
|------|---------|---------|
| Graceful | 首次 SIGTERM | 等 LLM 流结束、等工具执行完，用户可见进度，可随时升级 |
| Forceful | 重复 SIGTERM 或 SIGINT | 立即 kill 工具进程和 LLM 请求，依赖 recovery 恢复 |

### 升级路径

任一情况下触发 graceful → forceful 升级：
- 收到第二次 SIGTERM 或 SIGINT
- 用户通过进度通知选择"强制关闭"

升级后：已有序停止的 session 直接持久化，未停止的切换为 forceful 模式继续。

### Session 停止策略

Daemon 不感知 session 树结构和停止顺序——全部委托 SessionManager 统一处理。SessionManager 构建 session 父子树，叶子→根顺序、并发停止同级 session。

Graceful 模式下单 session 的停止行为（详见 [session-execution.md](../session/session-execution.md)）：

| Session 当前状态 | 行为 |
|-----------------|------|
| LLM 流式输出中 | 等待流结束。结束后 assistant 消息若含工具调用请求，将工具调用写入待完成操作记录并持久化会话检查点，不执行工具。下次启动由恢复机制注入工具失败结果，LLM 自行决策 |
| 工具执行中 | 等待工具完成。完成后工具结果写入对话记录、清除待完成操作记录、持久化会话检查点。不触发新一轮 LLM turn——下次用户消息自然衔接 |
| Idle | 直接持久化 |

Daemon 不依赖 session 停止流程的硬超时自动升级——工具执行时间不可预测。Daemon 可从 SessionManager 查询当前停止进度，汇总为进度通知展示给用户，由用户决定继续等待还是升级为 forceful。

## 数据流

### 关闭全流程

```
信号到达
  ↓
Phase 0：信号接收 & 模式判定
  关闭门控标志置为拒绝状态（组件拒绝新操作，两种模式通用）
  ├── SIGTERM（首次）→ Graceful 模式
  └── SIGTERM（重复）/ SIGINT → Forceful 模式
  ↓
Phase 1：入站停摆 + Drain
  ├── IM Adapters 关闭入站（websocket 断开、webhook 退订）
  └── 调用 drain 等待 in-flight 消息处理完毕
      ├── 全部排空 → 进入 Phase 2
      └── 超时（可配，默认 30s）→ 记录剩余活跃操作计数，进入 Phase 2
  ↓
Phase 2：Session 停止
  委托 SessionManager 统一关闭所有 session：
  ├── 构建 session 父子树
  ├── 叶子→根顺序，同级并发停止
  ├── Graceful：按 session 当前状态分别处理（见架构节）
  └── Forceful：立即 kill 工具进程、cancel LLM 请求。LLM 流被中断后当前 assistant 消息片段丢弃，不写入对话记录。会话检查点中 pending_operations 残留，下次启动由恢复扫描处理
  ↓
Phase 3：后台任务停止
  ├── ArchiveSweeper：取消定时器，给当前扫描短 grace period 后强制停止
  ├── Skill Watcher：取消
  ├── Config Hot Reload：取消
  └── DreamingScheduler：取消定时器
  ↓
Phase 4：最终持久化
  └── 通过 SessionManager 执行全局 fsync 同步，确保 Phase 2 所有 session 的持久化写入已安全落盘（forceful 模式中未持久化的 session 在此阶段兜底持久化）
  ↓
Phase 5：出站关闭
  ├── IM Adapters 关闭出站连接
  └── Gateway 清理路由表、processor 注册表
  ↓
Phase 6：存储关闭
  └── 关闭存储连接，释放文件句柄
  ↓
Phase 7：退出
  ├── 异常 session → 日志告警
  └── 进程退出
```

### 用户进度通知

Graceful 关闭期间，向用户发送实时状态，收集各组件状态汇总输出：

```
⏳ 正在优雅关闭...

活跃 Session：
  • session-1 — LLM 流式输出中，已等待 3s
  • session-2 — 工具执行中：make build 编译任务，已运行 12s
  • session-3 — 已就绪

[继续等待] [强制关闭]
```

用户可选择等待或升级为 forceful。通知在 Session 停止阶段开始时发送，有状态变化时更新（session 完成、新 session 开始停止等）。进度通知通过 IM Adapters 出站通道发送——Phase 1 仅关闭入站，出站在 Phase 5 才关闭，Phase 2-4 期间出站通道可用。

### Recovery 衔接

关闭流程与 [session-recovery.md](../session/session-recovery.md) 的衔接点：

**Graceful 关闭后重启**：
- LLM 流结束后工具调用未执行：会话检查点中已写入待完成操作记录。重启时恢复机制扫描到待完成操作非空 → 标记为异常 → 注入恢复通知（系统消息，列出未完成任务摘要）和工具失败结果到对话流 → LLM 自行决策重试
- 工具执行完毕、未做新一轮 LLM：工具结果已写入对话记录，待完成操作已清除。会话检查点干净，下次用户消息触发 LLM turn 时 LLM 自然看到此前工具结果继续处理

**Forceful 关闭后重启**：
- 工具被终止：会话检查点中待完成操作记录残留。重启时恢复机制扫描到待完成操作非空 → 标记为异常 → 注入恢复通知和工具失败结果到对话流 → LLM 自行决策。工具副作用不可控（编译到一半等），这是用户选择 forceful 时已知的代价

## 模块关系

### 上游

- **操作系统**：通过信号（SIGTERM、SIGINT）触发关闭
- **用户**：通过进度通知选择升级为 forceful

### 下游

- **ShutdownHandle**：Daemon 创建并持有，调用门控设置、drain 等待、状态查询
- **SessionManager**：委托统一关闭所有 session（含最终持久化），传入模式参数
- **IM Adapters**：关闭入站/出站连接
- **Gateway**：清理路由表和注册表
- **SqliteStorage**：关闭存储连接
- **后台任务**（ArchiveSweeper、Skill Watcher、Config Hot Reload、DreamingScheduler）：逐一停止

### 无关

- **LLM Provider**（无调用关系）：关闭流程通过 SessionManager 间接影响 LLM 请求，不直接调用 Provider
- **Processor Chain**（无调用关系）：处理器链由 Gateway 管理，关闭时随 Gateway 清理
