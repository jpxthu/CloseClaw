# daemon 模块规格说明书

## 模块概述

daemon 是 CloseClaw 的后台服务进程，负责启动和协调所有组件（Gateway、AgentRegistry、PermissionEngine、ChatServer、AuditLogger），并在接收到 SIGINT/SIGTERM 信号时执行优雅关闭。

启动流程按顺序完成：加载 .env → 初始化 SqliteStorage（失败则 start() 返回 Err）→ 初始化 JsonSessionConfigProvider（文件不存在则 warn + 硬编码默认值）→ 创建 ArchiveSweeper 并 spawn 后台任务 → 初始化 PermissionEngine → 初始化 AgentRegistry → 初始化 Gateway（注入 storage）→ 注册 Feishu 适配器 → 启动 ShutdownCoordinator → 注册 LLM Providers → 启动 Chat TCP Server → 启动 AuditLogger。

SqliteStorage 的 data_dir 直接使用 `config_dir` 参数，不做 fallback 计算。

关闭流程采用状态机管理：Running → ShuttingDown → Draining → Stopped，等待所有在途操作完成后退出，超时强制终止；Daemon shutdown 时通过 `sweeper_shutdown_tx` 通知 ArchiveSweeper 优雅退出。

## 公开接口

### 构造

- **`Daemon::start(config_dir)`** — 异步构造器，按启动流程初始化所有组件，返回完整的 Daemon 实例
- **`Daemon::start_with_audit_logger(config_dir, audit_logger)`** — 同 `start()` 但使用外部注入的 `AuditLogger`（用于测试）
- **`Daemon::start_with_audit_logger_and_engine(config_dir, audit_logger, permission_engine)`** — 同 `start()` 但同时注入 `AuditLogger` 和 `PermissionEngine`（用于测试）

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

`src/daemon/mod.rs` 内置 `#[cfg(test)]` 模块，覆盖：

**`load_env_file` 测试**

| 测试 | 描述 |
|------|------|
| `test_load_env_file_normal_parsing` | 多行 key=value 正常解析 |
| `test_load_env_file_comment_lines` | `#` 开头的注释行被跳过 |
| `test_load_env_file_empty_lines` | 空行被跳过 |
| `test_load_env_file_empty_value` | 空 value（`KEY=`）被忽略 |
| `test_load_env_file_empty_key` | 空 key（`=value`）被忽略 |
| `test_load_env_file_no_equal_sign` | 不含 `=` 的行被跳过 |
| `test_load_env_file_file_not_found` | 文件不存在返回 Err |
| `test_load_env_file_whitespace_trimming` | key/value 前后空格被正确 trim |

**`Daemon::load_agents_config` 测试**

| 测试 | 描述 |
|------|------|
| `test_load_agents_config_success` | 合法 agents.json 成功加载并解析 |
| `test_load_agents_config_file_not_found` | 文件不存在返回 Err |
| `test_load_agents_config_invalid_json` | JSON 非法返回 Err |

**`Daemon::build_permission_engine` 测试**

| 测试 | 描述 |
|------|------|
| `test_build_permission_engine_empty_dir` | 空 config 目录正常创建 engine |
| `test_build_permission_engine_with_templates_dir` | 存在 templates 目录时正常加载 |

**`Daemon` 实例方法集成测试**（需启动 Daemon）

| 测试 | 描述 |
|------|------|
| `test_daemon_start_with_valid_env` | 含 .env 文件时 Daemon 正常启动 |
| `test_audit_lifecycle` | 验证从 `Daemon::start()` 到 `shutdown_audit()` 全程审计事件（ConfigReload、PermissionCheck、AgentStart、AgentStop、AgentError）的生成与落盘完整性，测试使用临时目录注入自定义 `AuditLogger` |
| `test_evaluate_with_audit_returns_response` | evaluate_with_audit 不 panic 并返回 Allowed/Denied |
| `test_log_agent_start_does_not_panic` | log_agent_start 不 panic，audit_logger 存活 |
| `test_log_agent_stop_does_not_panic` | log_agent_stop 不 panic |
| `test_log_agent_error_does_not_panic` | log_agent_error 不 panic |
| `test_shutdown_audit_closes_logger` | shutdown_audit 不 panic，logger 正常关闭 |

### E2E 测试

`src/daemon/shutdown.rs` 内置 `#[cfg(test)]` 模块，覆盖：
- 状态机转换（Running → ShuttingDown → Draining → Stopped）
- busy count 递增/递减/归零触发转换
- 多订阅者 drain signal broadcast 不丢信号
- `drain_timeout_secs` / `drain_poll_interval` 在 test 模式下的返回值

`tests/e2e_daemon_shutdown_tests.rs`：

| 测试 | 描述 |
|------|------|
| `test_drain_waits_until_busy_count_zero` | increment_busy × 3 → initiate_shutdown → 逐步 decrement → 验证 Stopped |
| `test_drain_timeout_forces_exit` | initiate_shutdown 后不 decrement → 等待 3s → 验证强制 Stopped |
| `test_drain_signal_broadcast` | 两个 subscribe_drain 订阅者 → initiate_shutdown → 两者 1s 内均收到信号 |
| `test_daemon_run_sigterm_shutdown` | Daemon::start + Daemon::run + initiate_shutdown → 验证 5s 内 is_stopped |

`tests/e2e_daemon_audit_tests.rs`：

| 测试 | 描述 |
|------|------|
| `test_audit_config_reload_on_start` | Daemon 启动后审计文件包含 ConfigReload 事件（details 含 component 和 version） |
| `test_audit_permission_check_allow_deny` | evaluate_with_audit Allowed 和 Denied 各一次，审计文件包含对应事件及正确 result |
| `test_audit_agent_lifecycle_events` | log_agent_start/stop/error 调用后审计文件包含正确事件类型和 result |
| `test_audit_buffer_flushed_on_shutdown` | shutdown_audit() 后 buffer 为空，所有事件均落盘 |
| `test_audit_no_external_dependencies` | 测试仅使用 temp dir + fake mock，无外网依赖 |

### 超时配置差异

| 配置项 | test 模式 | prod 模式 |
|--------|-----------|-----------|
| `drain_timeout_secs()` | 3 秒 | 30 秒 |
| `drain_poll_interval()` | 100 毫秒 | 2 秒 |
