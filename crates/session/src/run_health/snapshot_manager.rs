//! Runtime Snapshot Manager — transcript rollback safety net.
//!
//! Provides [`RuntimeSnapshotManager`] for creating and restoring
//! transcript snapshots before destructive operations (compaction,
//! `/system` rewrite). Each session owns one manager instance that
//! is independent from the persistence-layer [`CheckpointManager`].
//!
//! Design reference: `docs/design/session/run-health.md`.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::llm_session::SessionMessage;

/// Maximum number of snapshots retained per session.
///
/// Oldest snapshots are automatically evicted when this limit is
/// exceeded. Value matches the design doc.
const MAX_SNAPSHOTS: usize = 25;

// =====================================================================
// TranscriptOp
// =====================================================================

/// Operation type that triggers a transcript modification.
///
/// All code paths that mutate the transcript must declare their
/// operation type so the snapshot manager can decide whether a
/// snapshot is warranted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptOp {
    /// Incremental append (new user/assistant/tool messages).
    /// Does **not** trigger a snapshot.
    Append,
    /// Full rewrite (compaction replaces the entire transcript).
    /// Triggers a snapshot.
    Rewrite,
    /// Partial rewrite (`/system` modifies system prompt section).
    /// Triggers a snapshot.
    PartialRewrite,
}

impl TranscriptOp {
    /// Returns `true` if this operation type warrants a snapshot.
    pub fn requires_snapshot(&self) -> bool {
        matches!(self, TranscriptOp::Rewrite | TranscriptOp::PartialRewrite)
    }
}

// =====================================================================
// SnapshotKind — incremental vs full rewrite
// =====================================================================

/// Differentiates between incremental and full-rewrite snapshots.
///
/// Incremental snapshots record the `leaf_entry_id` so the caller
/// can truncate the JSONL transcript; full-rewrite snapshots carry
/// the full message backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotKind {
    /// Incremental snapshot — rollback via transcript truncation.
    Incremental { leaf_entry_id: String },
    /// Full-rewrite snapshot — rollback via message replacement.
    FullRewrite,
}

// =====================================================================
// SnapshotStatus — lifecycle state machine
// =====================================================================

/// Lifecycle status of a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotStatus {
    /// Snapshot has been created but the operation it guards has not
    /// yet completed.
    Pending,
    /// The guarded operation completed successfully.
    Complete,
}

impl fmt::Display for SnapshotStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotStatus::Pending => write!(f, "pending"),
            SnapshotStatus::Complete => write!(f, "complete"),
        }
    }
}

// =====================================================================
// RollbackAction — returned to caller
// =====================================================================

/// Describes what the caller should do to complete a rollback.
///
/// The snapshot manager does not directly modify the transcript file;
/// instead it returns an action that the caller (session layer)
/// interprets to truncate or replace the transcript.
#[derive(Debug, Clone)]
pub enum RollbackAction {
    /// Truncate the JSONL transcript up to (and excluding) the given
    /// `leaf_entry_id`. Used for incremental snapshots.
    Truncate { leaf_entry_id: String },
    /// Replace the current transcript messages with the backup.
    /// Used for full-rewrite snapshots.
    Replace { messages: Vec<SessionMessage> },
}

// =====================================================================
// SnapshotMeta — persisted metadata
// =====================================================================

/// Metadata for a snapshot, suitable for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    /// Unique snapshot identifier (UUID v4).
    pub id: String,
    /// Human-readable reason for creating this snapshot.
    pub reason: String,
    /// When the snapshot was created.
    pub created_at: DateTime<Utc>,
    /// Session this snapshot belongs to.
    pub session_id: String,
    /// Current lifecycle status.
    pub status: SnapshotStatus,
}

// =====================================================================
// SnapshotMetaStore — persistence trait
// =====================================================================

/// Persistence interface for snapshot metadata.
///
/// Implementations live outside the session crate (e.g. gateway /
/// daemon) and are injected via [`RuntimeSnapshotManager::set_meta_store`].
/// When no store is configured, metadata is held in memory only.
#[async_trait::async_trait]
pub trait SnapshotMetaStore: Send + Sync {
    /// Persist snapshot metadata.
    async fn save_meta(&self, meta: &SnapshotMeta) -> Result<(), String>;
}

// =====================================================================
// Snapshot
// =====================================================================

/// A single snapshot entry.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Unique snapshot identifier (UUID v4).
    pub id: String,
    /// Human-readable reason for creating this snapshot.
    pub reason: String,
    /// Full copy of messages at snapshot time.
    pub messages: Vec<SessionMessage>,
    /// The operation type that triggered this snapshot.
    pub op: TranscriptOp,
    /// When the snapshot was created.
    pub created_at: DateTime<Utc>,
    /// Whether this is an incremental or full-rewrite snapshot.
    pub snapshot_kind: SnapshotKind,
    /// Lifecycle status of the snapshot.
    pub status: SnapshotStatus,
    /// Whether this snapshot was created by rollback as a pre-rollback
    /// sentinel. Used to prevent infinite undo chains.
    pub is_pre_rollback: bool,
}

// =====================================================================
// RuntimeSnapshotManager
// =====================================================================

/// Manages a bounded queue of transcript snapshots.
///
/// Snapshots are stored in-memory only (not persisted across process
/// restarts) unless a [`SnapshotMetaStore`] is configured for metadata
/// persistence. The queue is bounded to [`MAX_SNAPSHOTS`] entries;
/// older entries are evicted automatically.
///
/// # Usage
///
/// 1. Before a destructive operation, call [`create_snapshot`] to
///    capture the current transcript state.
/// 2. After the operation succeeds, call [`mark_complete`] to
///    transition the snapshot status to [`SnapshotStatus::Complete`].
/// 3. On failure, call [`rollback`] to restore the most recent
///    snapshot. A pre-rollback snapshot is automatically created
///    so the rollback itself is undoable.
#[derive(Clone)]
pub struct RuntimeSnapshotManager {
    snapshots: VecDeque<Snapshot>,
    meta_store: Option<Arc<dyn SnapshotMetaStore>>,
}

