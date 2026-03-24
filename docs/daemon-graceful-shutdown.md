# Graceful Shutdown Design

## Why Graceful Shutdown Matters

A naive `kill -TERM` sends SIGTERM to the process, which:
- **Immediately terminates** any in-flight agent tasks (lost work)
- **No chance to save** session state or checkpoints
- **Race conditions** — messages may be half-sent to IM adapters
- **Resource leaks** — temp files, locks, connections not released cleanly

## Shutdown States

```
                    ┌─────────────────────────────────────────┐
                    │                                         │
                    ▼                                         │
┌──────────┐   ┌──────────────┐   ┌───────────┐   ┌────────┴───┐
│  Running │──▶│ ShuttingDown │──▶│  Draining │──▶│   Stopped  │
└──────────┘   └──────────────┘   └───────────┘   └────────────┘
                    │                   │
                    │ SIGTERM/SIGINT    │ All components
                    │ or closeclaw stop  │ confirmed idle
                    │                   │
                    ▼                   ▼
              Stop accepting      Wait for in-flight
              new requests        tasks to complete
```

## Core Components

### 1. `ShutdownCoordinator`

Central authority that manages the shutdown lifecycle.

```rust
pub struct ShutdownCoordinator {
    state: AtomicU8,  // RUNNING, SHUTTING_DOWN, DRAINING, STOPPED
    busy_count: AtomicUsize,  // number of in-flight operations
    drain_complete: broadcast::Sender<()>,  // "all idle" signal
    stop_tx: oneshot::Sender<()>,  // final exit signal
}
```

**State machine (atomic, lock-free):**
- `RUNNING (0)` → normal operation
- `SHUTTING_DOWN (1)` → signal received, stop accepting new work
- `DRAINING (2)` → waiting for `busy_count` to reach 0
- `STOPPED (3)` → clean exit

### 2. `Component` trait

All daemon components implement:

```rust
pub trait GracefulComponent {
    /// Name for logging
    fn name(&self) -> &str;

    /// Called when shutdown begins — stop accepting new work
    async fn shutdown(&self) {
        // Default: do nothing, just register as idle
    }

    /// Called after all components signal idle — flush state
    async fn drain(&self) {
        // Default: drain immediately
    }

    /// Return true if this component has no in-flight work
    async fn is_idle(&self) -> bool {
        true
    }
}
```

### 3. Components That Need Graceful Shutdown

| Component | `shutdown()` | `drain()` | `is_idle()` |
|-----------|-------------|-----------|-------------|
| **Gateway** | Stop routing new messages | Flush pending outbound queue | Check sessions empty |
| **AgentRegistry** | Stop spawning new agents | Wait for agent processes to complete | Check all agents Stopped/Idle |
| **Feishu Adapter** | Stop polling / webhook receiver | Serialize outbox to disk (deliver on restart) | Check message queue empty |
| **Config Reloader** | Stop watching config files | Persist any dirty config | Always idle |

### 4. SIGTERM / SIGINT Handling

```rust
// In the daemon entry point:
tokio::spawn(async move {
    tokio::signal::ctrl_c().await.unwrap();
    tracing::info!("Received Ctrl+C, initiating graceful shutdown...");
    coordinator.initiate_shutdown().await;
});

// Also handle SIGTERM (for `kill` and `closeclaw stop`)
tokio::spawn(async move {
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
    tracing::info!("Received SIGTERM, initiating graceful shutdown...");
    coordinator.initiate_shutdown().await;
});
```

### 5. `closeclaw stop` Flow (Updated)

