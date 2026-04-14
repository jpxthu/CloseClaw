# Config Module Specification

> 本文档按 SPEC_CONVENTION.md v3 标准编写，描述 `src/config/` 模块的精确功能说明。

---

## 模块概述

配置热加载系统，管理 JSON 配置文件的读取、验证、持久化和热重载。

包含四个子模块：
- **agents**：agents.json 和 per-agent 配置目录的 ConfigProvider 实现
- **backup**：写前备份 + 滚动清理
- **reload**：基于 notify 的文件监控 + 自动重载
- **providers**：ConfigProvider trait 和 ConfigError

---

## 公开类型（`mod.rs` 导出）

| 类型 | 说明 |
|------|------|
| `ConfigProvider` | Trait，所有配置提供者的接口 |
| `ConfigError` | 错误枚举（SchemaError / ValueError / IoError / JsonError） |
| `AgentsConfig` | agents.json 完整配置（version + agents 列表） |
| `AgentsConfigProvider` | agents.json 的 ConfigProvider 实现 |
| `AgentConfig` | agents.json 中单个 agent 的条目数据结构（name/model/parent/persona/max_iterations/timeout_minutes） |
| `AgentDirectoryEntry` | 从 `~/.closeclaw/agents/<id>/` 加载的 agent 条目 |
| `AgentDirectoryProvider` | 扫描 `~/.closeclaw/agents/` 的 ConfigProvider 实现 |
| `BackupManager` | 配置文件滚动备份管理器 |
| `SafeBackupManager` | BackupManager 的线程安全封装（内部 Mutex） |
| `ConfigReloadManager<P>` | 泛型配置热重载管理器 |
| `ConfigReloadEvent` | 热重载事件通知（Reloaded/Rollback/ValidationFailed） |
| `ReloadResult` | 重载操作结果（Success/ValidationFailed/RolledBack） |
| `WatcherHandle` | 文件监控句柄，drop 时停止监控 |

> 注：以下完整实现存在但 mod.rs 未导出——如需通过 crate 外部访问，需在 mod.rs 中补充 re-export：BackupManager、SafeBackupManager、ConfigReloadManager<P>、ConfigReloadEvent、ReloadResult、WatcherHandle

---

## 核心 Trait：`ConfigProvider`

```rust
pub trait ConfigProvider {
    fn version(&self) -> &'static str;
    fn validate(&self) -> Result<(), ConfigError>;
    fn config_path() -> &'static str;
    fn is_default(&self) -> bool;
}
```

---

## `ConfigError`

```rust
pub enum ConfigError {
    SchemaError(String),                    // Schema 校验失败
    ValueError { field: String, message: String },  // 字段值无效
    IoError(std::io::Error),                // IO 错误
    JsonError(serde_json::Error),           // JSON 解析错误
}
```

---

## 子模块结构

### agents：配置加载

**`AgentsConfigProvider`** — 从 JSON 文件加载 agents.json 并验证（内部使用 `AgentConfig` 作为 agent 条目数据结构）。

**目录结构**：
```
~/.closeclaw/agents/<agent-id>/
├── config.json         # 必填，AgentDirConfig
└── permissions.json    # 可选，AgentPermissions
```

**`AgentDirectoryProvider`** — 扫描 `~/.closeclaw/agents/` 目录，加载每个 agent 的 config.json 和可选的 permissions.json。非 UTF-8 目录名和缺少 config.json 的目录会被跳过。

验证规则：agent id 非空、id 全局唯一。

**查询接口**（AgentDirectoryProvider）：
- `get(id: &str) -> Option<&AgentDirectoryEntry>` — 按 id 查找
- `agent_ids() -> Vec<&String>` — 所有 agent id
- `entries() -> &HashMap<String, AgentDirectoryEntry>` — 完整映射
- `AgentDirectoryProvider::new()` — 扫描目录并加载
- `save_agent()` — 写入 config.json 和 permissions.json
- `remove_agent()` — 删除目录和内存条目
- `reload()` — 重新扫描目录

**`AgentsConfigProvider`** 构造与查询接口：
- `new(path: &str)` — 从文件路径构造
- `from_json_str(content: &str)` — 从 JSON 字符串构造（测试用）
- `get(name: &str) -> Option<&AgentConfig>` — 按名字查找 agent
- `agents() -> &[AgentConfig]` — 列出所有 agent
- `lookup() -> HashMap<&str, &AgentConfig>` — 批量查找映射
- `inner() / inner_mut()` — 获取原始配置
- `reload()` — 重新从磁盘加载

---

### backup：写前备份

**`BackupManager`** — 维护每个配置文件的滚动备份历史（最近 N 份）。构造：`new(backup_dir, max_backups)`。接口：`backup` / `backup_with_content` / `rollback` / `list_backups` / `find_latest_backup`。

**`SafeBackupManager`** — BackupManager 的线程安全封装（内部 Mutex）。构造：`new(manager)`。接口：同名代理方法（`backup` / `backup_with_content` / `rollback` / `list_backups`）。

轮转规则：超过 `max_backups` 时删除最旧的备份文件。

---

### reload：热重载

**`ConfigReloadManager<P>`** — 监控配置文件变更，验证通过后自动重载。

手动重载流程：读取文件 → 备份 → 解析 → 验证 → 通过则替换内存配置，失败则返回 ValidationFailed。

Watch 后台线程流程：debounce 防抖 → 读取新内容 → 备份 → 解析验证 → 通过则替换，失败则发送 ValidationFailed 事件。

**构造**：
- `new(...)` — 无事件通道
- `with_events(...)` — 带 mpsc::Channel<ConfigReloadEvent> 通道

**查询/操作**：
- `provider() -> Arc<std::sync::Mutex<P>>` — 获取 provider 句柄
- `snapshot() -> P` — 克隆当前配置快照（需要 P: Clone）
- `reload(path: &str) -> ReloadResult` — 手动触发重载
- `watch(paths: Vec<PathBuf>) -> Result<WatcherHandle, ConfigError>` — 启动文件监控

**事件类型**（通过 mpsc 发送）：
```rust
pub enum ConfigReloadEvent {
    Reloaded { path: String },
    Rollback { path: String, error: String },
    ValidationFailed { path: String, error: String },
}

pub enum ReloadResult {
    Success,
    ValidationFailed(ConfigError),
    RolledBack(ConfigError),
}
```

**`WatcherHandle`** — watch 返回的句柄，drop 时停止监控。
