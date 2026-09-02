//! Announce pipeline: child → parent push-based completion notification.

use super::spawn::SpawnMode;
use super::SessionManager;
use crate::session_manager::communication::CommunicationError;
use crate::Gateway;
use chrono::Utc;
use closeclaw_common::{ChildCompletionStatus, ChildSessionState};
use closeclaw_session::llm_session::{AnnounceEvent, ChatSession, ConversationSession, QueueEntry};
use closeclaw_session::spawn::types::ChildSessionStatus;
use closeclaw_tasks::NotificationPriority;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

// ── Priority prefix helper ────────────────────────────────────────────────

/// Map a [`NotificationPriority`] to a display prefix injected into
/// system messages so the user can distinguish urgency at a glance.
///
/// - `Now`  → `[紧急]`
/// - `Next` → `[注意]`
/// - `Later` → `[后台]`
fn priority_prefix(priority: &NotificationPriority) -> &'static str {
    match priority {
        NotificationPriority::Now => "[紧急] ",
        NotificationPriority::Next => "[注意] ",
        NotificationPriority::Later => "[后台] ",
    }
}

/// Return type for drain functions: separates announce events from
/// system notifications (routed via simplified outbound path).
#[derive(Debug)]
pub(crate) struct DrainResult {
    pub announces: Vec<AnnounceEvent>,
    pub system_notifications: Vec<String>,
}

#[cfg(test)]
impl DrainResult {
    /// Whether the announce list is empty.
    pub fn is_empty(&self) -> bool {
        self.announces.is_empty()
    }
    /// Number of announce events.
    pub fn len(&self) -> usize {
        self.announces.len()
    }
    /// Iterate over announce events.
    pub fn iter(&self) -> std::slice::Iter<'_, AnnounceEvent> {
        self.announces.iter()
    }
}

impl std::ops::Index<usize> for DrainResult {
    type Output = AnnounceEvent;
    fn index(&self, index: usize) -> &Self::Output {
        &self.announces[index]
    }
}

// ── Queue accessors (push / drain) ─────────────────────────────────────────

impl SessionManager {
    /// Push an announce event onto the parent session's in-memory queue.
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

    /// Drain all queued announce events. System notifications go to
    /// `DrainResult::system_notifications` (simplified outbound path).
    pub(crate) async fn drain_announces(&self, session_id: &str) -> DrainResult {
        let Some(cs) = self.get_conversation_session(session_id).await else {
            warn!(session_id = %session_id, "drain_announces: session not found");
            return DrainResult {
                announces: vec![],
                system_notifications: vec![],
            };
        };
        let mut cs = cs.write().await;
        let all = cs.drain_all_entries();
        let mut announces = Vec::new();
        let mut system_notifications = Vec::new();
        for entry in all {
            match entry {
                QueueEntry::Announce(e) => announces.push(e),
                QueueEntry::UserMessage(pm) => {
                    cs.push_pending(pm);
                }
                QueueEntry::BackgroundToolNotification(notif) => {
                    announces.push(notif_to_announce(notif));
                }
                QueueEntry::SystemNotification(text, _) => {
                    system_notifications.push(text);
                }
            }
        }
        DrainResult {
            announces,
            system_notifications,
        }
    }

    /// Drain and inject announce events. System notifications route
    /// via simplified outbound path; announce events inject as system messages.
    pub async fn drain_and_inject_announces(
        &self,
        session_id: &str,
        gateway: Option<&Arc<Gateway>>,
    ) {
        loop {
            let result = self.drain_announces(session_id).await;
            if result.announces.is_empty() && result.system_notifications.is_empty() {
                break;
            }
            let Some(cs) = self.get_conversation_session(session_id).await else {
                warn!(
                    session_id = %session_id,
                    "drain_and_inject_announces: session missing mid-drain"
                );
                return;
            };
            inject_announces_as_system_messages(&cs, &result.announces).await;
            self.route_system_notifications(session_id, &result.system_notifications, gateway)
                .await;
        }
    }

