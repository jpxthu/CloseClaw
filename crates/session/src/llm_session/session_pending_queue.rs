//! Unified message queue for `ConversationSession`.
//!
//! Merges pending user messages and child-session announce events
//! into a single priority-ordered queue. See
//! `docs/design/session/session-execution.md` §统一消息队列 for
//! the full ordering specification.

use super::AnnounceEvent;
use super::ConversationSession;
use closeclaw_common::ContentBlock;
use closeclaw_tasks::NotificationPriority;

// ── Priority level ─────────────────────────────────────────────────────────

/// Priority level for the unified message queue.
///
/// Ordering: `Now > Next > Later` (higher priority drains first).
/// Within the same priority, non-user messages drain before user
/// messages; within the same group, FIFO order is preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QueuePriority {
    Later = 0,
    Next = 1,
    Now = 2,
}

impl From<NotificationPriority> for QueuePriority {
    fn from(p: NotificationPriority) -> Self {
        match p {
            NotificationPriority::Later => QueuePriority::Later,
            NotificationPriority::Next => QueuePriority::Next,
            NotificationPriority::Now => QueuePriority::Now,
        }
    }
}

// ── Queue entry ────────────────────────────────────────────────────────────

/// A single entry in the unified message queue.
#[derive(Debug, Clone)]
pub enum QueueEntry {
    /// A pending user message (always `Later` priority).
    UserMessage(crate::persistence::PendingMessage),
    /// A child session completion announce event.
    Announce(AnnounceEvent),
    /// A background tool (BashTool) completion notification.
    /// Priority is taken from the inner `CompletionNotification`.
    BackgroundToolNotification(closeclaw_tasks::CompletionNotification),
    /// A system-level notification (e.g. yield timeout warning).
    /// Injected as a `role="system"` message during drain.
    SystemNotification(String, NotificationPriority),
}

impl QueueEntry {
    /// Returns the priority of this entry.
    pub fn priority(&self) -> QueuePriority {
        match self {
            QueueEntry::UserMessage(_) => QueuePriority::Later,
            QueueEntry::Announce(e) => QueuePriority::from(e.priority),
            QueueEntry::BackgroundToolNotification(n) => QueuePriority::from(n.priority),
            QueueEntry::SystemNotification(_, p) => QueuePriority::from(*p),
        }
    }

    /// Returns `true` if this entry is a user message.
    pub fn is_user(&self) -> bool {
        matches!(self, QueueEntry::UserMessage(_))
    }
}

// ── Unified message queue ──────────────────────────────────────────────────

/// An internal wrapper pairing a queue entry with its insertion
/// sequence number for stable FIFO ordering within the same
/// priority / user group.
#[derive(Debug, Clone)]
pub struct QueueItem {
    pub entry: QueueEntry,
    pub seq: u64,
}

impl QueueItem {
    /// Sort key: higher priority first, non-user before user, lower
    /// seq first (FIFO).
    fn sort_key(&self) -> (std::cmp::Reverse<QueuePriority>, bool, u64) {
        (
            std::cmp::Reverse(self.entry.priority()),
            self.entry.is_user(),
            self.seq,
        )
    }
}

/// Unified message queue for a `ConversationSession`.
///
/// Maintains entries in sorted order at all times. Draining returns
/// entries from highest to lowest priority, with non-user messages
/// preceding user messages at the same priority level.
#[derive(Debug, Clone, Default)]
pub struct UnifiedMessageQueue {
    entries: Vec<QueueItem>,
    next_seq: u64,
}

impl UnifiedMessageQueue {
    /// Push an entry, maintaining sorted order.
    ///
    /// Announce entries are deduplicated by `child_session_id`.
    pub fn push(&mut self, entry: QueueEntry) {
        // Dedup: skip announce if same child_session_id already queued.
        if let QueueEntry::Announce(ref event) = entry {
            if self.entries.iter().any(|i| {
                matches!(
                    &i.entry,
                    QueueEntry::Announce(e) if e.child_session_id == event.child_session_id
                )
            }) {
                tracing::debug!(
                    child_session_id = %event.child_session_id,
                    "UnifiedMessageQueue: skipping duplicate announce"
                );
                return;
            }
        }
        let item = QueueItem {
            entry,
            seq: self.next_seq,
        };
        self.next_seq += 1;
        let key = item.sort_key();
        let pos = self
            .entries
            .iter()
            .position(|i| key < i.sort_key())
            .unwrap_or(self.entries.len());
        self.entries.insert(pos, item);
    }

    /// Pop the highest-priority entry, if any.
    pub fn pop(&mut self) -> Option<QueueEntry> {
        if self.entries.is_empty() {
            return None;
        }
        self.entries.drain(..1).next().map(|i| i.entry)
    }

