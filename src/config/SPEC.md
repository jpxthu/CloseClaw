# Config Module Specification

> 本文档按 SPEC_CONVENTION.md v3 标准编写，描述 `src/config/` 模块的精确功能说明。

---

## 模块概述

配置热加载系统，管理 JSON 配置文件的读取、验证、持久化和热重载。

包含七个子模块：
- **agents**：agents.json 和 per-agent 配置目录的 ConfigProvider 实现
- **backup**：写前备份 + 滚动清理
- **manager**：ConfigManager 统一配置管理入口，提供原子写入、备份集成和配置访问
- **migration**：openclaw.json → config/ 目录的引导迁移
- **providers**：ConfigProvider trait 和 ConfigError
- **reload**：基于 notify 的文件监控 + 自动重载
- **session**：per-agent per-role session 配置（idle/purge），供 ArchiveSweeper 和 Daemon 使用

---

## 公开类型（`mod.rs` 导出）

| 类型 | 说明 |
|------|------|
| `ConfigProvider` | Trait，所有配置提供者的接口 |
| `ConfigError` | 错误枚举（SchemaError / ValueError / IoError / JsonError） |
| `ConfigManager` | 统一配置管理入口，提供原子写入、备份集成和配置访问 |
| `ConfigSection` | 配置节枚举（Models / Channels / Gateway / Plugins / System / Credentials） |
| `ConfigInfo` | 配置文件元数据（path / version / last_modified） |
| `ConfigLoadError` | 配置加载错误（ConfigDirNotFound / ConfigFileNotFound / ParseError / ValidationError / BackupNotFound / IoError） |
| `ConfigWriteError` | 配置写入错误（ValidationFailed / BackupFailed / WriteFailed / FileNotFound） |
| `ConfigValidationError` | 配置验证错误（path + message） |
| `write_atomically` | 原子写入函数（写临时文件 + fsync + rename） |
| `PerAgentSessionConfig` | 单个 agent-role 的 session 配置（idle_minutes / purge_after_minutes） |
| `SessionConfig` | 完整 session 配置容器（defaults / agents / sweeper_interval_secs） |
| `SessionConfigProvider` | Trait，获取 per-agent session 配置和 sweeper 间隔 |
| `JsonSessionConfigProvider` | JSON 文件实现的 SessionConfigProvider |
| `AgentRole` | Agent 角色枚举（MainAgent / SubAgent），imported from `crate::session::persistence` |
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
| `migrate_if_needed` | 检测并执行 openclaw.json → config/ 目录的引导迁移 |
| `ConfigMigrationError` | 迁移错误枚举（ReadError / ParseError / WriteError / MalformedJson / NotFound） |

> 注：BackupManager、ConfigReloadManager<P>、ConfigReloadEvent、ReloadResult、WatcherHandle 存在但 mod.rs 未导出。SafeBackupManager 已 re-exported。

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

### session：per-agent session 配置

**用途**：解析 `session_config.json`，提供 per-agent per-role 的 idle/purge 配置，为 ArchiveSweeper 和 Daemon 集成提供基础。

**常量**：
- `DEFAULT_IDLE_MINUTES`（30）：默认 idle 超时（分钟）
- `DEFAULT_PURGE_AFTER_MINUTES`（10080，7天）：默认 purge 超时（分钟），0 = 永不过期
- `DEFAULT_SWEEPER_INTERVAL_SECS`（300，5分钟）：默认 sweeper 轮询间隔（秒）

**数据结构**：
- `PerAgentSessionConfig`：单个 agent-role 的配置（idle_minutes、purge_after_minutes）
- `SessionConfig`：完整配置容器（defaults: BTreeMap<AgentRole, PerAgentSessionConfig>、agents: BTreeMap<agent_id, BTreeMap<AgentRole, PerAgentSessionConfig>>、sweeper_interval_secs）

