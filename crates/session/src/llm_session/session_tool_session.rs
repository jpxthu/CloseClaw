//! `ToolSession` trait implementation for `ConversationSession`.
//!
//! This module holds the *adapter* that lets the tool layer treat a
//! `ConversationSession` as a [`ToolSession`]
//! (tool-call bookkeeping, file-mtime caches, child-session state and
//! checkpoint persistence). The inherent methods it delegates to live
//! in sibling modules: `session_exec`, `session_pending`, `workflow`,
//! `session_handles`, `session_accessors` and `mod.rs`.
//!
//! A trait impl cannot be split across multiple `impl` blocks in Rust,
//! so this file intentionally contains the single (large) trait impl.

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use closeclaw_common::canonicalize_or_clone;
use closeclaw_common::{FileReadCache, ReadRange};

use super::ConversationSession;

#[async_trait::async_trait]
impl closeclaw_common::tool_session::ToolSession for ConversationSession {
    async fn register_tool_handle(
        &self,
        call_id: String,
        handle: Arc<dyn closeclaw_common::tool_session::KillHandle>,
    ) {
        // Delegate to the inherent method which also handles
        // shutdown-gate and busy-count tracking.
        self.register_tool_handle(call_id, handle);
    }

    fn manual_background_notify(&self) -> Option<Arc<tokio::sync::Notify>> {
        Some(self.manual_background_notify())
    }

    fn enter_waiting(&self) {
        ConversationSession::enter_waiting(self);
    }

    fn exit_waiting(&self) {
        ConversationSession::exit_waiting(self);
    }

    fn is_waiting(&self) -> bool {
        ConversationSession::is_waiting(self)
    }

    async fn register_tool_call(&self, call_id: String, tool_name: String, args_summary: String) {
        ConversationSession::register_tool_call(self, call_id, tool_name, args_summary);
        if let Err(e) = self.persist_pending_checkpoint().await {
            tracing::warn!(
                session_id = %self.session_id,
                "register_tool_call: checkpoint persist failed: {}",
                e
            );
        }
    }

    async fn deregister_tool_call(&self, call_id: String) {
        ConversationSession::deregister_tool_call(self, &call_id);
        if let Err(e) = self.persist_pending_checkpoint().await {
            tracing::warn!(
                session_id = %self.session_id,
                "deregister_tool_call: checkpoint persist failed: {}",
                e
            );
        }
    }

    async fn update_tool_state(&self, call_id: &str, state: closeclaw_common::ToolExecState) {
        ConversationSession::update_tool_state(self, call_id, state);
    }

    async fn report_tool_progress(&self, call_id: &str, progress: closeclaw_common::ToolProgress) {
        tracing::debug!(
            call_id = %call_id,
            lines = progress.lines,
            bytes = progress.bytes,
            elapsed_ms = progress.elapsed.as_millis() as u64,
            "tool progress"
        );
    }

    async fn register_child_state(&self, child_id: String, agent_id: String, task_summary: String) {
        ConversationSession::register_child(self, child_id, agent_id, task_summary);
        if let Err(e) = self.persist_pending_checkpoint().await {
            tracing::warn!(
                session_id = %self.session_id,
                "register_child_state: checkpoint persist failed: {}",
                e
            );
        }
    }

    async fn deregister_child_state(&self, child_id: String) {
        ConversationSession::deregister_child(self, &child_id);
        if let Err(e) = self.persist_pending_checkpoint().await {
            tracing::warn!(
                session_id = %self.session_id,
                "deregister_child_state: checkpoint persist failed: {}",
                e
            );
        }
    }

    fn has_running_child(&self) -> bool {
        ConversationSession::has_running_child(self)
    }

    async fn record_file_read(&self, path: &str, mtime: Option<SystemTime>) {
        let canonical = canonicalize_or_clone(Path::new(path));
        let mut map = self.file_mtimes.write().expect("file_mtimes lock poisoned");
        match mtime {
            Some(t) => {
                map.insert(canonical, t);
            }
            None => {
                map.remove(&canonical);
            }
        }
    }

    fn get_file_mtime(&self, path: &str) -> Option<SystemTime> {
        let canonical = canonicalize_or_clone(Path::new(path));
        let map = self.file_mtimes.read().expect("file_mtimes lock poisoned");
        map.get(&canonical).copied()
    }

    fn get_file_read_cache(&self, path: &str) -> Option<FileReadCache> {
        let canonical = canonicalize_or_clone(Path::new(path));
        let map = self
            .file_read_ranges
            .read()
            .expect("file_read_ranges lock poisoned");
        map.get(&canonical).cloned()
    }

    async fn record_file_read_range(
        &self,
        path: &str,
        mtime: Option<SystemTime>,
        range: ReadRange,
    ) {
        let canonical = canonicalize_or_clone(Path::new(path));
        let mut map = self
            .file_read_ranges
            .write()
            .expect("file_read_ranges lock poisoned");
        let entry = map.entry(canonical).or_insert_with(|| FileReadCache {
            mtime,
            ranges: Vec::new(),
        });
        entry.mtime = mtime;
        entry.ranges.push(range);
    }

    async fn persist_pending_checkpoint(&self) -> Result<(), String> {
        let Some(ref storage) = self.checkpoint_storage else {
            return Ok(());
        };
        let session_id = self.session_id.clone();
        let collected_ops = self.collect_pending_operations();
        let system_appends = self.user_system_appends().to_vec();
        let verbosity = self.verbosity_level();
        let workflow_run = self.workflow_run_value();
        // Load existing checkpoint or create a minimal one.
        let mut cp = match storage.load_checkpoint(&session_id).await {
            Ok(Some(cp)) => cp,
            _ => crate::persistence::SessionCheckpoint::new(session_id),
        };
        // Merge pending operations: replace ToolCall/SubSessionSpawn ops
        // (collected from live session state) while preserving OutboundMessage
        // ops (managed at checkpoint layer via write-ahead / ack-clear).
        // This prevents write-ahead OutboundMessage ops from being overwritten
        // by the live-state collector which has no outbound queue.
        cp.pending_operations.retain(|op| {
            matches!(
                op.op_type,
                crate::persistence::PendingOperationType::OutboundMessage
            )
        });
        cp.pending_operations.extend(collected_ops);
        cp.user_appends = system_appends;
        cp.verbosity_level = verbosity;
        cp.workflow_run = workflow_run;
        cp.touch();
        cp.last_message_at = Some(chrono::Utc::now());
        storage.save_checkpoint(&cp).await.map_err(|e| {
            let msg = format!("persist_pending_checkpoint: save failed: {}", e);
            tracing::warn!(session_id = %cp.session_id, "{}", msg);
            msg
        })
    }
}
