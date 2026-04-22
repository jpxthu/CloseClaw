# SPEC: Session 模块规格说明书

> 本文档按 SPEC_CONVENTION.md v3 编写，描述模块做什么、公开接口和架构结构，不含详细字段列表或函数签名。

---

## 1. 模块概述

Session 模块负责 OpenClaw 会话的持久化恢复和 bootstrap 上下文保护，是网关稳定性的核心保障。

**核心思路**：会话状态以 Checkpoint 形式定期持久化到存储后端，网关重启时从存储恢复。Session 启动时注入的 bootstrap 内容（AGENTS.md、SOUL.md 等）在 compaction 期间受到保护，确保摘要操作不会扭曲 agent 的身份上下文。

**子模块组织**：
- `bootstrap` — compaction 期间保护 agent bootstrap 文件不被摘要扭曲
- `persistence` — Checkpoint 数据结构 + 持久化服务接口 + 本地缓存管理器
- `events` — Checkpoint 触发时机定义（模式切换/消息发送/网关关闭/compaction）
- `recovery` — 网关启动时从存储恢复会话
- `storage/` — 可插拔存储后端（Memory/Redis/SqliteStorage）

---

## 2. 公开接口（mod.rs re-export）

按工作流分组，组内按字母序排列：

### 构造 / 配置

| 接口 | 功能 |
|------|------|
| `BootstrapProtection::new()` | 创建无 workspace 的保护器（用于 transcript 扫描） |
| `BootstrapProtection::with_workspace(PathBuf)` | 创建带 workspace 路径的保护器（用于 reinject） |
| `BootstrapProtection::with_bootstrap_files(Vec<String>)` | 自定义要保护的 bootstrap 文件列表 |
| `BootstrapProtection::with_size_limit(usize)` | 设置 reinject 字符数上限（默认 60K） |
| `MemoryStorage::new()` | 创建内存存储后端（测试/单实例用） |
| `RedisStorage::new(&str, &str) -> Result<Self, PersistenceError>` | 从 URL 创建 Redis 存储后端 |
| `SqliteStorage::new(&Path) -> Result<Self, PersistenceError>` | 从路径创建 SQLite 存储后端（自动建库+Schema） |
| `RedisStorage::key_prefix(&self) -> &str` | 查询存储使用的 key 前缀 |
| `CheckpointManager::new(Arc<S>)` | 创建带存储后端的 Checkpoint 管理器 |
| `SessionRecoveryService::new(Arc<S>)` | 创建恢复服务 |

### 主操作

| 接口 | 功能 |
|------|------|
| `BootstrapProtection::protect_session(&str) -> (String, BootstrapContext)` | 扫描 transcript 中已有的 bootstrap 内容并标记，返回标记后 transcript 和上下文 |
| `BootstrapProtection::before_compact(&mut BootstrapContext)` | 存储所有 region 的 hash，供 compaction 后校验 |
| `BootstrapProtection::after_compact(&str, &mut BootstrapContext) -> Vec<String>` | 检测 bootstrap 内容是否在 compaction 后被扭曲，返回需 reinject 的文件名列表 |
| `BootstrapProtection::reinject(&[String], &mut BootstrapContext) -> Result<String, BootstrapProtectionError>` | 从 workspace 读取 bootstrap 文件，生成带标记的注入文本供 prepend |
| `CheckpointManager::save(SessionCheckpoint)` | 异步保存 checkpoint（不阻塞主流程） |
| `CheckpointManager::save_sync(SessionCheckpoint)` | 同步保存 checkpoint（用于网关关闭） |
| `SessionRecoveryService::set_restore_callback(F)` | 设置恢复回调，接收 session_id 和 checkpoint |
| `SessionRecoveryService::recover() -> Result<RecoveryReport, PersistenceError>` | 扫描所有活跃 session 并逐个恢复 |

### 查询

| 接口 | 功能 |
|------|------|
| `BootstrapProtection::read_bootstrap_files() -> Result<HashMap<String, String>, BootstrapProtectionError>` | 从 workspace 读取所有 bootstrap 文件内容 |
| `BootstrapProtection::workspace_path() -> Option<&PathBuf>` | 获取 workspace 路径 |
| `BootstrapProtection::bootstrap_files() -> &[String]` | 获取当前保护的 bootstrap 文件列表 |
| `CheckpointManager::load(&str) -> Result<Option<SessionCheckpoint>, PersistenceError>` | 加载 checkpoint（优先本地缓存，未命中查存储） |
| `CheckpointManager::cached_session_ids() -> Vec<String>` | 获取本地缓存中的所有 session_id |
| `CheckpointManager::storage(&self) -> &S` | 获取底层存储服务引用 |
| `SessionRecoveryService::storage(&self) -> &S` | 获取底层存储服务引用 |

### 清理

| 接口 | 功能 |
|------|------|
| `CheckpointManager::delete(&str)` | 删除 checkpoint（同时清本地缓存和存储） |
| `CheckpointManager::clear_cache()` | 清空本地缓存 |
| `CheckpointManager::archive(SessionCheckpoint)` | 归档 checkpoint（从活跃存储移至归档存储，清缓存） |
| `CheckpointManager::restore(&str)` | 恢复已归档 checkpoint（从归档移回活跃存储） |
| `CheckpointManager::purge(&str)` | 永久删除已归档 checkpoint |
| `CheckpointManager::archived_session_ids()` | 列出所有已归档的 session_id |