**Trait `SessionConfigProvider`**（Send + Sync）：
- `session_config_for(agent_id, role)` — 按 agent/role 查询配置，使用 fallback 链：per-agent override → defaults → hardcoded defaults
- `sweeper_interval_secs()` — sweeper 轮询间隔
- `list_agents()` — 所有配置了 per-agent override 的 agent_id 列表

**`JsonSessionConfigProvider`** 构造与行为：
- `new(path)` — 读取 JSON 文件
  - 文件不存在 → `warn!` + 硬编码默认值（不报错）
  - JSON 解析错误 → `Err(ConfigError::SchemaError)`
  - 负值校验失败 → `Err(ConfigError::ValueError)`
- `validate()` — 校验 idle_minutes >= 0、purge_after_minutes >= 0

**Fallback 链**（`session_config_for` 查询优先级）：
1. per-agent override（agents 字段，agent_id + role 精确匹配）
2. defaults（defaults 字段，role 匹配）
3. hardcoded defaults（常量）

---

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

**`ConfigManager::load()` 的 rollback-on-corruption 行为** — 当 mandatory section（models / channels / gateway / plugins / system）的 JSON 文件解析失败时，ConfigManager::load() 会尝试从备份恢复：

1. 调用 `backup_manager.find_latest_backup(&path)` 查找最新备份
2. 如果存在备份：调用 `backup_manager.rollback(&path)` 恢复文件，重新解析
   - 重试成功 → `tracing::warn!("已用备份恢复 {}", section)` + 继续加载
   - 重试失败 → `tracing::error!("配置文件 {} 恢复后仍无法解析，daemon 无法启动")` → 返回 `ConfigLoadError::ParseError`
3. 如果无备份 → `tracing::error!("配置文件 {} 损坏且无备份，daemon 无法启动")` → 返回 `ConfigLoadError::ParseError`

credentials 文件损坏不影响启动（warn-and-continue），无需 rollback 处理。

---

### providers：配置源实现

**用途**：提供各种配置文件的 ConfigProvider 实现，将配置数据与验证逻辑封装为统一 trait 接口。

**`GatewayConfigData`** — 从 `gateway.json` 加载网关配置，实现 `ConfigProvider` trait。

**数据结构**：version、name、port、timeout、rate_limit_per_minute、max_message_size、dm_scope。

**验证规则**：port > 0（port 为 u16，上限 65535 由类型系统天然保证）。

**构造方法**：
- `from_file(path)` — 从文件路径加载
- `from_json_str(content)` — 从 JSON 字符串加载（测试用）

**常量**：
- `DEFAULT_PORT`（3000）
- `DEFAULT_TIMEOUT`（30000）
- `DEFAULT_RATE_LIMIT_PER_MINUTE`（60）
- `DEFAULT_MAX_MESSAGE_SIZE`（16384）
- `DEFAULT_DM_SCOPE`（"per-channel-peer"）

**`SystemConfigData`** — 从 `openclaw.json` 的 system 区段加载系统配置，实现 `ConfigProvider` trait。涵盖：wizard、update、meta、messages、commands、session、cron、hooks、browser、auth（仅 profiles，不含 apiKey）。

**数据结构**：
- `WizardConfig` — wizard 上一次运行状态（lastRunAt / lastRunVersion / lastRunCommand / lastRunMode），均为 `Option`
- `UpdateConfig` — checkOnStart（默认 true）
- `MetaConfig` — 上次 touch 版本和时间（lastTouchedVersion / lastTouchedAt），均为 `Option`
- `MessagesConfig` — ackReactionScope（`Option`）
- `CommandsConfig` — native / nativeSkills / restart（均默认 true）、ownerDisplay（`Option`）
- `SessionConfig` — dmScope（默认 "per-account-channel-peer"）+ maintenance 子配置
  - `SessionMaintenanceConfig` — mode（默认 "enforce"）、pruneAfter（默认 "7d"）、maxEntries（默认 500）
