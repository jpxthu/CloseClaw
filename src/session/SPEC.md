# SPEC: Session 模块规格说明书

> 本文档描述 `src/session/` 模块的精确功能规格，以代码实现为准。

---

## 1. 模块概述

**职责**：Session 模块负责 OpenClaw 会话的持久化恢复和 bootstrap 上下文保护，是 CloseClaw 网关稳定性的核心保障。

**子模块组织**：

```
src/session/
├── mod.rs           # 模块入口，re-export 公开类型
├── bootstrap.rs     # Bootstrap 上下文保护（compaction 防护）
├── events.rs        # Checkpoint 触发事件和模式切换事件定义
├── persistence.rs   # 核心数据结构 + PersistenceService Trait + CheckpointManager
├── recovery.rs      # Session 恢复服务
└── storage/
    ├── mod.rs       # 存储后端统一导出
    ├── memory.rs    # 内存存储（测试用）
    └── redis.rs     # Redis 存储（生产用）
```

---

## 2. 公开 API（re-export）

以下类型从 `mod.rs` 统一导出，供其他模块使用：

```rust
// Bootstrap 保护
pub use bootstrap::{BootstrapContext, BootstrapProtection, BootstrapRegion};

// 持久化核心
pub use persistence::{
    CheckpointManager, PersistenceError, PersistenceService,
    ReasoningMode, SessionCheckpoint,
};

// 事件
pub use events::{CheckpointTrigger, ModeSwitchEvent, UserIntent};
```

---

## 3. Bootstrap 上下文保护（bootstrap.rs）

### 3.1 职责

在 OpenClaw 触发 session compaction 时，确保 agent 的 bootstrap 文件（AGENTS.md、SOUL.md、IDENTITY.md、USER.md）不被摘要扭曲，并在 compaction 完成后重新注入完整的 bootstrap 内容。

### 3.2 核心数据结构

#### BootstrapRegion

```rust
pub struct BootstrapRegion {
    pub region_id: String,           // 唯一标识（session 作用域）
    pub file_name: String,           // 文件名，如 "AGENTS.md"
    pub content_hash: String,        // SHA-256 前12位 hex（完整性校验）
    pub char_count: usize,           // 原始字符数
    pub is_reinject: bool,           // 是否为 compaction 后重新注入
    pub injected_at: DateTime<Utc>,  // 注入时间戳
    pub transcript_offset: Option<usize>, // 在 transcript 中的偏移（预留）
}
```

#### BootstrapContext

```rust
pub struct BootstrapContext {
    pub regions: Vec<BootstrapRegion>,           // 当前追踪的所有 region
    pub reinjected_after_last_compact: bool,     // 上次 compaction 后是否已重注入
    pub total_char_count: usize,                 // 所有 bootstrap 内容的总字符数
    pub pre_compact_hashes: HashMap<String, String>, // compaction 前存储的 hash（key=region_id）
}
```

### 3.3 行为规范

#### protect_session(transcript: &str) → (String, BootstrapContext)

1. 扫描 transcript，找到已有 bootstrap 内容区域（通过启发式匹配）
2. 用 `BOOTSTRAP_REGION_START` / `BOOTSTRAP_REGION_END` 标记包裹每个 bootstrap 文件内容
3. 返回修改后的 transcript 和初始 BootstrapContext

#### before_compact(ctx: &mut BootstrapContext)

存储所有 region 的 content_hash 到 `pre_compact_hashes`，供 compaction 后校验。

#### after_compact(transcript: &str, ctx: &mut BootstrapContext) → Vec<String>

1. 遍历所有 region，在 transcript 中查找对应标记区域
2. 提取标记内的内容，调用 `region.verify_integrity()` 校验 hash
3. 返回需要重新注入的文件名列表

#### reinject(file_names: &[String], ctx: &mut BootstrapContext) → Result<String, BootstrapProtectionError>

1. 从 workspace 路径读取原始 bootstrap 文件
2. 为每个文件创建 `is_reinject=true` 的 BootstrapRegion
3. 生成带标记的注入文本，返回待 prepend 的完整字符串
4. 超出 size_limit（默认 60K chars）时记录 warning log
5. 错误时返回 `BootstrapProtectionError` 变体（如 `WorkspacePathRequired`、`IoError` 等）