    /// Drain announce events matching a predicate.
    /// Non-matching events are re-inserted. System notifications go to
    /// `DrainResult::system_notifications`.
    pub(crate) async fn drain_announces_filtered(
        &self,
        session_id: &str,
        predicate: impl Fn(&NotificationPriority) -> bool,
    ) -> DrainResult {
        let Some(cs) = self.get_conversation_session(session_id).await else {
            warn!(session_id = %session_id, "drain_announces_filtered: session not found");
            return DrainResult {
                announces: vec![],
                system_notifications: vec![],
            };
        };
        let mut cs = cs.write().await;
        let all = cs.drain_all_entries();
        let mut matched_announces = Vec::new();
        let mut matched_notifications = Vec::new();
        for entry in all {
            match entry {
                QueueEntry::Announce(ref event) if predicate(&event.priority) => {
                    matched_announces.push(event.clone());
                }
                // Background tool completion → AnnounceEvent for inject path.
                QueueEntry::BackgroundToolNotification(ref notif) if predicate(&notif.priority) => {
                    matched_announces.push(notif_to_announce(notif.clone()));
                }
                QueueEntry::SystemNotification(ref text, ref priority) if predicate(priority) => {
                    matched_notifications.push(format!("{}{}", priority_prefix(priority), text));
                }
                QueueEntry::SystemNotification(text, priority) => {
                    cs.push_queue_entry(QueueEntry::SystemNotification(text, priority));
                }
                other => {
                    cs.push_queue_entry(other);
                }
            }
        }
        DrainResult {
            announces: matched_announces,
            system_notifications: matched_notifications,
        }
    }

    /// Drain and inject filtered announce events. Non-matching events
    /// stay in the queue. System notifications route via simplified outbound.
    pub async fn drain_and_inject_announces_filtered(
        &self,
        session_id: &str,
        predicate: impl Fn(&NotificationPriority) -> bool,
        gateway: Option<&Arc<Gateway>>,
    ) {
        loop {
            let result = self
                .drain_announces_filtered(session_id, |p| predicate(p))
                .await;
            if result.announces.is_empty() && result.system_notifications.is_empty() {
                break;
            }
            let Some(cs) = self.get_conversation_session(session_id).await else {
                warn!(
                    session_id = %session_id,
                    "drain_and_inject_announces_filtered: session missing mid-drain"
                );
                return;
            };
            inject_announces_as_system_messages(&cs, &result.announces).await;
            self.route_system_notifications(session_id, &result.system_notifications, gateway)
                .await;
        }
    }

    /// Route system notifications via simplified outbound path.
    /// Falls back to warn log when gateway is None or send fails.
    async fn route_system_notifications(
        &self,
        session_id: &str,
        notifications: &[String],
        gateway: Option<&Arc<Gateway>>,
    ) {
        if notifications.is_empty() {
            return;
        }
        let Some(gw) = gateway else {
            warn!(
                session_id = %session_id,
                count = notifications.len(),
                "route_system_notifications: gateway unavailable, dropping notifications"
            );
            return;
        };
        let chat_id = match self.get_chat_id(session_id).await {
            Some(id) => id,
            None => {
                warn!(
                    session_id = %session_id,
                    "route_system_notifications: chat_id not found, dropping notifications"
                );
                return;
            }
        };
        let channel = {
            let sessions = self.sessions.read().await;
            sessions.get(session_id).map(|s| s.channel.clone())
        };
        let Some(channel) = channel else {
            warn!(
                session_id = %session_id,
                "route_system_notifications: session not found, dropping notifications"
            );
            return;
        };
        for text in notifications {
            if let Err(e) = gw.send_outbound_simplified(&chat_id, &channel, text).await {
                warn!(
                    session_id = %session_id,
                    error = %e,
                    "route_system_notifications: send failed"
                );
            }
        }
    }

    /// Drain unsent outbound pending messages and re-deliver via gateway.
    pub async fn drain_outbound_pending_for_session(
        &self,
        session_id: &str,
    ) -> Result<usize, String> {
        // 1. Load checkpoint.
        let Some(cm) = self.checkpoint_manager.read().await.as_ref().cloned() else {
            return Ok(0);
        };
        let mut cp = cm
            .load(session_id)
            .await
            .map_err(|e| format!("drain_outbound_pending: failed to load checkpoint: {}", e))?
            .ok_or_else(|| {
                format!(
                    "drain_outbound_pending: checkpoint not found for session {}",
                    session_id
                )
            })?;

        // 2. Short-circuit: nothing to do.
        if cp.outbound_pending.is_empty() {
            return Ok(0);
        }
        // 3. Collect unsent message indices.
        let unsent_indices: Vec<usize> = cp
            .outbound_pending
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.sent)
            .map(|(i, _)| i)
            .collect();

