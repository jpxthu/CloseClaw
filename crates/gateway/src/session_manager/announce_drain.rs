//! Announce queue drain side: queue accessors, announce draining and
//! injection as system messages, system-notification routing, and the
//! outbound pending redelivery (Step 1.3).
//!
//! Split from `announce.rs` (which keeps the push / notify / recovery
//! side) so both files keep line-count margin (CONTRIBUTING.md
//! file-size limit); behavior unchanged, UT live in `announce_tests.rs`,
//! `announce_drain_outbound_tests.rs`, and the other announce test
//! siblings.

use std::collections::HashMap;
use std::sync::Arc;

use super::SessionManager;
use crate::outbound::SendOutboundIds;
use crate::Gateway;
use closeclaw_common::ChildCompletionStatus;
use closeclaw_common::NotificationPriority;
use closeclaw_session::llm_session::{
    AnnounceEvent, ChatSession, ConversationSession, NotificationAudience, QueueEntry,
};
use tracing::warn;

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
///
/// The test-only accessors (`is_empty` / `len` / `iter` / `Index<usize>`)
/// live in `tests.rs` (Step 1.8 ② / Step 1.9: test-only code moved out of
/// the production file).
#[derive(Debug)]
pub(crate) struct DrainResult {
    pub announces: Vec<AnnounceEvent>,
    pub system_notifications: Vec<OutboundSystemNotification>,
}

/// A drained system notification ready for outbound routing.
///
/// The [`NotificationAudience`] declaration is carried **independently**
/// of the text, so the audience flag never piggybacks on the message
/// body. Note that the `text` prefix shape differs by drain path: only
/// `drain_announces_filtered` concatenates the `[紧急]`/`[注意]` priority
/// prefix into `text`; the unfiltered `drain_announces` passes the
/// queued text through unchanged.
#[derive(Debug, Clone)]
pub(crate) struct OutboundSystemNotification {
    pub text: String,
    pub audience: NotificationAudience,
}

