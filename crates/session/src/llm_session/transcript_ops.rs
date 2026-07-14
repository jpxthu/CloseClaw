//! Unified transcript modification entry points.
//!
//! All code paths that mutate the conversation transcript must go
//! through the methods in this module so that snapshots are created
//! consistently and the operation type is declared explicitly.

use super::{ConversationSession, SessionMessage};
use crate::run_health::{RollbackAction, RuntimeSnapshotManager, TranscriptOp};
use closeclaw_common::ContentBlock;

/// Transcript modification methods for [`ConversationSession`].
impl ConversationSession {
    /// Unified transcript modification entry point for rewrite/partial-rewrite.
    ///
    /// Automatically creates a snapshot before replacing messages.
    /// Callers must declare their operation type via [`TranscriptOp`].
    pub fn apply_transcript_op(&mut self, op: TranscriptOp, new_messages: Vec<SessionMessage>) {
        if op.requires_snapshot() {
            let mgr = self
                .snapshot_manager
                .get_or_insert_with(RuntimeSnapshotManager::new);
            mgr.create_snapshot(&self.messages, op, "transcript-op");
        }
        self.messages = new_messages;
        self.last_activity_at = chrono::Utc::now().timestamp();
    }

    /// Incremental transcript append. Does not create a snapshot.
    pub fn append_transcript(&mut self, role: &str, content_blocks: Vec<ContentBlock>) {
        self.push_message(role, content_blocks);
    }

    /// Incremental transcript append with an incremental snapshot.
    ///
    /// Creates a [`SnapshotKind::Incremental`] snapshot recording the
    /// given `leaf_entry_id` *before* appending. This enables
    /// JSONL-truncation rollback for the appended entry.
    ///
    /// Returns `true` if a snapshot was created.
    pub fn append_transcript_with_snapshot(
        &mut self,
        role: &str,
        content_blocks: Vec<ContentBlock>,
        leaf_entry_id: &str,
    ) -> Option<String> {
        let mgr = self
            .snapshot_manager
            .get_or_insert_with(RuntimeSnapshotManager::new);
        let snapshot_id = mgr.create_incremental_snapshot(
            &self.messages,
            &format!("pre-append:{role}"),
            leaf_entry_id,
        );
        self.push_message(role, content_blocks);
        snapshot_id
    }

    /// Rollback to the most recent snapshot, if any.
    ///
    /// Returns `Some(RollbackAction)` if a snapshot was restored;
    /// `None` if no snapshot existed. A pre-rollback snapshot of the
    /// current messages is automatically created so the rollback is
    /// undoable.
    pub fn rollback_transcript(&mut self) -> Option<RollbackAction> {
        let mgr: &mut RuntimeSnapshotManager = self.snapshot_manager.as_mut()?;
        let action = mgr.rollback(&self.messages)?;
        match &action {
            RollbackAction::Replace { messages } => {
                self.messages = messages.clone();
                self.last_activity_at = chrono::Utc::now().timestamp();
            }
            RollbackAction::Truncate { .. } => {
                // Caller is responsible for truncating the JSONL file
                // based on the leaf_entry_id. We just return the action.
            }
        }
        Some(action)
    }

    /// Returns the number of snapshots held, or `None` if no
    /// snapshot manager exists.
    pub fn snapshot_count(&self) -> Option<usize> {
        self.snapshot_manager
            .as_ref()
            .map(|m: &RuntimeSnapshotManager| m.snapshot_count())
    }

    /// Clear all snapshots without restoring.
    pub fn clear_snapshots(&mut self) {
        if let Some(mgr) = self.snapshot_manager.as_mut() {
            RuntimeSnapshotManager::clear(mgr);
        }
    }

    /// Create a snapshot of the current transcript state without
    /// replacing messages. Used by pre-compaction and pre-rewrite
    /// paths that need a backup before a separate operation modifies
    /// the transcript.
    ///
    /// Returns the snapshot id if a snapshot was created, `None`
    /// otherwise (e.g. for [`Append`] operations).
    pub fn snapshot_current_state(&mut self, op: TranscriptOp, reason: &str) -> Option<String> {
        if op.requires_snapshot() {
            let mgr = self
                .snapshot_manager
                .get_or_insert_with(RuntimeSnapshotManager::new);
            mgr.create_snapshot(&self.messages, op, reason)
        } else {
            None
        }
    }

    /// Mark a snapshot as [`SnapshotStatus::Complete`].
    ///
    /// Used after a guarded operation (compaction, `/system` rewrite)
    /// succeeds to transition the snapshot from `Pending` to `Complete`.
    pub fn mark_complete_snapshot(&mut self, snapshot_id: &str) {
        if let Some(mgr) = self.snapshot_manager.as_mut() {
            mgr.mark_complete(snapshot_id);
        }
    }

    /// Extract tool-call info from the last `n` messages.
    ///
    /// Scans `self.messages` in reverse, collecting
    /// [`ToolUse`][ContentBlock::ToolUse] content blocks from each
    /// message until `n` messages have been visited.  Returns the
    /// result in chronological order (oldest first).  Tool results
    /// and thinking blocks are skipped.
    pub fn recent_tool_calls(&self, n: usize) -> Vec<crate::run_health::HookToolCallInfo> {
        let mut per_message: Vec<Vec<crate::run_health::HookToolCallInfo>> = Vec::new();
        for msg in self.messages.iter().rev().take(n) {
            let mut calls = Vec::new();
            for block in &msg.content_blocks {
                if let ContentBlock::ToolUse { name, input, .. } = block {
                    calls.push(crate::run_health::HookToolCallInfo {
                        name: name.clone(),
                        input: input.clone(),
                    });
                }
            }
            per_message.push(calls);
        }
        per_message.reverse();
        per_message.into_iter().flatten().collect()
    }
}