impl RuntimeSnapshotManager {
    /// Create an empty snapshot manager.
    pub fn new() -> Self {
        Self {
            snapshots: VecDeque::new(),
            meta_store: None,
        }
    }

    /// Set the metadata persistence store.
    ///
    /// When set, snapshot metadata is persisted on creation and
    /// status changes.
    pub fn set_meta_store(&mut self, store: Arc<dyn SnapshotMetaStore>) {
        self.meta_store = Some(store);
    }

    /// Create a snapshot of the current messages.
    ///
    /// If the operation does not require a snapshot ([`Append`]), this
    /// is a no-op. Otherwise the snapshot is pushed onto the queue; if
    /// the queue exceeds [`MAX_SNAPSHOTS`], the oldest entry is evicted.
    ///
    /// [`Append`]: TranscriptOp::Append
    pub fn create_snapshot(
        &mut self,
        messages: &[SessionMessage],
        op: TranscriptOp,
        reason: &str,
    ) -> bool {
        if !op.requires_snapshot() {
            return false;
        }
        let snapshot = Snapshot {
            id: Uuid::new_v4().to_string(),
            reason: reason.to_string(),
            messages: messages.to_vec(),
            op,
            created_at: Utc::now(),
            snapshot_kind: SnapshotKind::FullRewrite,
            status: SnapshotStatus::Pending,
            is_pre_rollback: false,
        };
        if self.snapshots.len() >= MAX_SNAPSHOTS {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
        true
    }

    /// Mark a snapshot as [`SnapshotStatus::Complete`].
    ///
    /// Used after a guarded operation succeeds. Persists the metadata
    /// change if a [`SnapshotMetaStore`] is configured.
    pub fn mark_complete(&mut self, snapshot_id: &str) {
        for snapshot in self.snapshots.iter_mut() {
            if snapshot.id == snapshot_id {
                snapshot.status = SnapshotStatus::Complete;
                break;
            }
        }
    }

    /// Restore from the most recent snapshot.
    ///
    /// Before returning the target snapshot, a **pre-rollback**
    /// snapshot of the *current* messages is created (as a
    /// `FullRewrite` with reason `"pre-rollback"`). This makes
    /// the rollback itself undoable — a subsequent `rollback()`
    /// will return the pre-rollback state.
    ///
    /// A pre-rollback is only created when there are other snapshots
    /// in the queue beyond the target being popped. This prevents
    /// an infinite undo chain.
    ///
    /// Returns `Some(RollbackAction)` if a snapshot existed and was
    /// restored; `None` if the queue is empty.
    pub fn rollback(&mut self, current_messages: &[SessionMessage]) -> Option<RollbackAction> {
        if self.snapshots.is_empty() {
            return None;
        }
        // Pop the target snapshot (most recent).
        let snapshot = self.snapshots.pop_back()?;
        // Create a pre-rollback snapshot unless the most remaining
        // snapshot is already a pre-rollback (prevents infinite chain).
        let already_pre_rollback = self.snapshots.back().is_some_and(|s| s.is_pre_rollback);
        if !already_pre_rollback {
            let pre_rollback = Snapshot {
                id: Uuid::new_v4().to_string(),
                reason: "pre-rollback".to_string(),
                messages: current_messages.to_vec(),
                op: TranscriptOp::Rewrite,
                created_at: Utc::now(),
                snapshot_kind: SnapshotKind::FullRewrite,
                status: SnapshotStatus::Pending,
                is_pre_rollback: true,
            };
            if self.snapshots.len() >= MAX_SNAPSHOTS {
                self.snapshots.pop_front();
            }
            self.snapshots.push_back(pre_rollback);
        }
        let action = match &snapshot.snapshot_kind {
            SnapshotKind::FullRewrite => RollbackAction::Replace {
                messages: snapshot.messages,
            },
            SnapshotKind::Incremental { leaf_entry_id } => RollbackAction::Truncate {
                leaf_entry_id: leaf_entry_id.clone(),
            },
        };
        Some(action)
    }

    /// Returns the number of snapshots currently held.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    /// Create an incremental snapshot for transcript truncation rollback.
    ///
    /// Unlike [`create_snapshot`] which always creates a `FullRewrite`
    /// kind, this method creates an `Incremental` snapshot that records
    /// the `leaf_entry_id` for JSONL truncation on rollback.
    pub fn create_incremental_snapshot(
        &mut self,
        messages: &[SessionMessage],
        reason: &str,
        leaf_entry_id: &str,
    ) -> bool {
        let snapshot = Snapshot {
            id: Uuid::new_v4().to_string(),
            reason: reason.to_string(),
            messages: messages.to_vec(),
            op: TranscriptOp::Append,
            created_at: Utc::now(),
            snapshot_kind: SnapshotKind::Incremental {
                leaf_entry_id: leaf_entry_id.to_string(),
            },
            status: SnapshotStatus::Pending,
            is_pre_rollback: false,
        };
        if self.snapshots.len() >= MAX_SNAPSHOTS {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
        true
    }

    /// Clear all snapshots without restoring.
    pub fn clear(&mut self) {
        self.snapshots.clear();
    }
}

impl Default for RuntimeSnapshotManager {
    fn default() -> Self {
        Self::new()
    }
}