- `CronConfig` — enabled（默认 true）
- `HooksConfig` — internal 子配置（enabled + entries map）
  - `HooksInternalConfig` — enabled（默认 true）、entries（`BTreeMap<String, HookEntryConfig>`）
  - `HookEntryConfig` — enabled（默认 true）
- `BrowserConfig` — executablePath（`Option`）、headless（默认 true）、defaultProfile（`Option`）
- `AuthProfilesConfig` — profiles（`BTreeMap<String, AuthProfileEntryConfig>`）
  - `AuthProfileEntryConfig` — provider（非空字符串）、mode（默认空字符串）

**验证规则**：
- `session.maintenance.mode` ∈ {enforce, warn, off}
- `session.dmScope` ∈ {per-account-channel-peer, per-channel-peer, per-peer, main}

**构造方法**：
- `from_file(path)` — 从文件路径加载
- `from_json_str(content)` — 从 JSON 字符串加载

**`is_default` 判断**：所有子结构均为默认值时返回 true（与 JSON 中字段缺失/存在无关）。

**`PluginsConfigData`** — 从 `plugins.json` 加载插件配置，实现 `ConfigProvider` trait。涵盖：插件启用状态、白名单、插件条目和安装信息。

**数据结构**：
- `version`（String，默认 `"1.0.0"`）
- `enabled`（bool，默认 `true`）
- `allow`（`Vec<String>`，默认 `[]`）— 允许的插件名称列表
- `entries`（`BTreeMap<String, PluginEntry>`，默认 `{}`）— 插件条目映射
- `installs`（`BTreeMap<String, PluginInstallInfo>`，默认 `{}`）— 插件安装信息映射

**子结构**：`PluginEntry` / `PluginInstallInfo`

`PluginEntry` 字段：
- `enabled`（bool，默认 `false`）

`PluginInstallInfo` 字段：
- `source`（`Option<String>`，默认 `None`）
- `sourcePath`（`Option<String>`，默认 `None`）
- `installPath`（`Option<String>`，默认 `None`）
- `version`（`Option<String>`，默认 `None`）
- `installedAt`（`Option<String>`，默认 `None`）

**验证规则**：
- `allow` 列表中不允许存在空字符串

**构造方法**：
- `from_file(path)` — 从文件路径加载
- `from_json_str(content)` — 从 JSON 字符串加载

**`is_default` 判断**：version = `"1.0.0"` 且 enabled = `true` 且 allow / entries / installs 均为空时返回 true。

**`ModelsConfigData`** — 从 `models.json` 加载模型配置，实现 `ConfigProvider` trait。mode 字段控制合批策略（`merge` / `replace`），providers 以 provider id 为键组织各厂商的 endpoint、认证信息和模型列表。

**子结构**：`ProviderConfig` / `ModelDefinition`

`ProviderConfig` 字段：
- `baseUrl`（`Option<String>`，默认 `None`）— 厂商 API 基础地址
- `apiKey`（`Option<String>`，默认 `None`）— 认证密钥
- `api`（`Option<String>`，默认 `None`）— API 版本路径
- `models`（`Vec<ModelDefinition>`，默认 `[]`）— 该厂商下的模型定义列表

`ModelDefinition` 字段：
- `id`（`String`，必填）— 模型唯一标识
- `name`（`Option<String>`，默认 `None`）— 显示名称
- `enabled`（`Option<bool>`，默认 `None`）— 是否启用

**验证规则**：
- provider id 非空
- model id 非空
- `baseUrl` 为空字符串（`""`）时跳过校验；非空时必须以 `http://` 或 `https://` 开头
- `apiKey` 为空字符串（`""`）时返回错误；`None`（字段缺失）时跳过校验

**查询接口**：
- `get_provider(id)` — 按 provider id 查找 ProviderConfig
- `get_model(provider_id, model_id)` — 按 provider id 和 model id 查找 ModelDefinition
- `enabled_providers()` — 返回存在至少一个 enabled=true 模型的 provider id 列表

