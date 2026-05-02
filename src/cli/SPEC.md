# CLI 模块规格说明书

## 模块概述

`closeclaw chat` 命令行工具。通过 TCP 协议连接 chat server（默认 `127.0.0.1:18889`），提供两种工作模式：

- **单次模式**（`--message`）：发送一条消息、打印响应、立即退出
- **REPL 模式**：交互式会话，从 stdin 读取输入，持续对话直到用户输入 `quit`/`exit`

协议基于 newline-delimited JSON，与 server 交换 `chat.start` / `chat.message` / `chat.response` / `chat.stop` 消息。agent_id 优先级：CLI 参数 > `CLOSEWCLAW_DEFAULT_AGENT` 环境变量 > 默认值 `"guide"`。

---

## 公开接口

### ChatCommand

`closeclaw chat` 子命令的 clap 命令结构体。

**配置字段：**
- `--addr`：TCP 地址（默认 `127.0.0.1:18889`）
- `--agent-id`：指定会话使用的 agent（默认 `"guide"`）
- `--message`：单次模式，传入要发送的消息

**主操作：**
- `run()` — 异步入口，根据是否传入 `--message` 路由到单次模式或 REPL 模式

---

## 架构与结构

### 子模块

- `chat` — `src/cli/chat.rs`，包含全部 CLI 实现（命令解析、会话管理、REPL、数据流）
- `args` — `src/cli/args.rs`，reserved（未来参数扩展位）

### 数据流

两种模式共享相同的协议序列：

```
client → chat.start         (带 agent_id)
server → chat.started       (带 session_id)
client → chat.message       (带 content)
server → chat.response     (流式内容片段)
server → chat.response.done
client → chat.stop
```

- **超时兜底**：连接和读取阶段各有一层 30s timeout safety net，daemon 无响应时 CLI 明确报错而非悬停。连接超时返回 `connect timeout after {n}s`，读取超时返回 `read timeout after {n}s`。

### 关键设计

- **协议**：JSON-RPC 风格，TCP newline-delimited（每条消息以 `\n` 分隔）
- **并发模型**：REPL 模式使用 `tokio::select!` 同时监听 stdin 和 server 消息
- **错误处理**：server 返回 `chat.error` 时 abort 并打印错误信息
