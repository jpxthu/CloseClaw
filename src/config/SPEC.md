# Config Module Specification

> 本文档描述 `src/config/` 模块的精确功能说明。

---

## 模块职责

配置热加载系统：
- 管理 agents.json 的读取、验证、解析
- 管理 `~/.closeclaw/agents/<id>/` 目录下 per-agent 配置的加载和持久化
- 提供配置变更监控（notify）和自动重载
- 提供写前备份和回滚能力

---

## 公开类型（`mod.rs` 导出）

| 类型 | 说明 |
|------|------|
| `ConfigProvider` | Trait，所有配置提供者的接口 |
| `ConfigError` | 错误枚举：SchemaError / ValueError / IoError / JsonError |
| `AgentsConfig` | agents.json 完整配置（含 version + agents 列表） |
| `AgentsConfigProvider` | agents.json 的 ConfigProvider 实现 |
| `AgentConfig` | 单个 agent 的配置（name / model / parent / persona / max_iterations / timeout_minutes） |
| `AgentDirectoryEntry` | 从 `~/.closeclaw/agents/<id>/` 加载的 agent 条目（config.json + 可选 permissions.json） |
| `AgentDirectoryProvider` | 扫描 `~/.closeclaw/agents/` 的 ConfigProvider 实现 |

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

## `AgentsConfigProvider`

文件：`src/config/agents.rs`

**职责**：从 JSON 文件加载 agents.json 并验证。

**构造方法**：
- `new(path: P) -> Result<Self, ConfigError>` — 从文件路径加载
- `from_json_str(content: &str) -> Result<Self, ConfigError>` — 从字符串加载（测试用）

**查询方法**：
- `get(name: &str) -> Option<&AgentConfig>` — 按名字查找 agent
- `agents() -> &[AgentConfig]` — 列出所有 agent
- `lookup() -> HashMap<&str, &AgentConfig>` — 批量查找映射
- `inner() -> &AgentsConfig` — 获取原始配置
- `inner_mut() -> &mut AgentsConfig` — 可变借用

## `AgentsConfig`

文件：`src/config/agents.rs`

**职责**：agents.json 配置的数据结构，包含 version 和 agent 列表。

**字段**：
- `version: String`
- `agents: Vec<AgentConfig>`

**方法**：
- `validate(&self) -> Result<(), ConfigError>` — 验证 version 非空、agent 字段有效、parent 引用存在

---

## `AgentDirectoryProvider`

**验证规则**：
- version 字段不得为空
- agent name 不得为空
- agent model 不得为空
- agent name 唯一（重复报错）
- parent 引用必须存在（指向已定义的 agent）

---

## `AgentDirectoryProvider`

文件：`src/config/agents.rs`

**职责**：从 `~/.closeclaw/agents/<id>/` 目录扫描并加载 per-agent 配置。

**目录结构**：
```
~/.closeclaw/agents/<agent-id>/
├── config.json         # 必填，AgentDirConfig
└── permissions.json    # 可选，AgentPermissions
```

**构造方法**：
- `new(agents_dir: PathBuf) -> Result<Self, ConfigError>` — 扫描目录并加载

**查询方法**：
- `get(id: &str) -> Option<&AgentDirectoryEntry>` — 按 id 查找
- `agent_ids() -> Vec<&String>` — 所有 agent id
- `entries() -> &HashMap<String, AgentDirectoryEntry>` — 完整映射

**持久化方法**：
- `save_agent(entry: &AgentDirectoryEntry) -> Result<(), ConfigError>` — 写入 config.json 和 permissions.json
- `remove_agent(id: &str) -> Result<(), ConfigError>` — 删除目录和内存条目

**重载**：
- `reload(&mut self) -> Result<(), ConfigError>` — 重新扫描目录

**特殊行为**：
- 非 UTF-8 目录名会被跳过（不会导致加载失败）
- 缺少 config.json 的目录会被跳过

---

## `BackupManager`

文件：`src/config/backup.rs`

**职责**：写前备份 + 滚动清理。

**构造**：
- `new(backup_dir: P, max_backups: usize) -> io::Result<Self>`