### 公开数据类型（不含字段列表）

| 类型 | 说明 |
|------|------|
| `BootstrapContext` | bootstrap 区域元数据容器，含 regions 列表和 integrity hash |
| `BootstrapRegion` | 单个 bootstrap 文件的区域标记（含 hash 用于完整性校验） |
| `BootstrapProtectionError` | bootstrap 保护操作错误（FileNotFound/IntegrityCheckFailed/IoError/MarkerParseError/WorkspacePathRequired） |
| `SessionCheckpoint` | 会话持久化状态快照（含 status/last_message_at/message_count/channel/chat_id 等生命周期字段） |
| `SessionStatus` | 会话生命周期状态枚举（Active/Archived） |
| `ReasoningMode` | 推理模式枚举（Direct/Plan/Stream/Hidden） |
| `ReasoningModeState` | 推理模式运行时状态（步骤计数/步骤消息/完成标志） |
| `PendingMessage` | 未最终确认的中间消息 |
| `PersistenceService` | 持久化存储接口（save/load/delete/list_active_sessions + archive/restore/purge/list_archived，后四个有默认实现：archive/restore/purge 返回 NotFound，list_archived 返回空列表） |
| `PersistenceError` | 持久化操作错误（Redis/Postgres/Io/Serialization/NotFound/Lock） |
| `CheckpointTrigger` | Checkpoint 触发时机（ModeSwitch/MessageSent/GatewayShutdown/PreCompact/PostCompact） |
| `ModeSwitchEvent` | 模式切换事件（含 from/to mode 和 user_intent） |
| `UserIntent` | 解析后的用户意图（raw_input/parsed_goal/entities） |
| `RecoveryReport` | 恢复结果报告（recovered/failed 列表 + is_full_success/total） |

---

## 3. 架构与数据流

### 3.1 Bootstrap 保护流程

```
Session 启动
    ↓
protect_session() — 扫描 transcript，找到 bootstrap 内容，打上标记
    ↓
before_compact() — 存储所有 region hash
    ↓
Compaction 发生（摘要 transcript）
    ↓
after_compact() — 用 pre_compact hash 校验标记区域内容
    ↓
若内容被扭曲 → reinject() — 从 workspace 重新读取原始文件，生成带标记文本
    ↓
prepend 到 transcript 头部
```

**标记格式**：
```
<bootstrap:file=AGENTS.md,hash=abc123def456,chars=1234,reinject=false>
[原始内容]
</bootstrap>
```

**完整性校验**：SHA-256 前 12 位 hex 前缀匹配。

### 3.2 Checkpoint 持久化流程

```
CheckpointTrigger 事件触发
    ↓
CheckpointManager::save() — 更新本地缓存 + tokio::spawn 异步写存储
    ↓
GatewayShutdown — save_sync() 确保同步落盘
    ↓
存储后端（PersistenceService）— MemoryStorage / RedisStorage / SqliteStorage
```

**加载优先顺序**：本地缓存 → 存储后端。

### 3.3 归档流程

```
CheckpointManager::archive()
    ↓
archive_checkpoint() — 写入归档存储
    ↓
delete_checkpoint() — 从活跃存储删除 + 清本地缓存

CheckpointManager::restore()
    ↓
restore_checkpoint() — 从归档存储读取
    ↓
save_checkpoint() — 写回活跃存储 + purge 归档 + 更新缓存

CheckpointManager::purge() — 永久删除归档记录
CheckpointManager::archived_session_ids() — 列出归档 session
```

### 3.4 会话恢复流程

```
网关启动
    ↓
SessionRecoveryService::recover()
    ↓
遍历所有活跃 session（list_active_sessions）
    ↓
逐个加载 checkpoint（load_checkpoint）
    ↓
调用用户回调（restore_fn）
```

---

## 4. 存储后端约定

| 后端 | 用途 | TTL |
|------|------|-----|
| `MemoryStorage` | 测试/单实例 | 无；内存 HashMap + 独立 archived HashMap |
| `RedisStorage` | 生产 | checkpoint.ttl_seconds（默认 7 天） |
| `SqliteStorage` | 生产（单实例/嵌入式） | 无；SQLite 元数据 + JSONL transcript 文件；归档/恢复/清理；两阶段事务保证原子性 |

RedisStorage 的 `list_active_sessions()` 使用 `KEYS` 命令扫描。

SqliteStorage 将 checkpoint 元数据（session_id/status/created_at/archived_at 等）存入 SQLite 表，transcript 内容写入独立 JSONL 文件，通过两阶段事务（BEGIN IMMEDIATE / COMMIT）保证 DB 与文件系统操作原子性。归档时 `std::fs::rename` 移动 transcript 文件，DB 与文件系统状态一致。

---

## 5. 已知偏差（v3 重写后）

| 偏差项 | 类型 | 说明 | 状态 |
|--------|------|------|------|
| PostgreSQL 存储后端 | 少了 | 代码无实现，旧的偏差表曾记录此为偏差 | 记入偏差表待确认 |
| File 存储后端 | 少了 | 代码无实现，旧的偏差表曾记录此为偏差 | 记入偏差表待确认 |

**注**：v3 标准下，许多旧偏差表中的条目（字段顺序、签名详细程度、类型名等）已不算有效偏差，不在此处重复记录。按 v3 判断标准重写后，`BootstrapProtectionError` 变体名、`verify_integrity` 形式、`PostCompact` 字段顺序等旧条目均已失效。