    /// Push an entry with a specific sequence number, maintaining
    /// sorted order. Does NOT increment `next_seq`.
    ///
    /// Used by callers that need to re-insert entries while
    /// preserving the original insertion order.
    pub fn push_preserving_seq(&mut self, entry: QueueEntry, seq: u64) {
        let item = QueueItem { entry, seq };
        let key = item.sort_key();
        let pos = self
            .entries
            .iter()
            .position(|i| key < i.sort_key())
            .unwrap_or(self.entries.len());
        self.entries.insert(pos, item);
    }

    /// Drain all entries in priority order, leaving the queue empty.
    pub fn drain_all(&mut self) -> Vec<QueueEntry> {
        self.entries.drain(..).map(|i| i.entry).collect()
    }

    /// Drain all entries preserving sequence numbers.
    ///
    /// Used by callers that need to re-insert non-matching entries
    /// while preserving original FIFO ordering within the same
    /// priority level.
    pub fn drain_all_items(&mut self) -> Vec<QueueItem> {
        std::mem::take(&mut self.entries)
    }

    /// Returns `true` if the queue contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of entries in the queue.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns all pending user messages without consuming the queue.
    pub fn pending_user_messages(&self) -> Vec<crate::persistence::PendingMessage> {
        self.entries
            .iter()
            .filter_map(|i| match &i.entry {
                QueueEntry::UserMessage(pm) => Some(pm.clone()),
                _ => None,
            })
            .collect()
    }

    /// Returns all announce events without consuming the queue.
    pub fn announce_events(&self) -> Vec<AnnounceEvent> {
        self.entries
            .iter()
            .filter_map(|i| match &i.entry {
                QueueEntry::Announce(e) => Some(e.clone()),
                _ => None,
            })
            .collect()
    }

    /// Clear all entries and return the count removed.
    pub fn clear(&mut self) -> usize {
        let n = self.entries.len();
        self.entries.clear();
        n
    }

    /// Clear only user-message entries, preserving announces.
    /// Returns the number of user messages removed.
    pub fn clear_user_messages(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|i| !i.entry.is_user());
        before - self.entries.len()
    }
}

// ── ConversationSession queue methods ──────────────────────────────────────

/// Unified message queue methods for `ConversationSession`.
impl ConversationSession {
    // ── push ────────────────────────────────────────────────────────────

    /// Push a pending user message onto the unified queue
    /// (priority `Later`).
    pub fn push_pending(&mut self, msg: crate::persistence::PendingMessage) {
        self.unified_queue.push(QueueEntry::UserMessage(msg));
    }

    /// Push an announce event onto the unified queue.
    ///
    /// Events are inserted in priority order (`Now` > `Next` > `Later`).
    /// Within the same priority level, non-user messages come first.
    /// Deduplication: events with the same `child_session_id` are
    /// rejected.
    pub fn push_announce_to_queue(&mut self, event: AnnounceEvent) {
        self.unified_queue.push(QueueEntry::Announce(event));
    }

    /// Push a background tool completion notification onto the unified
    /// queue (priority from the notification, typically `Later`).
    pub fn push_background_tool_notification(
        &mut self,
        notif: closeclaw_tasks::CompletionNotification,
    ) {
        self.unified_queue
            .push(QueueEntry::BackgroundToolNotification(notif));
    }

    /// Push a system-level notification onto the unified queue.
    ///
    /// Used by yield timeout handlers to enqueue warnings and timeout
    /// notifications with the specified priority (typically `Next`).
    /// During drain, system notifications are injected as
    /// `role="system"` messages into the conversation history.
    pub fn push_system_notification(&mut self, text: String, priority: NotificationPriority) {
        self.unified_queue
            .push(QueueEntry::SystemNotification(text, priority));
    }

    /// Push a `QueueEntry` directly onto the unified queue.
    ///
    /// Used by drain-and-filter callers that re-insert non-matching
    /// entries back into the queue after draining.
    pub fn push_queue_entry(&mut self, entry: QueueEntry) {
        self.unified_queue.push(entry);
    }

    // ── pop / drain ─────────────────────────────────────────────────────

    /// Pop the highest-priority entry from the unified queue.
    pub fn pop_queue_entry(&mut self) -> Option<QueueEntry> {
        self.unified_queue.pop()
    }

    /// Drain all entries from the unified queue in priority order.
    ///
    /// Per the design doc, entries should only be dequeued when
    /// `llm_active` and `foreground_tool_active` are both false;
    /// this condition is enforced by the caller.
    pub fn drain_queue(&mut self) -> Vec<QueueEntry> {
        self.unified_queue.drain_all()
    }

    /// Drain all entries from the unified queue (legacy compat alias).
    pub fn drain_all_entries(&mut self) -> Vec<QueueEntry> {
        self.unified_queue.drain_all()
    }

