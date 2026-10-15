//! Announce pipeline: child → parent push-based completion notification
//! (push / notify / recovery side; the queue drain side lives in
//! `announce_drain.rs`).

use super::spawn::SpawnMode;
use super::SessionManager;
use crate::outbound::SendOutboundIds;
use crate::session_handler_cache_check::check_cache_anomaly_and_accumulate_usage;
use crate::session_manager::communication::CommunicationError;
use chrono::Utc;
use closeclaw_common::NotificationPriority;
use closeclaw_common::{ChildCompletionStatus, ChildSessionState};
use closeclaw_session::llm_session::{AnnounceEvent, ChatSession, ConversationSession};
use closeclaw_session::spawn::types::ChildSessionStatus;
use tracing::{debug, warn};

// ── try_push_announce + private helpers ─────────────────────────────────────

impl SessionManager {
    /// Push announce from completed child to parent's queue.
    pub async fn try_push_announce(&self, child_session_id: &str, priority: NotificationPriority) {
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

        // Dedup protection: skip announce if child state is already terminal.
        // This prevents duplicate AnnounceEvent injection when
        // AnnounceSweeper and clear_busy_and_send race on the same child.
        // Style matches notify_child_forced_termination / notify_child_error.
        if let Some(parent_cs) = self.get_conversation_session(&parent_session_id).await {
            let parent_guard = parent_cs.read().await;
            let states = parent_guard
                .child_states
                .read()
                .expect("child_states lock poisoned");
            if let Some((state, _)) = states.get(child_session_id) {
                if matches!(
                    state,
                    ChildSessionState::Completed
                        | ChildSessionState::Errored
                        | ChildSessionState::Terminated
                ) {
                    debug!(
                        child_session_id = %child_session_id,
                        ?state,
                        "try_push_announce: child already terminal, skipping"
                    );
                    return;
                }
            }
        }

        let Some(result_text) = self.extract_last_assistant_text(child_session_id).await else {
            warn!(
                child_session_id = %child_session_id,
                "try_push_announce: no assistant message on child, skipping"
            );
            return;
        };

        let child_status = self
            .resolve_child_completion_status(&parent_session_id, child_session_id)
            .await;

        // Set child state to Completed before pushing announce.
        // Push is async and other concurrent try_push_announce calls may
        // interleave during the push. The dedup guard above checks this
        // state before we reach here, so setting it here ensures any
        // racing call will see Completed and skip (dedup protection).
        if let Some(parent_cs) = self.get_conversation_session(&parent_session_id).await {
            let parent_guard = parent_cs.read().await;
            parent_guard.update_child_state(child_session_id, ChildSessionState::Completed);
        }

        let event = build_announce_event(
            child_session_id,
            child_agent_id,
            result_text,
            priority,
            child_status,
        );
        // Step 1.2: On push success, reclaim node from SpawnTree immediately
        // (design doc §节点回收: 入队成功后立即回收). On failure, mark as
        // Completed so AnnounceSweeper can pick it up later (完成待回收).
        let push_ok = self.push_announce(&parent_session_id, event).await;
        {
            let mut children = self.children.write().await;
            if let Ok(()) = push_ok {
                // Push succeeded — remove child from tree (no long-term memory hold).
                children.remove_child(&parent_session_id, child_session_id);
                debug!(
                    child_session_id = %child_session_id,
                    parent_session_id = %parent_session_id,
                    "try_push_announce: child reclaimed from SpawnTree"
                );
            } else {
                // Push failed — mark Completed for sweeper to reclaim later.
                // SAFETY: we are in the else branch of if let Ok, so push_ok is Err.
                let e = push_ok.expect_err("push_ok is Err in else branch");
                warn!(
                    parent_session_id = %parent_session_id,
                    error = %e,
                    "try_push_announce: push_announce failed"
                );
                if !children.mark_child_status(child_session_id, ChildSessionStatus::Completed) {
                    warn!(
                        child_session_id = %child_session_id,
                        "try_push_announce: child not found in SpawnTree for status update"
                    );
                }
            }
        }

        // ── Notify DreamingScheduler for immediate mining (design doc §触发 1).
        // Only run-mode sub-agent sessions trigger this; owner sessions
        // still go through the ArchiveSweeper idle→archive path.
        if let Some(tx) = self.mining_notify_tx.read().unwrap().as_ref() {
            if let Err(e) = tx.try_send(child_session_id.to_string()) {
                warn!(
                    child_session_id = %child_session_id,
                    %e,
                    "try_push_announce: mining notification failed"
                );
            }
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

        // Step 1.6: Auto-recovery — check if parent session should
        // exit Waiting state after all run-mode children complete.
        self.maybe_recover_yielded_session(&parent_session_id).await;
    }

    /// Find run-mode parent for a child session.
    async fn find_run_mode_parent(&self, child_session_id: &str) -> Option<(String, String)> {
        let children = self.children.read().await;
        children
            .find_child(child_session_id)
            .filter(|i| i.mode == SpawnMode::Run)
            .map(|info| (info.parent_session_id.clone(), info.agent_id.clone()))
    }

    /// Extract text from child's last assistant message.
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

    /// Check if any run-mode children are still running.
    async fn has_running_run_mode_children(&self, parent_id: &str) -> bool {
        let children = self.children.read().await;
        children
            .list_children(parent_id)
            .iter()
            .any(|info| info.mode == SpawnMode::Run)
    }

    /// Resolve child completion status from parent's child_states map.
    async fn resolve_child_completion_status(
        &self,
        parent_session_id: &str,
        child_session_id: &str,
    ) -> ChildCompletionStatus {
        let Some(parent_cs) = self.get_conversation_session(parent_session_id).await else {
            warn!(
                parent_session_id = %parent_session_id,
                "resolve_child_completion_status: parent session not found, defaulting to Completed"
            );
            return ChildCompletionStatus::Completed;
        };
        let cs = parent_cs.read().await;
        let state = cs
            .child_states
            .read()
            .expect("child_states lock poisoned")
            .get(child_session_id)
            .map(|(s, _)| *s);
        match state {
            Some(ChildSessionState::Completed) => ChildCompletionStatus::Completed,
            Some(ChildSessionState::Errored) => ChildCompletionStatus::Errored,
            Some(ChildSessionState::Terminated) => ChildCompletionStatus::Terminated,
            Some(ChildSessionState::Running) | None => {
                // Running or missing state at announce time: treat as
                // Completed since the child has finished its last assistant
                // turn (which is why try_push_announce was called).
                ChildCompletionStatus::Completed
            }
        }
    }

    /// Auto-recovery: exit Waiting and drain pending messages when
    /// all run-mode children have completed.
    async fn maybe_recover_yielded_session(&self, parent_id: &str) {
        // Only recover if the session is actively yielding.
        if !self.is_session_yielding(parent_id).await {
            return;
        }
        // Check if there are still running run-mode children.
        if self.has_running_run_mode_children(parent_id).await {
            return;
        }
        tracing::info!(
            parent_id = %parent_id,
            "maybe_recover_yielded_session: all run-mode children done, recovering"
        );
        // Cancel the yield timeout (normal recovery path).
        self.cancel_yield_timeout(parent_id).await;
        // Exit Waiting state.
        if let Some(cs) = self.get_conversation_session(parent_id).await {
            cs.read().await.exit_waiting();
        }
        // Trigger the pending-message drain loop to process queued
        // announces and user messages.
        self.drain_pending_for_session(parent_id).await;
    }

    /// Notify parent about a forcefully terminated child session.
    pub(crate) async fn notify_child_forced_termination(&self, session_id: &str) {
        let Some(info) = self.find_child_info(session_id).await else {
            // Not a child in the tree — nothing to do.
            return;
        };
        if info.mode != SpawnMode::Run {
            // Only run-mode children produce announce notifications.
            return;
        }
        let parent_session_id = &info.parent_session_id;
        let child_agent_id = &info.agent_id;

        // Check if the parent session exists.
        let Some(parent_cs) = self.get_conversation_session(parent_session_id).await else {
            warn!(
                parent_session_id = %parent_session_id,
                "notify_child_forced_termination: parent session not found"
            );
            return;
        };

        // Dedup protection: skip if child state is already terminal.
        {
            let parent_guard = parent_cs.read().await;
            let states = parent_guard
                .child_states
                .read()
                .expect("child_states lock poisoned");
            if let Some((state, _)) = states.get(session_id) {
                if matches!(
                    state,
                    ChildSessionState::Completed
                        | ChildSessionState::Errored
                        | ChildSessionState::Terminated
                ) {
                    tracing::debug!(
                        session_id = %session_id,
                        ?state,
                        "notify_child_forced_termination: child already terminal, skipping"
                    );
                    return;
                }
            }
        }

        // Set child state to Terminated in parent's child_states.
        {
            let parent_guard = parent_cs.read().await;
            parent_guard.update_child_state(session_id, ChildSessionState::Terminated);
        }

        // Build announce event with Terminated status.
        let event = build_announce_event(
            session_id,
            child_agent_id.clone(),
            "任务被终止".to_string(),
            NotificationPriority::Next,
            ChildCompletionStatus::Terminated,
        );

        if let Err(e) = self.push_announce(parent_session_id, event).await {
            warn!(
                parent_session_id = %parent_session_id,
                error = %e,
                "notify_child_forced_termination: push_announce failed"
            );
        }

        // Decrement busy count for drain tracking.
        if let Some(sh) = self.get_shutdown_handle().await {
            sh.decrement_busy();
        }

        // Unregister child handle from parent's ConversationSession.
        if let Some(parent_cs) = self.get_conversation_session(parent_session_id).await {
            parent_cs.read().await.unregister_child_handle(session_id);
        }
    }

    /// Notify parent about a child session that errored.
    pub(crate) async fn notify_child_error(&self, session_id: &str) {
        let Some(info) = self.find_child_info(session_id).await else {
            return;
        };
        if info.mode != SpawnMode::Run {
            return;
        }
        let parent_session_id = &info.parent_session_id;

        let Some(parent_cs) = self.get_conversation_session(parent_session_id).await else {
            warn!(
                parent_session_id = %parent_session_id,
                "notify_child_error: parent session not found"
            );
            return;
        };

        // Dedup protection: skip if child state is already terminal.
        {
            let parent_guard = parent_cs.read().await;
            let states = parent_guard
                .child_states
                .read()
                .expect("child_states lock poisoned");
            if let Some((state, _)) = states.get(session_id) {
                if matches!(
                    state,
                    ChildSessionState::Completed
                        | ChildSessionState::Errored
                        | ChildSessionState::Terminated
                ) {
                    tracing::debug!(
                        session_id = %session_id,
                        ?state,
                        "notify_child_error: child already terminal, skipping"
                    );
                    return;
                }
            }
        }

        // Set child state to Errored in parent's child_states.
        {
            let parent_guard = parent_cs.read().await;
            parent_guard.update_child_state(session_id, ChildSessionState::Errored);
        }
    }

    /// Find child info in spawn tree by session ID.
    async fn find_child_info(&self, session_id: &str) -> Option<super::spawn::ChildSessionInfo> {
        let children = self.children.read().await;
        children.find_child(session_id).cloned()
    }

    /// Trigger recovery check for a yielded session.
    ///
    /// Public (within crate) wrapper around `maybe_recover_yielded_session`
    /// so tests can directly trigger the recovery path without relying
    /// on `try_push_announce` (which requires the child to be in the tree).
    #[allow(dead_code)] // used by tests in yield_recovery_tests
    pub(crate) async fn trigger_yield_recovery(&self, parent_id: &str) {
        self.maybe_recover_yielded_session(parent_id).await;
    }

    /// Drain pending messages for a session after recovery from Waiting.
    ///
    /// Mirrors the drain loop in `SessionMessageHandler::drain_pending_loop`
    /// but runs directly on SessionManager: queued announce events first,
    /// then each pending user message via the shared
    /// `invoke_llm_with_call_target` + unified cache-check advancement
    /// (check + accumulate), exactly like the main dispatch path.
    pub(crate) async fn drain_pending_for_session(&self, session_id: &str) {
        let gw = self.get_gateway_ref().await;
        self.drain_and_inject_announces(session_id, gw.as_ref())
            .await;

        // Resolve channel from session for outbound dispatch.
        let channel = {
            let sessions = self.sessions.read().await;
            sessions.get(session_id).map(|s| s.channel.clone())
        };

        // Then process queued pending messages (user messages).
        loop {
            let Some(pending) = self.pop_pending_message(session_id).await else {
                break;
            };
            // Set busy and dispatch LLM call for each queued message.
            if let Some(cs) = self.get_conversation_session(session_id).await {
                {
                    let cs_write = cs.write().await;
                    cs_write.set_llm_busy(true);
                    cs_write.set_llm_state(closeclaw_llm::session_state::LlmState::Requesting);
                }
                // Invoke LLM with pre-call reasoning resolution.
                cs.read()
                    .await
                    .set_request_context(closeclaw_common::RequestContext::default());
                crate::session_handler_reasoning::resolve_before_llm_call(self, session_id).await;
                let result = crate::session_handler_dispatch::invoke_llm_with_call_target(
                    &cs,
                    &pending.content,
                    false,
                )
                .await;
                // Clear busy state.
                {
                    let cs_write = cs.write().await;
                    cs_write.set_llm_busy(false);
                    cs_write.set_llm_state(closeclaw_llm::session_state::LlmState::Idle);
                }
                // Append response to session history, run the unified
                // cache-check advancement + usage accumulation, and send
                // to user. Recovery rounds must check + accumulate exactly
                // like the main path so the next round's reference value
                // reflects the recovery round (doc :65/:73).
                if let Ok(stream_result) = result {
                    let text = stream_result
                        .content_blocks
                        .iter()
                        .filter_map(|b| match b {
                            closeclaw_llm::types::ContentBlock::Text(t) => Some(t.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    {
                        let mut cs_write = cs.write().await;
                        cs_write.append_response(stream_result.clone().into());
                        check_cache_anomaly_and_accumulate_usage(
                            session_id,
                            &mut cs_write,
                            &stream_result,
                            // No metrics emitter is plumbed into the
                            // SessionManager recovery path; the warn log
                            // and Owner notification still run.
                            &None,
                        );
                    }
                    // Send response to user via Gateway outbound pipeline.
                    if let (Some(ref gw), Some(ref ch)) = (&gw, &channel) {
                        // Explicit SendOutboundIds::default() — matches the
                        // style of all other send_outbound call sites.
                        let ids = SendOutboundIds::default();
                        if let Err(e) = gw.send_outbound(session_id, ch, &text, vec![], ids).await {
                            warn!(
                                session_id = %session_id,
                                error = %e,
                                "drain_pending_for_session: failed to send response to user"
                            );
                        }
                    }
                }
            }
        }
    }
}

// ── Free helpers ────────────────────────────────────────────────────────────

/// Build a fresh `AnnounceEvent` with the current UTC timestamp.
pub(crate) fn build_announce_event(
    child_session_id: &str,
    child_agent_id: String,
    result_text: String,
    priority: NotificationPriority,
    status: ChildCompletionStatus,
) -> AnnounceEvent {
    AnnounceEvent {
        child_session_id: child_session_id.to_string(),
        child_agent_id,
        result_text,
        completed_at: Utc::now(),
        priority,
        status,
    }
}