// ── Queue accessors (push / drain) + drain pipeline ────────────────────────

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

    /// Drain all queued announce events.
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
                QueueEntry::SystemNotification(text, _, audience) => {
                    system_notifications.push(OutboundSystemNotification { text, audience });
                }
            }
        }
        DrainResult {
            announces,
            system_notifications,
        }
    }

    /// Drain and inject announce events.
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

    /// Drain announce events matching a predicate. Non-matching re-inserted.
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
                QueueEntry::SystemNotification(ref text, ref priority, ref audience)
                    if predicate(priority) =>
                {
                    matched_notifications.push(OutboundSystemNotification {
                        text: format!("{}{}", priority_prefix(priority), text),
                        audience: *audience,
                    });
                }
                QueueEntry::SystemNotification(text, priority, audience) => {
                    cs.push_queue_entry(QueueEntry::SystemNotification(text, priority, audience));
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

    /// Drain and inject filtered announce events.
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
    ///
    /// Each notification's audience declaration travels with it: the
    /// session's current sender (checkpoint `sender_id`) is resolved
    /// once and handed to the Gateway together with the flag, so the
    /// Gateway can perform the Owner-only judgment at delivery time.
    async fn route_system_notifications(
        &self,
        session_id: &str,
        notifications: &[OutboundSystemNotification],
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
        let sender_id = self.get_sender_id(session_id).await;
        for notification in notifications {
            gw.send_system_notification(
                &chat_id,
                &channel,
                &notification.text,
                notification.audience,
                sender_id.as_deref(),
            )
            .await;
        }
    }

    /// Drain unsent outbound pending messages and re-deliver via gateway.
    ///
    /// Step 1.3: Drive source is `pending_operations` with `op_type ==
    /// OutboundMessage` (not the entire `outbound_pending` cache).
    /// For each op: content lookup in `outbound_pending` by `message_id`,
    /// fallback to transcript; channel from `op.detail.target_channel` or
    /// session fallback. On success/notified: clear the pending op.
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

        // 2. Short-circuit: no OutboundMessage pending ops → nothing to do.
        //    Collect into owned tuples to avoid holding a borrow on cp
        //    across the mutable clear_outbound_pending_op calls.
        let outbound_ops: Vec<(String, String)> = cp
            .pending_operations
            .iter()
            .filter(|op| {
                op.op_type == closeclaw_session::persistence::PendingOperationType::OutboundMessage
            })
            .map(|op| {
                (
                    op.detail.message_id().unwrap_or(&op.op_id).to_string(),
                    op.detail.target_channel().unwrap_or(""),
                )
            })
            .map(|(mid, ch)| (mid, ch.to_string()))
            .collect();
        if outbound_ops.is_empty() {
            return Ok(0);
        }

        // 3. Fallback channel from sessions map (when target_channel is empty).
        let fallback_channel = {
            let sessions = self.sessions.read().await;
            sessions.get(session_id).map(|s| s.channel.clone())
        };
        // 4. Get Gateway reference for outbound delivery.
        let gw = self
            .get_gateway_ref()
            .await
            .ok_or_else(|| "drain_outbound_pending: gateway not available".to_string())?;

        // 4a. Persist checkpoint before delivery for crash recovery detection.
        cp.touch();
        if let Err(e) = cm.save_raw(&cp).await {
            warn!(
                session_id = %session_id,
                error = %e,
                "drain_outbound_pending: failed to persist checkpoint before delivery"
            );
        }

        // 5. Pre-build transcript content lookup table.
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

        // 6. Build outbound_pending content lookup: message_id → content cache.
        //    Used for content resolution when the pending op matches a cache entry.
        let cache_map: HashMap<String, String> = cp
            .outbound_pending
            .iter()
            .map(|pm| (pm.message_id.clone(), pm.content.clone()))
            .collect();

        // 7. Deliver each OutboundMessage pending op.
        //    Content: cache (by message_id) → transcript → skip.
        //    Channel: op.detail.target_channel → session fallback → skip.
        let mut delivered = 0usize;
        for (message_id, op_channel) in &outbound_ops {
            // 7a. Resolve channel.
            let channel = if !op_channel.is_empty() {
                op_channel.clone()
            } else if let Some(ref ch) = fallback_channel {
                ch.clone()
            } else {
                warn!(
                    session_id = %session_id,
                    message_id = %message_id,
                    "drain_outbound_pending: no channel available, skipping"
                );
                continue;
            };

            // 7b. Resolve content: cache hit → transcript refresh; miss → skip.
            let content = if let Some(cached) = cache_map.get(message_id) {
                // Transcript is authoritative; use cached content as key.
                transcript_map
                    .get(cached)
                    .cloned()
                    .unwrap_or_else(|| cached.clone())
            } else {
                // No cache entry — preserve op for next attempt.
                warn!(
                    session_id = %session_id,
                    message_id = %message_id,
                    "drain_outbound_pending: no content in cache, skipping (op preserved)"
                );
                continue;
            };

            // 7c. Send via gateway.
            // Explicit SendOutboundIds::default() (not bare Default::default())
            // to match the explicit style of all other send_outbound call sites.
            let ids = SendOutboundIds::default();
            match gw
                .send_outbound(session_id, &channel, &content, vec![], ids)
                .await
            {
                Ok(crate::outbound::SendOutcome::Sent) => {
                    delivered += 1;
                    // Only clear on Sent; preserve Notified for retry (doc: 宁可重复也不遗漏).
                    cp.clear_outbound_pending_op(message_id);
                }
                Ok(crate::outbound::SendOutcome::Notified) => {
                    // Send failed but user notified — preserve op for retry.
                    warn!(session_id = %session_id, message_id = %message_id,
                        "drain_outbound_pending: delivery not confirmed (Notified), op preserved");
                }
                Err(e) => {
                    warn!(
                        session_id = %session_id,
                        message_id = %message_id,
                        error = %e,
                        "drain_outbound_pending: delivery failed, op preserved for retry"
                    );
                }
            }
        }

        // 8. Persist checkpoint if any ops were cleared.
        cp.touch();
        if let Err(e) = cm.save_raw(&cp).await {
            warn!(
                session_id = %session_id,
                error = %e,
                "drain_outbound_pending: failed to persist checkpoint"
            );
        }

        Ok(delivered)
    }
}

// ── Private helpers ─────────────────────────────────────────────────────────
/// Inject announce events as system messages with priority prefixes.
///
/// Each event is formatted as:
/// `[子 agent {id}] [{priority_prefix}] {status_label}：\n{result_text}`
/// (extracted from the two drain_and_inject variants to avoid duplication).
async fn inject_announces_as_system_messages(
    cs: &std::sync::Arc<tokio::sync::RwLock<ConversationSession>>,
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
/// [`AnnounceEvent`] (shared by `drain_announces` / `drain_announces_filtered`).
fn notif_to_announce(notif: closeclaw_common::CompletionNotification) -> AnnounceEvent {
    use closeclaw_common::TaskState;
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