    /// Pop the oldest pending user message (legacy compat).
    ///
    /// Returns only user messages. Non-user-message entries are
    /// collected, the first user message (if any) is returned, and
    /// all other entries are re-inserted in original order.
    pub fn pop_pending(&mut self) -> Option<crate::persistence::PendingMessage> {
        let all = self.unified_queue.drain_all();
        let mut result = None;
        let mut kept = Vec::new();
        for item in all {
            if result.is_none() {
                if let QueueEntry::UserMessage(msg) = item {
                    result = Some(msg);
                    continue;
                }
            }
            kept.push(item);
        }
        for item in kept {
            self.unified_queue.push(item);
        }
        result
    }

    /// Drain all announce events from the unified queue (legacy compat).
    ///
    /// Returns announce events in priority order. User messages and
    /// background tool notifications remain in the queue with their
    /// original sequence numbers preserved.
    pub fn drain_announce_queue(&mut self) -> Vec<AnnounceEvent> {
        let all = self.unified_queue.drain_all_items();
        let mut announces = Vec::new();
        for item in all {
            if let QueueEntry::Announce(e) = item.entry {
                announces.push(e);
            } else {
                self.unified_queue.push_preserving_seq(item.entry, item.seq);
            }
        }
        announces
    }

    // ── status ──────────────────────────────────────────────────────────

    /// Returns whether the unified queue has any entries.
    pub fn has_pending(&self) -> bool {
        !self.unified_queue.is_empty()
    }

    /// Returns the total number of entries in the unified queue.
    pub fn pending_count(&self) -> usize {
        self.unified_queue.len()
    }

    /// Returns whether the queue is empty.
    pub fn is_queue_empty(&self) -> bool {
        self.unified_queue.is_empty()
    }

    /// Returns the total number of entries in the queue.
    pub fn queue_len(&self) -> usize {
        self.unified_queue.len()
    }

    // ── accessors (no consume) ──────────────────────────────────────────

    /// Returns all pending user messages without consuming the queue.
    pub fn get_pending_messages(&self) -> Vec<crate::persistence::PendingMessage> {
        self.unified_queue.pending_user_messages()
    }

    /// Returns all announce events without consuming the queue.
    pub fn get_announce_events(&self) -> Vec<AnnounceEvent> {
        self.unified_queue.announce_events()
    }

    // ── restore / clear ─────────────────────────────────────────────────

    /// Restore pending user messages from checkpoint data.
    /// Only pushes messages where `sent == false` back into the queue.
    pub fn restore_pending_messages(&mut self, messages: Vec<crate::persistence::PendingMessage>) {
        for msg in messages {
            if !msg.sent {
                self.unified_queue.push(QueueEntry::UserMessage(msg));
            }
        }
    }

    /// Clear all entries from the unified queue.
    /// Returns the number of entries that were cleared.
    pub fn clear_queue(&mut self) -> usize {
        self.unified_queue.clear()
    }

    /// Clear all pending user messages from the queue (legacy compat).
    /// Returns the number of messages that were cleared.
    pub fn clear_pending(&mut self) -> usize {
        self.unified_queue.clear_user_messages()
    }

    // ── transcript helpers (unchanged) ──────────────────────────────────

    /// Persist a user message into the conversation history.
    pub fn append_user_message(&mut self, content: &str) {
        self.append_transcript("user", vec![ContentBlock::Text(content.to_string())]);
    }

    /// Persist a user message with structured content blocks.
    ///
    /// Used when the user message contains multimodal content (e.g.,
    /// images as [`ContentBlock::Image`]) that should be preserved
    /// as structured blocks rather than flattened to text.
    pub fn append_user_content_blocks(&mut self, blocks: Vec<ContentBlock>) {
        self.append_transcript("user", blocks);
    }

    /// Inject a system message into the conversation history.
    pub fn inject_system_message(&mut self, text: String) {
        self.append_transcript("system", vec![ContentBlock::Text(text)]);
    }

    /// Inject a tool result into the conversation history.
    pub fn inject_tool_result(&mut self, tool_call_id: &str, content: &str) {
        self.append_transcript(
            "tool",
            vec![ContentBlock::ToolResult {
                tool_call_id: tool_call_id.to_string(),
                content: content.to_string(),
            }],
        );
    }

    /// Extract pending tool calls from the last assistant message.
    pub fn extract_pending_tool_calls(&self) -> Vec<crate::persistence::PendingOperation> {
        use crate::persistence::{
            PendingOperation, PendingOperationDetail, PendingOperationStatus, PendingOperationType,
        };
        let last_assistant = self.messages.iter().rev().find(|m| m.role == "assistant");
        let Some(msg) = last_assistant else {
            return Vec::new();
        };
        let now = chrono::Utc::now();
        msg.content_blocks
            .iter()
            .filter_map(|block| {
                if let ContentBlock::ToolUse { id, name, input } = block {
                    Some(PendingOperation {
                        op_id: id.clone(),
                        op_type: PendingOperationType::ToolCall,
                        status: PendingOperationStatus::Running,
                        detail: PendingOperationDetail::ToolCall {
                            tool_name: name.clone(),
                            args_summary: input.clone(),
                        },
                        created_at: now,
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}
