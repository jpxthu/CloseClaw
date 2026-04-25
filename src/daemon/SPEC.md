# daemon 模块规格说明书

## 模块概述

daemon 是 CloseClaw 的后台服务进程，负责启动和协调所有组件（Gateway、AgentRegistry、PermissionEngine、ChatServer、AuditLogger），并在接收到 SIGINT/SIGTERM 信号时执行优雅关闭。

启动流程按顺序完成：加载 .env → 初始化 SqliteStorage（失败则 start() 返回 Err）→ 初始化 JsonSessionConfigProvider（文件不存在则 warn + 硬编码默认值）→ 创建 ArchiveSweeper 并 spawn 后台任务 → 初始化 PermissionEngine → 初始化 AgentRegistry → 初始化 Gateway（注入 storage）→ 注册 Feishu 适配器 → 启动 ShutdownCoordinator → 注册 LLM Providers → 启动 Chat TCP Server → 启动 AuditLogger。

SqliteStorage 的 data_dir 直接使用 `config_dir` 参数，不做 fallback 计算。

关闭流程采用状态机管理：Running → ShuttingDown → Draining → Stopped，等待所有在途操作完成后退出，超时强制终止；Daemon shutdown 时通过 `sweeper_shutdown_tx` 通知 ArchiveSweeper 优雅退出。

## 公开接口

### 构造

- **`Daemon::start(config_dir)`** — 异步构造器，按启动流程初始化所有组件，返回完整的 Daemon 实例

### 主操作

- **`Daemon::run()`** — 阻塞当前协程直到收到 SIGINT/SIGTERM 信号，然后触发优雅关闭（Unix 平台监听两种信号，非 Unix 平台仅支持 Ctrl+C）
- **`Daemon::evaluate_with_audit(request)`** — 执行权限评估并将结果记录到审计日志
- **`Daemon::shutdown_audit()`** — 关闭审计日志刷新器，确保持续的审计事件被写入

### 查询

- `Daemon::log_agent_start(agent_id, model)` — 记录 Agent 启动审计事件（异步，不阻塞）
- `Daemon::log_agent_stop(agent_id)` — 记录 Agent 停止审计事件（异步，不阻塞）
- `Daemon::log_agent_error(agent_id, error)` — 记录 Agent 错误审计事件（异步，不阻塞）

### shutdown 子模块

#### 构造

- **`ShutdownHandle::new()`** — 创建新的 shutdown handle 及内部的 ShutdownCoordinator

#### 主操作

- **`ShutdownHandle::initiate_shutdown()`** — 发起优雅关闭：原子切换到 ShuttingDown，等待所有 in-flight 操作完成（最多 30 秒），超时强制切换到 Draining → Stopped
- **`ShutdownHandle::subscribe_drain()`** — 返回 broadcast receiver，组件通过它感知关闭信号

#### 查询

- **`ShutdownHandle::state()`** — 返回当前 ShutdownState
- **`ShutdownHandle::is_shutting_down()`** — 返回是否已发起关闭
- **`ShutdownHandle::busy_count()`** — 返回当前 in-flight 操作计数（用于监控/调试）

#### 状态原语

- **`ShutdownHandle::increment_busy()`** — 将 busy 计数 +1（组件在开始异步工作前调用）
- **`ShutdownHandle::decrement_busy()`** — 将 busy 计数 -1（组件在异步工作完成后调用）
- **`ShutdownHandle::is_stopped()`** — 返回是否已达到 Stopped 状态

## 架构 / 结构

```
daemon
├── Daemon（主结构）
│   ├── gateway: Arc<Gateway>
│   ├── agent_registry: Arc<RwLock<AgentRegistry>>
│   ├── permission_engine: Arc<PermissionEngine>
│   ├── shutdown: ShutdownHandle
│   ├── chat_server: Arc<ChatServer>
│   ├── audit_logger: Arc<AuditLogger>
│   ├── storage: Arc<SqliteStorage>
│   └── sweeper_shutdown_tx: watch::Sender<()>（控制 ArchiveSweeper 生命周期）
│
└── shutdown（子模块）
    ├── ShutdownState（枚举，状态机状态）
    │   ├── Running
    │   ├── ShuttingDown
    │   ├── Draining
    │   └── Stopped
    └── ShutdownHandle（共享句柄，Clone）
        ├── state: ShutdownState（通过内部 ShutdownCoordinator 原子管理）
        ├── drain_done_tx: broadcast::Sender<()>（关闭广播）
        └── busy_count: Arc<AtomicUsize>（在途操作计数）
```

### 状态转换

```
Running ──(initiate_shutdown)──> ShuttingDown ──(busy_count==0 或超时)──> Draining ──> Stopped
```

### 关键设计模式

- **状态机无锁化**：`ShutdownCoordinator` 使用 AtomicU8 存储状态，组件轮询状态无需加锁
- **busy count 追踪**：组件在异步工作前后分别 increment/decrement busy count，shutdown 时等待计数归零
- **broadcast 通知**：所有组件通过 `subscribe_drain()` 获取关闭信号，同时收到通知
- **审计日志后台刷新**：独立的 tokio task 每 5 秒刷新审计缓冲区，Daemon run() 结束时调用 `shutdown_audit()` 确保落盘

## 测试

### 单元测试

`src/daemon/shutdown.rs` 内置 `#[cfg(test)]` 模块，覆盖：
- 状态机转换（Running → ShuttingDown → Draining → Stopped）
- busy count 递增/递减/归零触发转换
- 多订阅者 drain signal broadcast 不丢信号
- `drain_timeout_secs` / `drain_poll_interval` 在 test 模式下的返回值

### E2E 测试

`tests/e2e_daemon_shutdown_tests.rs`：

| 测试 | 描述 |
|------|------|
| `test_drain_waits_until_busy_count_zero` | increment_busy × 3 → initiate_shutdown → 逐步 decrement → 验证 Stopped |
| `test_drain_timeout_forces_exit` | initiate_shutdown 后不 decrement → 等待 3s → 验证强制 Stopped |
| `test_drain_signal_broadcast` | 两个 subscribe_drain 订阅者 → initiate_shutdown → 两者 1s 内均收到信号 |
| `test_daemon_run_sigterm_shutdown` | Daemon::start + Daemon::run + initiate_shutdown → 验证 5s 内 is_stopped |

### 超时配置差异

| 配置项 | test 模式 | prod 模式 |
|--------|-----------|-----------|
| `drain_timeout_secs()` | 3 秒 | 30 秒 |
| `drain_poll_interval()` | 100 毫秒 | 2 秒 |
