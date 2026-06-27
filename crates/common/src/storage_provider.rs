//! Storage provider trait for session persistence abstraction.
//!
//! Decouples the gateway from the concrete persistence implementation,
//! allowing the storage backend to be swapped or mocked in tests.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Session status during persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// Session is active and processing messages.
    Active,
    /// Session is idle (no pending work).
    Idle,
    /// Session has been stopped.
    Stopped,
}

/// A snapshot of session state for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    /// Unique session identifier.
    pub session_id: String,
    /// Agent ID associated with this session.
    pub agent_id: String,
    /// Channel identifier.
    pub channel: String,
    /// Current session status.
    pub status: SessionStatus,
    /// Unix timestamp of the last activity.
    pub last_activity: i64,
}

/// Result of a persistence operation.
#[derive(Debug)]
pub enum PersistResult {
    /// Data was successfully persisted.
    Success,
    /// Data was persisted but with warnings.
    PartialSuccess { warnings: Vec<String> },
    /// Persistence failed.
    Failure(String),
}

/// Trait for session persistence operations.
///
/// Implemented by the concrete persistence service (e.g. SQLite);
/// used by gateway and session manager to avoid direct dependency
/// on the persistence crate internals.
#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Save a session checkpoint to persistent storage.
    async fn save_checkpoint(&self, checkpoint: &SessionCheckpoint) -> PersistResult;

    /// Load a session checkpoint from persistent storage.
    async fn load_checkpoint(&self, session_id: &str) -> Option<SessionCheckpoint>;

    /// Delete a session checkpoint from persistent storage.
    async fn delete_checkpoint(&self, session_id: &str) -> bool;

    /// List all stored session checkpoints.
    async fn list_checkpoints(&self) -> Vec<SessionCheckpoint>;

    /// Flush all in-memory state to persistent storage.
    async fn flush(&self) -> PersistResult;
}