        if unsent_indices.is_empty() {
            return Ok(0);
        }
        // 4. Fallback channel from sessions map (when target_channel is empty).
        let fallback_channel = {
            let sessions = self.sessions.read().await;
            sessions.get(session_id).map(|s| s.channel.clone())
        };
        // 5. Get Gateway reference for outbound delivery.
        let gw = self
            .get_gateway_ref()
            .await
            .ok_or_else(|| "drain_outbound_pending: gateway not available".to_string())?;

        // 5a. Persist checkpoint before delivery for crash recovery detection.
        cp.touch();
        if let Err(e) = cm.save_raw(&cp).await {
            warn!(
                session_id = %session_id,
                error = %e,
                "drain_outbound_pending: failed to persist checkpoint before delivery"
            );
        }

        // 6. Pre-build transcript content lookup table.
        //    HashMap<content, content> for O(1) lookups in the delivery loop.
        let transcript_map: HashMap<String, String> =
            if let Some(cs) = self.get_conversation_session(session_id).await {
                let cs_read = cs.read().await;
                cs_read
                    .messages()
                    .iter()
                    .filter(|m| m.role == "assistant")
                    .filter_map(|m| {
                        let text: String = m
                            .content_blocks
                            .iter()
                            .filter_map(|b| match b {
                                closeclaw_common::ContentBlock::Text(t) => Some(t.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        (!text.is_empty()).then_some((text.clone(), text))
                    })
                    .collect()
            } else {
                HashMap::new()
            };
        // 7. Deliver each unsent message. Channel: target_channel → session fallback.
        //    Content: transcript O(1) lookup → outbound_pending cache fallback.
        let mut delivered = 0usize;
        let mut handled_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for idx in &unsent_indices {
            // Clone fields before any mutable access to avoid borrow conflicts.
            let (msg_id, target_channel, content_cache) = {
                let pm = &cp.outbound_pending[*idx];
                (
                    pm.message_id.clone(),
                    pm.target_channel.clone(),
                    pm.content.clone(),
                )
            };
            let channel = if !target_channel.is_empty() {
                target_channel
            } else if let Some(ref ch) = fallback_channel {
                ch.clone()
            } else {
                warn!(
                    session_id = %session_id,
                    message_id = %msg_id,
                    "drain_outbound_pending: no channel available for message, skipping"
                );
                continue;
            };
            // Look up message content from the pre-built transcript map.
            // Falls back to outbound_pending cache if not found.
            let content = transcript_map
                .get(&content_cache)
                .cloned()
                .unwrap_or(content_cache);
            match gw
                .send_outbound(session_id, &channel, &content, vec![], None, None)
                .await
            {
                Ok(crate::outbound::SendOutcome::Sent) => {
                    cp.outbound_pending[*idx].mark_sent();
                    delivered += 1;
                    handled_ids.insert(msg_id);
                }
                Ok(crate::outbound::SendOutcome::Notified) => {
                    // Original send failed; user was already notified via
                    // simplified path. Do NOT mark_sent (wasn't delivered)
                    // and do NOT count as delivered. Track as handled so
                    // pending_operations is cleaned up (no retry needed).
                    handled_ids.insert(msg_id);
                }
                Err(e) => {
                    warn!(
                        session_id = %session_id,
                        message_id = %msg_id,
                        error = %e,
                        "drain_outbound_pending: delivery failed, skipping"
                    );
                }
            }
        }

        // 8. Remove OutboundMessage entries from pending_operations for handled
        //     messages (delivered or notified), then persist the updated checkpoint.
        if !handled_ids.is_empty() {
            cp.pending_operations.retain(|op| {
                if op.op_type
                    == closeclaw_session::persistence::PendingOperationType::OutboundMessage
                {
                    !handled_ids.contains(&op.op_id)
                } else {
                    true
                }
            });
            cp.touch();
            if let Err(e) = cm.save_raw(&cp).await {
                warn!(
                    session_id = %session_id,
                    error = %e,
                    "drain_outbound_pending: failed to persist checkpoint"
                );
            }
        }

        Ok(delivered)
    }
}

// ── try_push_announce + private helpers ─────────────────────────────────────

impl SessionManager {
    /// Push announce from completed child to parent's queue. Best-effort,
    /// errors logged but not propagated.
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

    /// Find run-mode parent for a child session. Returns None for
    /// non-children or session-mode children.
    async fn find_run_mode_parent(&self, child_session_id: &str) -> Option<(String, String)> {
        let children = self.children.read().await;
        children
            .find_child(child_session_id)
            .filter(|i| i.mode == SpawnMode::Run)
            .map(|info| (info.parent_session_id.clone(), info.agent_id.clone()))
    }

    /// Extract concatenated Text blocks from child's last assistant message.
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
    /// but runs directly on SessionManager. Processes queued announce
    /// events first, then any pending user messages.
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
                // Invoke LLM for the queued message.
                // Set default request context (no inbound metadata for queued messages).
                cs.read()
                    .await
                    .set_request_context(closeclaw_common::RequestContext::default());
                let result = cs.write().await.invoke_llm(&pending.content).await;
                // Clear busy state.
                {
                    let cs_write = cs.write().await;
                    cs_write.set_llm_busy(false);
                    cs_write.set_llm_state(closeclaw_llm::session_state::LlmState::Idle);
                }
                // Append response to session history and send to user.
                if let Ok(response) = result {
                    let text = response
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
                        cs_write.append_response(response);
                    }
                    // Send response to user via Gateway outbound pipeline.
                    if let (Some(ref gw), Some(ref ch)) = (&gw, &channel) {
                        if let Err(e) = gw
                            .send_outbound(session_id, ch, &text, vec![], None, None)
                            .await
                        {
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

// ── Private helpers ─────────────────────────────────────────────────────────

/// Inject announce events as system messages with priority prefixes.
///
/// Each event is formatted as:
/// `[子 agent {id}] [{priority_prefix}] {status_label}：\n{result_text}`
///
/// Extracted from `drain_and_inject_announces` and
/// `drain_and_inject_announces_filtered` to avoid duplicating the
/// injection logic.
async fn inject_announces_as_system_messages(
    cs: &std::sync::Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>>,
    events: &[AnnounceEvent],
) {
    let mut cs_write = cs.write().await;
    for event in events {
        let prefix = priority_prefix(&event.priority);
        let status_label = match event.status {
            ChildCompletionStatus::Completed => "任务已完成",
            ChildCompletionStatus::Errored => "任务出错",
            ChildCompletionStatus::Terminated => "任务被终止",
        };
        let text = format!(
            "[子 agent {}] {}{}：\n{}",
            event.child_agent_id, prefix, status_label, event.result_text
        );
        cs_write.inject_system_message(text);
    }
}

// ── Free helpers ────────────────────────────────────────────────────────────

/// Convert a background tool [`CompletionNotification`] into an
/// [`AnnounceEvent`]. Used by `drain_announces` and
/// `drain_announces_filtered` to avoid duplicating the conversion.
fn notif_to_announce(notif: closeclaw_tasks::CompletionNotification) -> AnnounceEvent {
    use closeclaw_tasks::TaskState;
    let status = match notif.state {
        TaskState::Completed { .. } => ChildCompletionStatus::Completed,
        TaskState::Failed { .. } => ChildCompletionStatus::Errored,
        TaskState::Killed => ChildCompletionStatus::Terminated,
        TaskState::Running { .. } => {
            unreachable!("CompletionNotification should never have Running state")
        }
    };
    let result_text = format!(
        "{}。输出文件：{}{}",
        notif.summary,
        notif.output_path.display(),
        notif
            .suggestion
            .as_ref()
            .map(|s| format!("。建议：{}", s))
            .unwrap_or_default()
    );
    AnnounceEvent {
        child_session_id: notif.task_id,
        child_agent_id: notif.command,
        result_text,
        completed_at: chrono::Utc::now(),
        priority: notif.priority,
        status,
    }
}

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
