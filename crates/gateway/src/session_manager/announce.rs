//! Announce pipeline: child → parent push-based completion notification.
//!
//! When a run-mode child session completes, the gateway calls
//! `SessionManager::try_push_announce` to read the child's final assistant
//! message, build an `AnnounceEvent`, and push it onto the parent session's
//! `announce_queue`. The parent session drains the queue at the start of
//! its next turn and injects each event as a `role="system"` message.
//!
//! Step 1.2 introduces the `AnnounceEvent` type and queue storage on
//! `ConversationSession` (see `closeclaw_llm::session`). This module adds
//! the `SessionManager`-level accessors (`push_announce`, `drain_announces`)
//! in Step 1.3 and `try_push_announce` in Step 1.4.

use super::spawn::SpawnMode;
use super::SessionManager;
use crate::session_manager::communication::CommunicationError;
use chrono::Utc;
use closeclaw_llm::session::{AnnounceEvent, ChatSession, ConversationSession};
use tracing::warn;

// ── Queue accessors (push / drain) ─────────────────────────────────────────

impl SessionManager {
    /// Push an announce event onto the parent session's in-memory queue.
    ///
    /// Called by `try_push_announce` (Step 1.4) after a run-mode child
    /// session completes. The parent session drains the queue at the
    /// start of its next turn via `drain_announces`.
    ///
    /// If the parent session does not exist, this logs a warning and
    /// returns an error — the caller (typically `clear_busy_and_send` in
    /// the gateway) should not block on announce delivery, but a missing
    /// parent is a programming error worth surfacing.
    pub async fn push_announce(
        &self,
        parent_session_id: &str,
        event: AnnounceEvent,
    ) -> Result<(), String> {
        let cs = self
            .get_conversation_session(parent_session_id)
            .await
            .ok_or_else(|| {
                format!(
                    "push_announce: parent session not found: {}",
                    parent_session_id
                )
            })?;
        let mut cs = cs.write().await;
        cs.push_announce_to_queue(event);
        Ok(())
    }

    /// Drain all queued announce events for a session.
    ///
    /// Called by `drain_pending_loop` (Step 1.5) at the start of each
    /// turn. If the session does not exist, returns an empty `Vec` so
    /// callers can treat "no session" and "empty queue" identically.
    pub async fn drain_announces(&self, session_id: &str) -> Vec<AnnounceEvent> {
        let Some(cs) = self.get_conversation_session(session_id).await else {
            warn!(
                session_id = %session_id,
                "drain_announces: session not found, returning empty list"
            );
            return Vec::new();
        };
        let mut cs = cs.write().await;
        cs.drain_announce_queue()
    }

    /// Drain all queued announce events and inject each one as a
    /// `role="system"` message at the head of the parent's next turn.
    ///
    /// Called by `SessionMessageHandler::drain_pending_loop` (Step 1.5)
    /// before processing user-pending messages. Loops until the queue is
    /// empty so bursts of child completions accumulated during LLM
    /// calls are all consumed in a single call. If the session is
    /// missing mid-drain, logs a warning and returns — the next turn
    /// will retry from an empty queue.
    pub async fn drain_and_inject_announces(&self, session_id: &str) {
        loop {
            let events = self.drain_announces(session_id).await;
            if events.is_empty() {
                break;
            }
            let Some(cs) = self.get_conversation_session(session_id).await else {
                warn!(
                    session_id = %session_id,
                    "drain_and_inject_announces: session missing mid-drain"
                );
                return;
            };
            let mut cs_write = cs.write().await;
            for event in events {
                let text = format!(
                    "[子 agent {}] 任务已完成：\n{}",
                    event.child_agent_id, event.result_text
                );
                cs_write.inject_system_message(text);
            }
        }
    }
}

// ── try_push_announce + private helpers ─────────────────────────────────────

