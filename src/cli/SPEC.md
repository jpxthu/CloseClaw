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
- `--addr`：TCP 地址（优先级：CLI参数 > `CHAT_SERVER_ADDR` 环境变量 > 默认值 `127.0.0.1:18889`）
- `--agent-id`：指定会话使用的 agent（默认 `"guide"`）
- `--message`：单次模式，传入要发送的消息

**主操作：**
- `run()` — 异步入口，根据是否传入 `--message` 路由到单次模式或 REPL 模式

### ConfigCommand

`closeclaw config setup` 子命令，交互式配置向导。

- `handle_config_setup()` — 启动 `config_wizard::run_wizard().await`，用户依次经历 SelectProvider → InputCredential → FetchModels → SelectModels → Confirm → WriteConfig，最终将配置写入 `~/.closeclaw/config/models.json` 和 `~/.closeclaw/config/credentials/<provider_id>.json`。InputCredential 使用 `Password`（不回显）；写入时采用合并策略：当前 provider 的模型整体替换，其他 provider 的已配置模型保留。

---

## 架构与结构

### 子模块

- `chat` — `src/cli/chat.rs`，包含全部 CLI 实现（命令解析、会话管理、REPL、数据流）
- `args` — `src/cli/args.rs`，reserved（未来参数扩展位）
- `config_wizard` — `src/cli/config_wizard/`，交互式配置向导，替代 `handle_config_setup` 的简单循环逻辑

### ConfigWizard 模块

`config_wizard` 提供 `closeclaw config setup` 的交互式配置流程。用户依次经历线性状态机：SelectProvider → InputCredential → FetchModels → SelectModels → Confirm → WriteConfig，最终将配置写入 `~/.closeclaw/config/models.json` 和 `~/.closeclaw/config/credentials/<provider_id>.json`。

FetchModels 阶段调用 `fetch_models_with_retry()` 获取模型列表，`fetch_models_with_retry()` 包装 `fetch_model_list()` 调用，最多重试 3 次。重试策略：
- **Transient 错误**（429 / 5xx / 网络超时）→ 指数退避重试（1s~10s 上界），耗尽 3 次后回退到 `ProviderModelKnowledge` 知识库
- **超时**（10s）→ 视为 Transient，一同参与指数退避重试
- **Auth / Billing / InvalidRequest** → 立即回退到知识库，不重试
- 重试延迟使用 `crate::llm::retry::backoff_delay()` 计算（1s base，10s 上界）

SelectModels 支持空格分隔编号，范围语法（`1-3,5,7`）和 `all` 关键字，每行显示列表时追加 `protocol: {proto} (recommended)` 标签。写入时采用合并策略：当前 provider 的模型整体替换，其他 provider 的已配置模型保留。`write_wizard_config()` 写入 `ProviderConfig.protocol` 字段，值为第一个选中模型的 `recommended_protocol`。

**公开接口：**

- `run_wizard()` — 异步入口函数（`async fn`），返回 `Ok(Some(WizardOutput))` 或 `Ok(None)`（Ctrl+C 干净退出）。dialoguer 的同步阻塞调用（Select/Password/Input/Confirm 的 `interact()`）通过 `tokio::task::spawn_blocking` 包装，确保在已有 tokio runtime 上下文中安全执行
- `parse_model_selection(input, total)` — 解析用户模型选择输入，返回 0-based 索引向量
- `write_wizard_config(output)` — 将 WizardOutput 写入 models.json 和凭据文件

**核心数据结构：**

- `WizardState` — 6 变体状态枚举，对应状态机各阶段
- `WizardContext` — 携带 current_state、selected_provider、credential、fetched_models、selected_models、existing_config、provider
- `WizardOutput` — Wizard 的最终输出，含 provider_id、credential、selected_models
- `ProviderInfo` — Provider 元数据（id、display_name、ProviderType），PROVIDERS 常量列出全部 4 个 Provider
- `ProviderType` — 枚举变体：Minimax / Glm / Volcengine / Deepseek
- `ProviderConfig`（见 `config/providers/models.rs`）— 含 `protocol: Option<String>` 字段，写入 models.json 时由第一个选中模型的 recommended_protocol 填充

**模块结构：**

- `types.rs` — WizardState、ProviderType、ProviderInfo、PROVIDERS 常量、WizardContext、WizardOutput
- `mod.rs` — run_wizard()、parse_model_selection()、write_wizard_config()、fetch_models_with_fallback()、知识库回退、UT

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
