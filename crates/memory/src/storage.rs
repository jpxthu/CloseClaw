//! Memory storage port — narrow persistence interface.
//!
//! The memory crate (miner + dreaming) consumes only four storage
//! operations: [`MemoryStorage::load_checkpoint`], [`MemoryStorage::mark_mined`],
//! [`MemoryStorage::list_mined_undreamt_sessions`], and
//! [`MemoryStorage::update_dreaming_status`]. Session-owned checkpoint types
//! are replaced by memory-owned snapshot / status / error types; the
//! composition layer (daemon) provides an adapter wrapping the session
//! persistence service at assembly time.

use async_trait::async_trait;
use thiserror::Error;

/// Dreaming processing status.
///
/// Memory-side mirror of the session checkpoint's dreaming status; all
/// five variants map one-to-one onto the session-side semantics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DreamingStatus {
    /// Dreaming not started (initial state).
    #[default]
    Pending,
    /// Light stage in progress.
    InLight,
    /// REM stage in progress.
    InRem,
    /// Deep stage in progress.
    InDeep,
    /// Dreaming completed.
    Completed,
}

/// Snapshot of the checkpoint fields consumed by the memory crate.
///
/// The miner only reads the `mined` flag; `dreaming_status` is carried
/// so storage implementations can filter undreamt sessions.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointSnapshot {
    /// Session unique identifier.
    pub session_id: String,
    /// Whether the session has been mined by the memory-miner.
    pub mined: bool,
    /// Current dreaming processing status.
    pub dreaming_status: DreamingStatus,
}

impl CheckpointSnapshot {
    /// Create a snapshot for the given session.
    pub fn new(
        session_id: impl Into<String>,
        mined: bool,
        dreaming_status: DreamingStatus,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            mined,
            dreaming_status,
        }
    }
}

/// Storage errors surfaced through the memory port.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Backend storage failure.
    #[error("storage backend error: {0}")]
    Backend(String),
    /// Checkpoint not found for the given session.
    #[error("checkpoint not found for session: {0}")]
    NotFound(String),
}

/// Narrow storage port consumed by the memory miner and dreaming pipeline.
#[async_trait]
pub trait MemoryStorage: Send + Sync {
    /// Load a session's checkpoint snapshot.
    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<CheckpointSnapshot>, StorageError>;

    /// Mark the given session as mined by the memory-miner.
    async fn mark_mined(&self, session_id: &str) -> Result<(), StorageError>;

    /// List sessions that are mined but whose dreaming has not completed.
    async fn list_mined_undreamt_sessions(&self) -> Result<Vec<String>, StorageError>;

    /// Update the dreaming status of the given session.
    async fn update_dreaming_status(
        &self,
        session_id: &str,
        status: DreamingStatus,
    ) -> Result<(), StorageError>;
}