**方法**：
- `backup(file_path: P) -> io::Result<PathBuf>` — 读取文件内容 → 写 backup → 轮转
- `backup_with_content(file_path: P, content: &[u8]) -> io::Result<PathBuf>` — 显式提供内容备份
- `rollback(file_path: P) -> io::Result<PathBuf>` — 回滚到最近一次备份
- `list_backups(file_path: P) -> io::Result<Vec<PathBuf>>` — 列出某文件的备份（ newest first）
- `find_latest_backup(file_path: P) -> io::Result<PathBuf>` — 最近备份路径

**轮转规则**：
- 超过 `max_backups` 时删除最旧的备份文件

### `SafeBackupManager`

文件：`src/config/backup.rs`

`BackupManager` 的线程安全封装，内部持有一个 `Mutex<BackupManager>`。

**构造**：
- `new(manager: BackupManager) -> Self` — 从 `BackupManager` 实例创建线程安全封装

**方法**（全部线程安全）：
- `backup(file_path: P) -> io::Result<PathBuf>` — 线程安全备份
- `backup_with_content(file_path: P, content: &[u8]) -> io::Result<PathBuf>` — 线程安全显式内容备份
- `rollback(file_path: P) -> io::Result<PathBuf>` — 线程安全回滚
- `list_backups(file_path: P) -> io::Result<Vec<PathBuf>>` — 线程安全列出备份

> 注：`find_latest_backup` 仅在 `BackupManager` 上有，`SafeBackupManager` 未透传。

---

## `ConfigReloadManager<P>`

文件：`src/config/reload.rs`

**职责**：监控配置文件变更，自动重载。

**构造**：
- `new(provider: P, backup_manager: SafeBackupManager, debounce_duration: Duration, parse_fn: impl Fn(&str) -> Result<P, ConfigError>)` — 无事件通道
- `with_events(...)` — 带 `mpsc::Channel<ConfigReloadEvent>` 通道

**方法**：
- `provider() -> Arc<std::sync::Mutex<P>>` — 获取 provider 句柄
- `snapshot() -> P` — 克隆当前配置快照（需要 P: Clone）
- `reload(path: &str) -> ReloadResult` — 手动触发重载
- `watch(paths: Vec<PathBuf>) -> Result<WatcherHandle, ConfigError>` — 启动文件监控

**手动 reload 流程**：
1. 读取当前文件内容做备份
2. 尝试解析新内容
3. 验证新配置
4. 验证通过 → 替换内存中的 provider
5. 验证失败 → 返回 ValidationFailed，不更新

**watch 后台线程流程**：
1. debounce 防抖（默认跳过）
2. 读取新内容 + 备份
3. 解析 + 验证
4. 通过 → 替换；失败 → 发送 ValidationFailed 事件

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

---

## 偏差追踪

### `mod.rs` 导出不完整

| 缺失导出 | 说明 |
|---------|------|
| `ConfigReloadManager` | 完全未导出 |
| `ConfigReloadEvent` | 完全未导出 |
| `ReloadResult` | 完全未导出 |
| `WatcherHandle` | 完全未导出 |
| `BackupManager` | 完全未导出 |
| `SafeBackupManager` | 完全未导出 |

以上类型均为完整的公开实现，但 `mod.rs` 未 re-export。

### docs/config/README.md 与代码的偏差

| 偏差 | 类型 | 说明 |
|------|------|------|
| `ConfigReloadManager` 未导出 | 少了 | 文档提及但 mod.rs 无导出 |
| `BackupManager` / `SafeBackupManager` 未导出 | 少了 | 同上 |
| `permissions.json` 描述位置错误 | 冲突 | 文档写"permission rules 配置文件"在根目录；实际 `AgentDirectoryProvider` 从 `~/.closeclaw/agents/<id>/permissions.json` 加载 |
| `skills.json` 的 ConfigProvider 不存在 | 少了 | 文档列出 skills.json 为配置项，但代码无 `SkillConfigProvider` |
| permissions.json 描述为"非热加载" | 说明 | 与代码行为一致（AgentDirectoryProvider 用文件路径，非 watch），但文档描述的是根目录 permissions.json，与实际不符 |

### 其他说明

- `reload.rs` 是模块内完整实现，不是 stub
- `backup.rs` 的轮转策略（删除最旧）与文档"保留 10 份"一致（文档描述的是默认行为，代码通过参数控制）