```
User runs: closeclaw stop
         │
         ▼
Read ~/.closeclaw/daemon.pid
         │
         ▼
Send SIGTERM to daemon PID  ──────────────────┐
         │                                           │
         ▼                                           │
Daemon receives SIGTERM                           │
         │                                           │
         ▼                                           │
ShutdownCoordinator.initiate_shutdown()          │
         │                                           │
         ├─▶ Gateway.shutdown()      ── stop routing new messages
         ├─▶ AgentRegistry.shutdown() ── stop spawning, wait agents
         ├─▶ IMAdapter.shutdown()    ── stop receiving
         │
         ▼
Wait for busy_count == 0 (with timeout)
         │
         ▼
All components.drain()  ── flush queues, save state
         │
         ▼
Close log files, remove PID file
         │
         ▼
Process exits with code 0  ◀──────────────────────┘
```

### 6. Timeout and Force Kill

If `drain()` exceeds a timeout (default: **30 seconds**), log a warning and:
1. Send SIGTERM again to all child agent processes
2. Wait 5 more seconds
3. If still not stopped → SIGKILL

## Implementation Plan

### Phase 1: Core infrastructure (new file: `src/daemon/shutdown.rs`)
- `ShutdownCoordinator` with atomic state machine
- `Component` trait
- `ShutdownHandle` passed to each component

### Phase 2: Integrate into existing components
- Add `is_idle()` checks to `Gateway`, `AgentRegistry`, adapters

### Phase 3: Update `Run` command
- Spawn SIGTERM/SIGINT handlers
- Pass `ShutdownHandle` to all components
- Implement the drain loop with timeout

### Phase 4: Update `Stop` command
- Remove the crude `kill` approach
- Instead: read PID, send SIGTERM (let the graceful flow handle it)
- Poll daemon.pid deletion as confirmation

## Decisions

1. **Checkpoint frequency** — drain 时**一次性落盘**，不频繁 checkpoint。Agent 状态运行中在内存，drain 开始时触发一次快照落盘即可。

2. **超时策略** — 全局超时 + 组件可声明自己的 drain_timeout：
   - `closeclaw stop` 默认 **30s 全局超时**
   - 每个组件可声明 `drain_timeout`（默认 10s），超时后组件自行强制落盘
   - 30s 后无论状态 → SIGKILL 强制终止

3. **IM adapter outbox** — 保留到**下次启动时再发送**。Shutdown 时将 outbox 序列化到磁盘，重启后检测并投递。

4. **Child agent processes** — **要等**。提示用户正在等待任务完成，提供 `--force/-f` 选项直接 SIGKILL。

## `closeclaw stop` Modes

```bash
closeclaw stop          # 优雅关闭：等任务完成（最多30s），然后 drain
closeclaw stop -f      # 强制关闭：SIGKILL 立即终止（数据可能丢失）
```

## `closeclaw stop` Flow

```
User runs: closeclaw stop
         │
         ▼
Read ~/.closeclaw/daemon.pid
         │
         ▼
Send SIGTERM to daemon PID ─────────────────────┐
         │                                       │
         ▼                                       │
Daemon receives SIGTERM                          │
         │                                       │
         ▼                                       │
ShutdownCoordinator.initiate_shutdown()          │
         │                                       │
         ├─▶ Gateway.shutdown()        ── 停止路由新消息，冲洗 outbox
         ├─▶ AgentRegistry.shutdown()  ── 停止 spawn，等 agent 完成
         ├─▶ FeishuAdapter.shutdown() ── 停止接收，序列化 outbox 到磁盘
         │   （符合 Decision #3：drain 时一次性落盘，重启后检测并投递）
         │
         ▼
busy_count 轮询（最多 30s）                     │
         │                                       │
         ├─ 每 10s 打印进度: "Waiting for N tasks..."   │
         │                                       │
         ▼                                       │
All components.drain() ── 全部 idle 后统一触发状态落盘
         │                                       │
         ▼                                       │
关闭日志、移除 PID file                          │
         │                                       │
         ▼                                       │
exit(0) ◀──────────────────────────────────────┘

If timeout exceeded at any point:
         │
         ▼
Log warning + SIGKILL to all child processes
         │
         ▼
exit(1)
```