#### verify_integrity(content: &str) → bool

使用 SHA-256 前 12 位 hex 进行前缀匹配校验。

### 3.4 标记格式

```
<bootstrap:file=AGENTS.md,hash=abc123def456,chars=1234,reinject=false>
[原始文件内容]
</bootstrap>
```

### 3.5 公开常量和辅助函数

```rust
pub const BOOTSTRAP_REGION_START: &str = "<bootstrap:file=";
pub const BOOTSTRAP_REGION_END: &str = "</bootstrap>";
pub fn make_bootstrap_marker(...) -> String; // 测试用公开函数

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootstrapProtectionError {
    FileNotFound(String),                      // bootstrap 文件不存在
    IntegrityCheckFailed(String),             // 内容完整性校验失败
    IoError(std::io::Error),                 // IO 错误
    MarkerParseError(String),                 // 标记解析失败
    WorkspacePathRequired,                     // reinject 需要 workspace 路径（无 payload）
}
```

> 注：`make_bootstrap_marker` 是无限制的公开函数（无 `#[cfg(test)]` 限制），路径使用 `file_name` 字段从 `BootstrapRegion` 获取。

---

## 4. 持久化层（persistence.rs）

### 4.1 ReasoningMode 枚举

```rust
pub enum ReasoningMode {
    Direct,  // 直接回答模式
    Plan,    // 规划模式（先展示思考框架）
    Stream,  // 流式输出模式
    Hidden,  // 隐藏思考过程模式
}
```

实现 `Default`（默认 Direct）、`Copy`、`PartialEq`、`Eq`、`Serialize`、`Deserialize`、`Display`。

### 4.2 ReasoningModeState 结构

```rust
pub struct ReasoningModeState {
    pub current_step: u32,      // 当前步骤编号（1-indexed）
    pub total_steps: u32,      // 总步骤数
    pub step_messages: Vec<String>, // 各步骤输出内容
    pub is_complete: bool,     // 是否完成
}
```

提供 `start_step()`、`add_step_message()`、`complete()` 方法。

### 4.3 PendingMessage 结构

```rust
pub struct PendingMessage {
    pub message_id: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub sent: bool,  // 是否已发送
}
```

提供 `new()` 和 `mark_sent()` 方法。

### 4.4 SessionCheckpoint 结构

```rust
pub struct SessionCheckpoint {
    pub session_id: String,
    pub last_message_id: Option<String>,
    pub mode_state: ReasoningModeState,
    pub pending_messages: Vec<PendingMessage>,
    pub mode: ReasoningMode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub ttl_seconds: u64,  // 默认 604800（7 天），0 表示不过期
}
```

提供 `new()` 和多个 `with_*()` builder 方法，以及 `touch()` 更新 `updated_at`。

### 4.5 PersistenceService Trait

```rust
#[async_trait]
pub trait PersistenceService: Send + Sync {
    async fn save_checkpoint(&self, checkpoint: &SessionCheckpoint) -> Result<(), PersistenceError>;
    async fn load_checkpoint(&self, session_id: &str) -> Result<Option<SessionCheckpoint>, PersistenceError>;
    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError>;
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError>;
}
```

### 4.6 CheckpointManager<S: PersistenceService>

提供：

- `save(checkpoint)` — 异步写入，不阻塞主流程
- `save_sync(checkpoint)` — 同步写入（用于网关关闭）
- `load(session_id)` — 优先查本地缓存，未命中则查存储
- `delete(session_id)` — 删除缓存和存储
- `clear_cache()` — 清空本地缓存
- `cached_session_ids()` — 获取缓存中的所有 session_id

本地缓存：`RwLock<HashMap<String, SessionCheckpoint>>`

### 4.7 PersistenceError 枚举

```rust
pub enum PersistenceError {
    Redis(String),
    Postgres(String),
    Io(std::io::Error),
    Serialization(serde_json::Error),
    NotFound(String),
    Lock(String),
}
```

实现 `std::error::Error` + `From` 常见错误类型。

---

## 5. 存储后端

### 5.1 MemoryStorage

```rust
pub struct MemoryStorage {
    checkpoints: RwLock<HashMap<String, SessionCheckpoint>>,
}
```
- 用途：测试和单实例部署
- 实现 `PersistenceService` Trait