**构造方法**：
- `from_file(path)` — 从文件路径加载
- `from_json_str(content)` — 从 JSON 字符串加载（测试用）

**`is_default` 判断**：mode = `"merge"` 且 providers 为空时返回 true。

**`ChannelsConfigData`** — 从 `channels.json` 加载 channels 和 bindings 配置，实现 `ConfigProvider` trait。channels 为 channel type → 配置值的映射，bindings 为路由规则列表。

**数据结构**：
- `ChannelsConfigData` — 根配置（channels + bindings）
- `BindingEntry` — 单条路由规则（agent_id + match）
- `BindingMatch` — 匹配条件（channel + account_id）

**常量**：`ALLOWED_CHANNEL_TYPES` 列出所有支持的 channel type（feishu / discord / telegram / slack / whatsapp / signal / matrix / msteams / mattermost / nostr / nextcloud-talk / synology-chat / line / googlechat / bluebubbles / imessage / irc / qqbot / twitch / openclaw）。

**验证规则**：
- 所有 channel type key 必须在允许列表中
- 每个 binding 的 agent_id / match.channel / match.account_id 均不能为空

**查询接口**：
- `enabled_channels()` — 返回 `enabled = true` 的 channel key 列表
- `get_channel(channel_type)` — 按 channel type 查找对应配置值
- `get_bindings_by_channel(channel_type)` — 返回 match.channel 等于给定值的 binding 列表
- `get_bindings_by_account(account_id)` — 返回 match.account_id 等于给定值的 binding 列表

**构造方法**：
- `from_file(path)` — 从文件路径加载
- `from_json_str(content)` — 从 JSON 字符串加载（测试用）

**`is_default` 判断**：channels 和 bindings 均为空时返回 true。

**`CredentialsProvider`** — 从 `config/credentials/` 目录加载各 provider 的 JSON 凭据文件，实现 `ConfigProvider` trait。目录不存在时返回空 provider。

**子结构**：`ApiKeyCredentials` / `FeishuCredentials` / `AnyProviderCredentials` / `CredentialsProvider`

`ApiKeyCredentials` 字段：
- `provider`（String）— provider 名称
- `apiKey`（String）— API 密钥

`FeishuCredentials` 字段：
- `provider`（String）— provider 名称
- `appId`（String）— 飞书应用 app_id
- `appSecret`（String）— 飞书应用 app_secret
- `botName`（`Option<String>`，默认 `None`）— 机器人名称

`AnyProviderCredentials` — `ApiKey` 和 `Feishu` 两个变体的无标签枚举（`#[serde(untagged)]`），支持不同 provider 的凭据格式混存。

`CredentialsProvider` 字段：
- `providers`（`HashMap<String, AnyProviderCredentials>`）— 按 provider 名称索引的凭据集合

**加载行为**：
- `load_from_dir(dir)` — 扫描 `dir/*.json`，按文件名（不含扩展名）作为 provider key 存入 HashMap；目录不存在返回 `Err(ConfigError::IoError)`；JSON 反序列化失败的文件跳过（silently continue）
- `from_json_str(content)` — 从 JSON 字符串解析（测试用）

**验证规则**：
- `ApiKey` 变体：`apiKey` 不能为空字符串
- `Feishu` 变体：`appId` 和 `appSecret` 均不能为空字符串

**查询接口**：
- `get(provider)` — 按名称查找对应凭据变体
- `get_api_key(provider)` — 返回 `ApiKey` 变体的 apiKey，`Feishu` 变体返回 `None`
- `feishu_creds()` — 返回第一个 `Feishu` 变体的引用（若存在）

**构造方法**：
- `load_from_dir(path)` — 从目录加载所有 JSON 文件
- `from_json_str(content)` — 从 JSON 字符串构造（测试用）

**`is_default` 判断**：providers 为空时返回 true。

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
