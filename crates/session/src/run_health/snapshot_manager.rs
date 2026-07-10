//! Runtime Snapshot Manager — transcript rollback safety net.
//!
//! Provides [`RuntimeSnapshotManager`] for creating and restoring
//! transcript snapshots before destructive operations (compaction,
//! `/system` rewrite). Each session owns one manager instance that
//! is independent from the persistence-layer [`CheckpointManager`].
//!
//! Design reference: `docs/design/session/run-health.md`.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};

use crate::llm_session::SessionMessage;

/// Maximum number of snapshots retained per session.
///
/// Oldest snapshots are automatically evicted when this limit is
/// exceeded. Value matches the design doc.
const MAX_SNAPSHOTS: usize = 25;

/// Operation type that triggers a transcript modification.
///
/// All code paths that mutate the transcript must declare their
/// operation type so the snapshot manager can decide whether a
/// snapshot is warranted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A single snapshot entry.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Full copy of messages at snapshot time.
    pub messages: Vec<SessionMessage>,
    /// The operation type that triggered this snapshot.
    pub op: TranscriptOp,
    /// When the snapshot was created.
    pub created_at: DateTime<Utc>,
}

/// Manages a bounded queue of transcript snapshots.
///
/// Snapshots are stored in-memory only (not persisted across process
/// restarts). The queue is bounded to [`MAX_SNAPSHOTS`] entries;
/// older entries are evicted automatically.
///
/// # Usage
///
/// 1. Before a destructive operation, call [`create_snapshot`] to
///    capture the current transcript state.
/// 2. After the operation succeeds, call [`clear`] to discard stale
///    snapshots, or leave them for potential rollback.
/// 3. On failure, call [`rollback`] to restore the most recent
///    snapshot.
pub struct RuntimeSnapshotManager {
    snapshots: VecDeque<Snapshot>,
}

impl RuntimeSnapshotManager {
    /// Create an empty snapshot manager.
    pub fn new() -> Self {
        Self {
            snapshots: VecDeque::new(),
        }
    }

    /// Create a snapshot of the current messages.
    ///
    /// If the operation does not require a snapshot ([`Append`]), this
    /// is a no-op. Otherwise the snapshot is pushed onto the queue; if
    /// the queue exceeds [`MAX_SNAPSHOTS`], the oldest entry is evicted.
    ///
    /// [`Append`]: TranscriptOp::Append
    pub fn create_snapshot(&mut self, messages: &[SessionMessage], op: TranscriptOp) -> bool {
        if !op.requires_snapshot() {
            return false;
        }
        let snapshot = Snapshot {
            messages: messages.to_vec(),
            op,
            created_at: Utc::now(),
        };
        if self.snapshots.len() >= MAX_SNAPSHOTS {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
        true
    }

    /// Restore messages from the most recent snapshot.
    ///
    /// Returns `Some(messages)` if a snapshot existed and was restored;
    /// `None` if the queue is empty (no-op).
    pub fn rollback(&mut self) -> Option<Vec<SessionMessage>> {
        self.snapshots.pop_back().map(|snapshot| snapshot.messages)
    }

    /// Returns the number of snapshots currently held.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
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