### 5.2 RedisStorage

```rust
pub struct RedisStorage {
    client: redis::Client,
    key_prefix: String,
}
```

**方法**：
- `new(redis_url: &str, key_prefix: &str) -> Result<Self, PersistenceError>` — 构造 Redis 存储
- `key_prefix(&self) -> &str` — 获取 key 前缀

- TTL：使用 checkpoint.ttl_seconds，默认 604800 秒
- `list_active_sessions()`：使用 `KEYS` 命令扫描

---

## 6. Checkpoint 触发事件（events.rs）

### 6.1 CheckpointTrigger 枚举

```rust
pub enum CheckpointTrigger {
    ModeSwitch { from_mode: ReasoningMode, to_mode: ReasoningMode },
    MessageSent { message_id: String },
    GatewayShutdown,  // requires_sync = true
    PreCompact { before_char_count: usize },
    PostCompact { after_char_count: usize, before_char_count: usize },
}
```

- `requires_sync()`：仅 `GatewayShutdown` 返回 true

### 6.2 ModeSwitchEvent 结构

```rust
pub struct ModeSwitchEvent {
    pub requested_mode: Option<ReasoningMode>,
    pub target_mode: Option<ReasoningMode>,
    pub user_intent: Option<Arc<UserIntent>>,
    pub session_id: Option<String>,
}
```

### 6.3 UserIntent 结构

```rust
pub struct UserIntent {
    pub raw_input: String,
    pub parsed_goal: Option<String>,
    pub entities: Vec<String>,
}
```

---

## 7. 恢复服务（recovery.rs）

### 7.1 SessionRecoveryService<S: PersistenceService>

```rust
pub struct SessionRecoveryService<S: PersistenceService> {
    storage: Arc<S>,
    restore_fn: RwLock<Option<Box<dyn Fn(&str, &SessionCheckpoint) -> Result<(), PersistenceError> + Send + Sync>>>,
}
```

- `storage(&self) -> &S` — 获取底层持久化服务引用
- `set_restore_callback()` — 设置恢复回调，接收 session_id 和 checkpoint
- `recover()` — 扫描所有活跃 session，逐个恢复，返回 `RecoveryReport`
- `recover_session()` — 恢复单个 session（从存储加载 checkpoint 并调用回调）

### 7.2 RecoveryReport 结构

```rust
pub struct RecoveryReport {
    pub recovered: Vec<String>,  // 成功恢复的 session_id 列表
    pub failed: Vec<String>,     // 恢复失败的 session_id 列表
}
```

提供 `is_full_success()` 和 `total()` 方法。

---

## 8. 行为规范总结

| 功能 | 行为 |
|------|------|
| Bootstrap 完整性校验 | SHA-256 前12位 hex 前缀匹配 |
| Compaction 保护 | before_compact 存 hash → after_compact 校验 → reinject |
| Checkpoint 异步保存 | GatewayShutdown 同步写入，其他异步 tokio::spawn |
| Checkpoint 加载 | 先查本地缓存，未命中查存储后端 |
| Recovery | 扫描活跃 session → 逐个加载 checkpoint → 调用回调 |

---

## 9. 测试覆盖

| 文件 | 测试内容 |
|------|---------|
| bootstrap.rs | BootstrapRegion 序列化/完整性校验/wrap/parse；BootstrapContext 默认/添加region/校验/exceeds；BootstrapProtection before_compact/after_compact/reinject；make_bootstrap_marker；region_id 唯一性 |
| persistence.rs | CheckpointManager save+load+cache_hit+delete+clear_cache；ReasoningMode Default+Display；PendingMessage mark_sent；ReasoningModeState operations |
| storage/memory.rs | save+load/load_none/delete/list_active_sessions/overwrite |
| storage/redis.rs | make_key/key_prefix/invalid_url；集成测试标注 `#[ignore]` |
| recovery.rs | RecoveryReport is_full_success/total；SessionRecoveryService recover_empty/recover_with_callback/recover_not_found |
| events.rs | CheckpointTrigger.requires_sync() |

**总计：约 40+ 测试用例**，覆盖全部核心逻辑路径。

---

## 10. 已知偏差（无）

`src/session/` 模块与文档高度一致，代码已完整实现文档中所有设计内容。无偏差需记录。
