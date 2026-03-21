# 权限引擎 — 概述

## 什么是权限引擎？

权限引擎是 CloseClaw 多 Agent 框架中的**安全组件**，用于对 Agent 的访问控制规则进行强制执行。在任何一个 Agent 执行特权操作之前——无论是读取文件、执行命令、发起网络请求，还是发送跨 Agent 消息——该操作都必须经过引擎的评估并被明确允许。

引擎被设计为**独立操作系统进程**，以实现纵深防御：即使 Agent 运行时被攻破，引擎的严格规则评估也无法被绕过。

## 架构

```
┌─────────────────────────────────────────────────────────────────┐
│                        CloseClaw Host Process                    │
│                                                                  │
│  ┌──────────────┐      IPC (Unix socket)      ┌──────────────┐  │
│  │ Agent Runtime │ ◄─────────────────────────► │  Permission  │  │
│  │              │    SandboxRequest/Response    │   Engine     │  │
│  │ - dev-agent  │                              │  (subprocess)│  │
│  │ - prod-agent │                              │              │  │
│  └──────────────┘                              └──────────────┘  │
│                                                              ▲   │
└──────────────────────────────────────────────────────────────┼───┘
                                                               │
                                                  Policy applied:
                                                  - seccomp (Linux)
                                                  - landlock (Linux)
                                                  - deny-by-default
```

### 关键设计决策

1. **独立进程** — 引擎作为子进程运行，与 Agent 运行时的内存空间隔离，可以独立地被杀死或重启。
2. **默认拒绝** — 任何未被规则明确允许的操作都会被**拒绝**。不存在隐式允许。
3. **无状态评估** — 每次 `evaluate()` 调用都是无状态的。引擎在启动时读取规则集并重建内存索引。规则可以通过 IPC 通道热重载。
4. **AWS IAM 风格优先级** — 当多条规则匹配时，**拒绝优先**。如果任何匹配的规则说"拒绝"，该操作就被拒绝，无论其他允许规则如何。
5. **O(1) Agent 查找** — 规则索引预构建为以 Agent ID 为键的 HashMap，精确匹配为常数时间。Glob 模式回退到线性扫描。

## 核心组件

### `PermissionEngine`（`engine.rs`）

核心规则评估逻辑。实现 AWS IAM 风格策略评估：
- `RuleSet` — 完整解析后的规则文档
- `Rule` — 具有主体、效果和操作的命名规则
- `PermissionRequest` / `PermissionResponse` — 评估输入/输出

### `Sandbox`（`sandbox/mod.rs`）

引擎子进程的生命周期管理：
- `spawn()` — 将引擎作为子进程启动
- `restart()` — 杀死并重新启动引擎
- `shutdown()` — 干净地终止引擎
- `evaluate()` — 通过 IPC 发送 `PermissionRequest` 并返回 `PermissionResponse`
- `reload_rules()` — 在运行的引擎中热重载规则集

### `IpcChannel`

带长度前缀的 JSON 帧的 Unix 域套接字：
```
[4字节大端 u32 长度][JSON payload]
```

### `SecurityPolicy`

Linux 特定的沙箱加固：
- **seccomp** — 限制可用的系统调用
- **landlock** — 限制文件系统访问路径

## 生命周期

```
Host 启动
    │
    ▼
Sandbox::spawn()
    │
    ├─► 启动引擎子进程（SANDBOX_ENGINE=1）
    │
    ├─► 等待 socket 出现
    │
    ├─► 发送 Ping，等待 Pong
    │
    ▼
引擎运行中 ◄──────────────────────┐
    │                              │
    ├── evaluate(request) ──► Response
    ├── reload_rules(ruleset) ──► ACK
    │                              │
    │  （检测到崩溃）                │
    ▼                              │
Sandbox::restart() ─────────────────┘
```

## 序列化

`PermissionRequest` 和 `PermissionResponse` 使用 Serde 进行序列化，可以通过 IPC 通道作为 JSON 发送。

## 安全注意事项

- 引擎子进程以与主机进程**相同的用户权限**运行。如需更强隔离，考虑在具有 seccomp 配置和无能力的容器（Docker/podman）中运行引擎。
- Landlock 支持取决于内核版本（≥ 5.13）。在旧内核上，landlock 策略会被静默跳过。
- 此实现中的 seccomp 是**演示用途** — 生产使用请替换为适当的 libseccomp BPF 程序。
