//! Graceful stop helpers for [`SessionManager`].
//!
//! Extracted from `stop.rs` to keep each `impl` block within the
//! 100-line limit required by CONTRIBUTING.md.

use std::sync::Arc;
use std::time::Duration;

use closeclaw_llm::session_state::LlmState;
use closeclaw_session::persistence::{
    PendingOperation, PersistenceError, SessionCheckpoint, SessionStatus,
};

use super::stop::{GracefulTimeoutInfo, StopError, StopSingleResult};
use super::SessionManager;

// ── graceful timeout logic ─────────────────────────────────────────────

impl SessionManager {
    /// Graceful wait with configurable timeout.
    /// Returns pending ops and, on timeout, [`GracefulTimeoutInfo`].
    pub(super) async fn graceful_wait_with_timeout(
        cs: &Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
        session_id: &str,
        timeout: Duration,
    ) -> (Vec<PendingOperation>, Option<GracefulTimeoutInfo>) {
        let start = tokio::time::Instant::now();
        let mut pending_ops = Vec::new();
        let mut streaming_seen = false;

        let result = tokio::time::timeout(timeout, async {
            loop {
                let (is_streaming, has_running_tools) = Self::check_session_active_state(cs).await;
                let should_break = Self::eval_graceful_iteration(
                    cs,
                    &mut pending_ops,
                    &mut streaming_seen,
                    is_streaming,
                    has_running_tools,
                );
                if should_break {
                    break;
                }
                tracing::debug!(
                    session_id = %session_id,
                    streaming = is_streaming,
                    running_tools = has_running_tools,
                    "graceful stop: waiting for completion"
                );
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        Self::handle_graceful_result(result, pending_ops, cs, session_id, start).await
    }

    /// Check whether the session is actively streaming or running tools.
    async fn check_session_active_state(
        cs: &Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
    ) -> (bool, bool) {
        let guard = &*cs.read().await;
        let state = *guard.llm_state.read().expect("llm_state lock poisoned");
        let tool_states = guard.tool_states.read().expect("tool_states lock poisoned");
        let streaming = matches!(state, LlmState::Receiving | LlmState::Requesting);
        let tools = tool_states.values().any(|s| {
            matches!(
                s,
                closeclaw_llm::session_state::ToolExecState::RunningForeground
                    | closeclaw_llm::session_state::ToolExecState::RunningBackground
            )
        });
        (streaming, tools)
    }

    /// Evaluate one graceful-loop iteration; returns true to break.
    fn eval_graceful_iteration(
        cs: &Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
        pending_ops: &mut Vec<PendingOperation>,
        streaming_seen: &mut bool,
        is_streaming: bool,
        has_running_tools: bool,
    ) -> bool {
        if is_streaming {
            *streaming_seen = true;
            return false;
        }
        if *streaming_seen {
            let ops = {
                (*cs)
                    .try_read()
                    .map(|g| g.extract_pending_tool_calls())
                    .unwrap_or_default()
            };
            if !ops.is_empty() {
                *pending_ops = ops;
                return true;
            }
            if !has_running_tools {
                return true;
            }
            return false;
        }
        !has_running_tools
    }

    /// Map timeout result to return type.
    async fn handle_graceful_result(
        result: Result<(), tokio::time::error::Elapsed>,
        pending_ops: Vec<PendingOperation>,
        cs: &Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
        session_id: &str,
        start: tokio::time::Instant,
    ) -> (Vec<PendingOperation>, Option<GracefulTimeoutInfo>) {
        match result {
            Ok(()) => (pending_ops, None),
            Err(_elapsed) => {
                let waiting_items = Self::collect_waiting_items(cs).await;
                (
                    pending_ops,
                    Some(GracefulTimeoutInfo {
                        session_id: session_id.to_string(),
                        waiting_items,
                        elapsed: start.elapsed(),
                    }),
                )
            }
        }
    }
}

// ── waiting items collection ───────────────────────────────────────────

impl SessionManager {
    /// Operations still in progress (for timeout reporting).
    pub(super) async fn collect_waiting_items(
        cs: &Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
    ) -> Vec<String> {
        let guard = cs.read().await;
        let mut items = Vec::new();
        let state = *guard.llm_state.read().expect("lock poisoned");
        if matches!(state, LlmState::Receiving | LlmState::Requesting) {
            items.push("LLM streaming".to_string());
        }
        for (id, s) in guard.tool_states.read().expect("lock poisoned").iter() {
            if matches!(
                s,
                closeclaw_llm::session_state::ToolExecState::RunningForeground
                    | closeclaw_llm::session_state::ToolExecState::RunningBackground
            ) {
                items.push(format!("tool {} running", id));
            }
        }
        for (id, s) in guard.child_states.read().expect("lock poisoned").iter() {
            if matches!(s, closeclaw_common::ChildSessionState::Running) {
                items.push(format!("child session {} running", id));
            }
        }
        items
    }
}

// ── session finalization ───────────────────────────────────────────────

impl SessionManager {
    /// Finalize a stopped session: stop, clean up, persist.
    ///
    /// When `cascade` is true the session's own stop cascades to
    /// children (used by `/stop --cascade`).
    pub(super) async fn finalize_session_stop(
        &self,
        cs: Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
        session_id: &str,
        pending_ops: Vec<PendingOperation>,
        cascade: bool,
    ) -> Result<StopSingleResult, StopError> {
        cs.read().await.stop(cascade).await;
        self.cleanup_and_persist(session_id, pending_ops).await?;
        Ok(StopSingleResult {
            _completed: true,
            graceful_timeout: None,
        })
    }

    /// Cleanup task manager and persist checkpoint.
    async fn cleanup_and_persist(
        &self,
        session_id: &str,
        pending_ops: Vec<PendingOperation>,
    ) -> Result<(), StopError> {
        if let Some(tm) = self.get_task_manager().await {
            tm.cleanup_finished().await;
        }
        if let Err(e) = self
            .persist_checkpoint_with_pending(session_id, pending_ops)
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "stop_all_sessions: checkpoint persist failed"
            );
            return Err(StopError::Failed);
        }
        Ok(())
    }
}

// ── checkpoint persistence ─────────────────────────────────────────────

impl SessionManager {
    /// Persist a session checkpoint with optional pending operations.
    /// Non-empty `pending_ops` (forceful shutdown) are recorded for recovery.
    async fn persist_checkpoint_with_pending(
        &self,
        session_id: &str,
        pending_ops: Vec<PendingOperation>,
    ) -> Result<(), PersistenceError> {
        let storage = {
            let guard = self.storage.read().await;
            match guard.as_ref() {
                Some(s) => std::sync::Arc::clone(s),
                None => return Ok(()),
            }
        };

        let (agent_id, channel) = {
            let sessions = self.sessions.read().await;
            match sessions.get(session_id) {
                Some(s) => (Some(s.agent_id.clone()), Some(s.channel.clone())),
                None => (None, None),
            }
        };

        let pending = {
            let conv = self.conversation_sessions.read().await;
            match conv.get(session_id) {
                Some(cs) => {
                    let guard = cs.read().await;
                    guard.get_pending_messages()
                }
                None => Vec::new(),
            }
        };

        let mut cp = match storage.load_checkpoint(session_id).await? {
            Some(mut cp) => {
                cp.status = SessionStatus::Active;
                if let Some(ch) = channel {
                    cp.platform = Some(ch);
                }
                if let Some(aid) = agent_id {
                    cp.agent_id = Some(aid);
                }
                cp.pending_messages = pending;
                cp
            }
            None => {
                let mut cp = SessionCheckpoint::new(session_id.to_string())
                    .with_status(SessionStatus::Active);
                if let Some(ch) = channel {
                    cp = cp.with_platform(ch);
                }
                if let Some(aid) = agent_id {
                    cp = cp.with_agent_id(aid);
                }
                cp.with_pending_messages(pending)
            }
        };

        // Sync system_appends and verbosity_level from ConversationSession.
        {
            let conv = self.conversation_sessions.read().await;
            if let Some(cs) = conv.get(session_id) {
                let guard = cs.read().await;
                cp.system_appends = guard.user_system_appends().to_vec();
                cp.verbosity_level = guard.verbosity_level();
            }
        }

        // Record pending operations from forceful shutdown.
        if !pending_ops.is_empty() {
            cp.pending_operations = pending_ops;
        }

        storage.save_checkpoint(&cp).await
    }
}
