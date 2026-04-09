# SPEC: Session 持久化层 — 网关重启恢复机制

> Issue: [#159](https://github.com/jpxthu/CloseClaw/issues/159)

## 1. 概述

本文档定义 CloseClaw 网关的 **Session 持久化层**设计。核心目标：

1. 在网关重启时恢复会话状态，避免中间消息丢失
2. 支持 `mode_switch`、`message_send`、`gateway_shutdown` 时机保存 checkpoint
3. 提供可配置的存储后端（默认 Redis，支持 PostgreSQL/File 等）
4. 持久化操作异步执行，不阻塞主流程

## 2. 现有结构

### 2.1 现有 Session 管理（待扩展）

根据现有代码，Session 相关信息存储在内存中，网关重启后会丢失。

### 2.2 现有平台消息交互

飞书等平台的消息发送依赖消息 ID，重启后无法关联历史消息。

## 3. 数据结构设计

### 3.1 SessionCheckpoint 结构

```rust
// src/session/persistence.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Session Checkpoint — 用于持久化恢复的核心数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    /// Session 唯一标识
    pub session_id: String,
    /// 最后一条持久化消息的 ID（平台相关）
    pub last_message_id: Option<String>,
    /// 当前推理模式状态
    pub mode_state: ReasoningModeState,
    /// 中间状态消息（尚未最终确认）
    pub pending_messages: Vec<PendingMessage>,
    /// 当前模式
    pub mode: ReasoningMode,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 最后更新时间
    pub updated_at: DateTime<Utc>,
    /// TTL（秒），0 表示不过期
    pub ttl_seconds: u64,
}

/// Reasoning Mode State — 推理模式的状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningModeState {
    /// 当前步骤编号（1-indexed）
    pub current_step: u32,
    /// 总步骤数
    pub total_steps: u32,
    /// 各步骤的输出内容
    pub step_messages: Vec<String>,
    /// 是否完成
    pub is_complete: bool,
}

/// Pending Message — 未最终确认的消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMessage {
    /// 消息 ID
    pub message_id: String,
    /// 消息内容
    pub content: String,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 是否已发送
    pub sent: bool,
}

/// Reasoning Mode — 推理模式枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningMode {
    /// 直接回答模式
    Direct,
    /// 规划模式（先展示思考框架）
    Plan,
    /// 流式输出模式
    Stream,
    /// 隐藏思考过程模式
    Hidden,
}

impl Default for ReasoningMode {
    fn default() -> Self {
        ReasoningMode::Direct
    }
}
```

### 3.2 PersistenceService Trait

```rust
// src/session/persistence.rs

use async_trait::async_trait;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PersistenceError {
    #[error("Redis error: {0}")]
    Redis(String),
    #[error("PostgreSQL error: {0}")]
    Postgres(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Checkpoint not found for session: {0}")]
    NotFound(String),
}

/// 持久化服务接口
#[async_trait]
pub trait PersistenceService: Send + Sync {
    /// 保存 Checkpoint
    async fn save_checkpoint(&self, checkpoint: &SessionCheckpoint) -> Result<(), PersistenceError>;

    /// 加载 Checkpoint
    async fn load_checkpoint(&self, session_id: &str) -> Result<Option<SessionCheckpoint>, PersistenceError>;

    /// 删除 Checkpoint
    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError>;

    /// 列出所有活跃 Session 的 Checkpoint
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError>;
}
```

## 4. 存储后端实现

### 4.1 Redis 后端（默认）

```rust
// src/session/storage/redis.rs

use crate::session::persistence::{PersistenceService, PersistenceError, SessionCheckpoint};
use redis::AsyncCommands;
use std::sync::Arc;

/// Redis 存储后端
pub struct RedisStorage {
    client: redis::Client,
    key_prefix: String,
}

impl RedisStorage {
    pub fn new redis_url: &str, key_prefix: impl Into<String> = "checkpoint") -> Result<Self, PersistenceError> {
        Ok(Self {
            client: redis::Client::open(redis_url)
                .map_err(|e| PersistenceError::Redis(e.to_string()))?,
            key_prefix: key_prefix.into(),
        })
    }

    fn make_key(&self, session_id: &str) -> String {
        format!("{}:{}", self.key_prefix, session_id)
    }
}

#[async_trait]
impl PersistenceService for RedisStorage {
    async fn save_checkpoint(&self, checkpoint: &SessionCheckpoint) -> Result<(), PersistenceError> {
        let mut conn = self.client.get_multiplexed_async_connection().await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        let key = self.make_key(&checkpoint.session_id);
        let value = serde_json::to_string(checkpoint)?;

        // 设置 TTL（默认 7 天 = 604800 秒）
        let ttl = if checkpoint.ttl_seconds > 0 {
            checkpoint.ttl_seconds
        } else {
            604800
        };

        conn.set_ex(&key, &value, ttl).await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn load_checkpoint(&self, session_id: &str) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let mut conn = self.client.get_multiplexed_async_connection().await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        let key = self.make_key(session_id);
        let value: Option<String> = conn.get(&key).await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        match value {
            Some(v) => {
                let checkpoint: SessionCheckpoint = serde_json::from_str(&v)?;
                Ok(Some(checkpoint))
            }
            None => Ok(None),
        }
    }

    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        let mut conn = self.client.get_multiplexed_async_connection().await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        let key = self.make_key(session_id);
        conn.del(&key).await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let mut conn = self.client.get_multiplexed_async_connection().await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        let pattern = format!("{}:*", self.key_prefix);
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query_async(&mut conn)
            .await
            .map_err(|e| PersistenceError::Redis(e.to_string()))?;

        // 提取 session_id（去掉前缀）
        let session_ids = keys.iter()
            .map(|k| k.strip_prefix(&format!("{}:", self.key_prefix)).unwrap_or(k).to_string())
            .collect();

        Ok(session_ids)
    }
}
```

### 4.2 内存后端（测试用）

```rust
// src/session/storage/memory.rs

use crate::session::persistence::{PersistenceService, PersistenceError, SessionCheckpoint};
use std::collections::HashMap;
use std::sync::RwLock;

/// 内存存储后端（仅用于测试）
pub struct MemoryStorage {
    checkpoints: RwLock<HashMap<String, SessionCheckpoint>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            checkpoints: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PersistenceService for MemoryStorage {
    async fn save_checkpoint(&self, checkpoint: &SessionCheckpoint) -> Result<(), PersistenceError> {
        let mut checkpoints = self.checkpoints.write()
            .map_err(|_| PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other, "Lock error"
            )))?;
        checkpoints.insert(checkpoint.session_id.clone(), checkpoint.clone());
        Ok(())
    }

    async fn load_checkpoint(&self, session_id: &str) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        let checkpoints = self.checkpoints.read()
            .map_err(|_| PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other, "Lock error"
            )))?;
        Ok(checkpoints.get(session_id).cloned())
    }

    async fn delete_checkpoint(&self, session_id: &str) -> Result<(), PersistenceError> {
        let mut checkpoints = self.checkpoints.write()
            .map_err(|_| PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other, "Lock error"
            )))?;
        checkpoints.remove(session_id);
        Ok(())
    }

    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let checkpoints = self.checkpoints.read()
            .map_err(|_| PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other, "Lock error"
            )))?;
        Ok(checkpoints.keys().cloned().collect())
    }
}
```

## 5. Checkpoint Manager

### 5.1 CheckpointManager 核心实现

```rust
// src/session/persistence.rs

use std::sync::Arc;
use tokio::sync::RwLock;

/// Checkpoint 管理器 — 负责保存和恢复 Session 状态
pub struct CheckpointManager<S: PersistenceService> {
    storage: Arc<S>,
    /// 本地缓存（减少对存储的访问）
    local_cache: RwLock<HashMap<String, SessionCheckpoint>>,
}

impl<S: PersistenceService> CheckpointManager<S> {
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage,
            local_cache: RwLock::new(HashMap::new()),
        }
    }

    /// 保存 Checkpoint（异步写入，不阻塞主流程）
    pub async fn save(&self, checkpoint: SessionCheckpoint) -> Result<(), PersistenceError> {
        // 先更新本地缓存
        {
            let mut cache = self.local_cache.write().await;
            cache.insert(checkpoint.session_id.clone(), checkpoint.clone());
        }

        // 异步保存到存储后端
        let storage = Arc::clone(&self.storage);
        tokio::spawn(async move {
            if let Err(e) = storage.save_checkpoint(&checkpoint).await {
                tracing::error!(session_id = %checkpoint.session_id, "Failed to save checkpoint: {}", e);
            }
        });

        Ok(())
    }

    /// 加载 Checkpoint（优先本地缓存）
    pub async fn load(&self, session_id: &str) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        // 先查本地缓存
        {
            let cache = self.local_cache.read().await;
            if let Some(cp) = cache.get(session_id) {
                return Ok(Some(cp.clone()));
            }
        }

        // 缓存未命中，从存储加载
        let cp = self.storage.load_checkpoint(session_id).await?;

        if let Some(ref checkpoint) = cp {
            // 更新本地缓存
            let mut cache = self.local_cache.write().await;
            cache.insert(session_id.to_string(), checkpoint.clone());
        }

        Ok(cp)
    }

    /// 删除 Checkpoint
    pub async fn delete(&self, session_id: &str) -> Result<(), PersistenceError> {
        // 删除本地缓存
        {
            let mut cache = self.local_cache.write().await;
            cache.remove(session_id);
        }

        // 删除存储中的数据
        self.storage.delete_checkpoint(session_id).await
    }
}
```

## 6. 触发时机

### 6.1 触发时机定义

```rust
// src/session/events.rs

/// Checkpoint 触发时机
#[derive(Debug, Clone, Copy)]
pub enum CheckpointTrigger {
    /// 模式切换时
    ModeSwitch {
        from_mode: ReasoningMode,
        to_mode: ReasoningMode,
    },
    /// 消息发送后
    MessageSent {
        message_id: String,
    },
    /// 网关关闭前（同步写入）
    GatewayShutdown,
}

impl CheckpointTrigger {
    /// 是否需要同步写入
    pub fn requires_sync(&self) -> bool {
        matches!(self, CheckpointTrigger::GatewayShutdown)
    }
}
```

### 6.2 集成到 Session 管理

```rust
// src/session/mod.rs

pub struct SessionManager {
    // ... existing fields
    checkpoint_manager: Arc<CheckpointManager<dyn PersistenceService>>,
}

impl SessionManager {
    /// 触发 Checkpoint 保存
    pub async fn trigger_checkpoint(
        &self,
        session_id: &str,
        trigger: CheckpointTrigger,
    ) -> Result<(), PersistenceError> {
        let checkpoint = self.build_checkpoint(session_id).await?;

        if trigger.requires_sync() {
            // 同步写入（网关关闭时）
            self.checkpoint_manager.storage.save_checkpoint(&checkpoint).await?;
        } else {
            // 异步写入
            self.checkpoint_manager.save(checkpoint).await?;
        }

        Ok(())
    }

    /// 构建当前 Session 的 Checkpoint
    async fn build_checkpoint(&self, session_id: &str) -> Result<SessionCheckpoint, PersistenceError> {
        // 从 Session 状态构建 checkpoint
        // ... 省略实现细节
    }
}
```

## 7. 网关重启恢复流程

### 7.1 恢复流程设计

```
网关启动
  ↓
扫描活跃 session（调用 storage.list_active_sessions()）
  ↓
对每个活跃 session：
  读取 checkpoint（调用 storage.load_checkpoint()）
  恢复 modeState
  重建 pendingMessages
  恢复最后一个 confirmed 消息的显示状态
  ↓
将恢复状态注入到当前 session 上下文
```

### 7.2 恢复服务实现

```rust
// src/session/recovery.rs

use crate::session::persistence::{PersistenceService, SessionCheckpoint};

/// Session 恢复服务
pub struct SessionRecoveryService<S: PersistenceService> {
    storage: Arc<S>,
    session_manager: Arc<SessionManager>,
}

impl<S: PersistenceService> SessionRecoveryService<S> {
    /// 执行恢复流程
    pub async fn recover(&self) -> Result<RecoveryReport, PersistenceError> {
        let active_sessions = self.storage.list_active_sessions().await?;
        let mut recovered = Vec::new();
        let mut failed = Vec::new();

        for session_id in active_sessions {
            match self.recover_session(&session_id).await {
                Ok(()) => recovered.push(session_id),
                Err(e) => {
                    tracing::error!(session_id = %session_id, "Failed to recover session: {}", e);
                    failed.push(session_id);
                }
            }
        }

        Ok(RecoveryReport { recovered, failed })
    }

    /// 恢复单个 Session
    async fn recover_session(&self, session_id: &str) -> Result<(), PersistenceError> {
        let checkpoint = self.storage.load_checkpoint(session_id).await?
            .ok_or_else(|| PersistenceError::NotFound(session_id.to_string()))?;

        // 重建 Session 状态
        self.session_manager.restore_from_checkpoint(checkpoint).await?;

        Ok(())
    }
}

/// 恢复报告
#[derive(Debug)]
pub struct RecoveryReport {
    pub recovered: Vec<String>,
    pub failed: Vec<String>,
}
```

## 8. 配置

### 8.1 配置项

```yaml
# config/default.yaml

session_persistence:
  # 存储后端类型: redis | postgres | memory
  backend: redis

  redis:
    url: "redis://localhost:6379"
    key_prefix: "checkpoint"
    default_ttl_seconds: 604800  # 7 days

  postgres:
    connection_string: "postgresql://user:pass@localhost/closeclaw"
    table_name: "session_checkpoints"

  # 是否启用持久化
  enabled: true

  # 异步写入队列大小
  async_queue_size: 1000
```

## 9. 验收标准

- [ ] `SessionCheckpoint` 数据结构完整定义
- [ ] Checkpoint 在 `mode_switch`、`message_send`、`gateway_shutdown` 时被正确保存
- [ ] 网关重启后，`load_checkpoint` 能恢复所有活跃 session 的状态
- [ ] 恢复后的 pendingMessages 在飞书上有合理的展示（不丢失用户体验）
- [ ] 存储选型可配置（默认 Redis，支持替换为 PostgreSQL/File 等）
- [ ] 持久化操作不影响主流程响应时间（异步写入）
- [ ] Redis 存储后端实现完整
- [ ] 内存存储后端实现（用于测试）
- [ ] CheckpointManager 提供本地缓存减少存储访问
- [ ] 网关关闭时同步写入 Checkpoint
- [ ] 提供 `SessionRecoveryService` 执行启动恢复

## 10. 文件结构

```
src/session/
├── mod.rs                    # Session 管理器入口
├── persistence.rs             # 核心数据结构和 PersistenceService Trait
├── recovery.rs                # 恢复服务
├── storage/
│   ├── mod.rs                # 存储后端导出
│   ├── redis.rs              # Redis 后端实现
│   └── memory.rs             # 内存后端实现（测试用）
└── events.rs                 # Checkpoint 触发事件定义
```