impl SessionManager {
    /// Try to push an announce event from a completed child session to
    /// its parent's in-memory queue.
    ///
    /// Called by `SessionMessageHandler::clear_busy_and_send` (Step 1.5)
    /// after a child session finishes `append_response`. Behavior:
    ///
    /// 1. Walk the `children` table to find the entry where
    ///    `child.session_id == child_session_id` and `mode == Run`.
    ///    A non-child session (not present in the table) or a child
    ///    with `mode == Session` is a no-op — no announce is produced.
    /// 2. Read the child's last `role="assistant"` message and
    ///    concatenate its `ContentBlock::Text` blocks into
    ///    `result_text`. `ContentBlock::Thinking` blocks are
    ///    intentionally excluded.
    /// 3. Drop the child read lock before acquiring the parent write
    ///    lock to avoid holding two session locks at once (which could
    ///    deadlock if another task is doing the reverse).
    /// 4. Push a fresh `AnnounceEvent` onto the parent session's
    ///    `announce_queue` via `push_announce_to_queue`.
    ///
    /// Errors are logged but not propagated — announce delivery is a
    /// best-effort signal and must not block child completion.
    pub async fn try_push_announce(&self, child_session_id: &str) {
        let Some((parent_session_id, child_agent_id)) =
            self.find_run_mode_parent(child_session_id).await
        else {
            // Not a child, or mode != Run: nothing to do.
            return;
        };

        // Step 1.3: Check communication permissions before pushing announce.
        // Child is the source (sending completion to parent), parent is the
        // target (receiving from child).
        if let Err(e) = self
            .check_session_communication(child_session_id, &parent_session_id)
            .await
        {
            match &e {
                CommunicationError::Denied { reason } => {
                    warn!(
                        child_session_id = %child_session_id,
                        parent_session_id = %parent_session_id,
                        reason = %reason,
                        "try_push_announce: communication check denied"
                    );
                }
                CommunicationError::SessionNotFound(s) => {
                    warn!(
                        session = %s,
                        "try_push_announce: session not found during communication check"
                    );
                }
                CommunicationError::NoCommunicationConfig(s) => {
                    warn!(
                        session = %s,
                        "try_push_announce: session missing communication config"
                    );
                }
            }
            return;
        }

        let Some(result_text) = self.extract_last_assistant_text(child_session_id).await else {
            warn!(
                child_session_id = %child_session_id,
                "try_push_announce: no assistant message on child, skipping"
            );
            return;
        };

        let event = build_announce_event(child_session_id, child_agent_id, result_text);
        if let Err(e) = self.push_announce(&parent_session_id, event).await {
            warn!(
                parent_session_id = %parent_session_id,
                error = %e,
                "try_push_announce: push_announce failed"
            );
        }

        // ── Decrement busy count for drain tracking ────────────────────
        // The child session result has been injected into the parent;
        // decrement the parent's busy count that was incremented in
        // `create_child_session`.
        if let Some(sh) = self.get_shutdown_handle().await {
            sh.decrement_busy();
        }

        // Unregister child handle from parent's ConversationSession.
        // This cleans up the Weak reference so the parent's child_handles
        // map does not accumulate stale entries for completed children.
        if let Some(parent_cs) = self.get_conversation_session(&parent_session_id).await {
            parent_cs
                .read()
                .await
                .unregister_child_handle(child_session_id);
        }
    }

    /// Find the (parent_session_id, child_agent_id) of a child whose
    /// `mode == Run` in the children table. Returns `None` for
    /// non-children or session-mode children.
    ///
    /// The children-table lock is acquired and dropped within this
    /// helper — we never hold it while touching any session lock.
    async fn find_run_mode_parent(&self, child_session_id: &str) -> Option<(String, String)> {
        let children = self.children.read().await;
        children
            .find_child(child_session_id)
            .filter(|i| i.mode == SpawnMode::Run)
            .map(|info| (info.parent_session_id.clone(), info.agent_id.clone()))
    }

    /// Extract the concatenated `Text` blocks from the child's last
    /// `role="assistant"` message. Returns `None` if the child has no
    /// `ConversationSession` or no assistant message.
    ///
    /// The session read lock is acquired and dropped within this
    /// helper — callers must not already hold it.
    async fn extract_last_assistant_text(&self, child_session_id: &str) -> Option<String> {
        let child_cs = self
            .get_conversation_session(child_session_id)
            .await
            .or_else(|| {
                warn!(
                    child_session_id = %child_session_id,
                    "try_push_announce: child ConversationSession missing, skipping"
                );
                None
            })?;
        let child_cs = child_cs.read().await;
        ConversationSession::collect_last_assistant_text(child_cs.messages())
    }
}

// ── Free helpers ────────────────────────────────────────────────────────────

/// Build a fresh `AnnounceEvent` with the current UTC timestamp.
fn build_announce_event(
    child_session_id: &str,
    child_agent_id: String,
    result_text: String,
) -> AnnounceEvent {
    AnnounceEvent {
        child_session_id: child_session_id.to_string(),
        child_agent_id,
        result_text,
        completed_at: Utc::now(),
    }
}
