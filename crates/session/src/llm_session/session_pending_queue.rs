//! Pending messages and announce queue methods for `ConversationSession`.
//!
//! Extracted from `mod.rs` to keep file sizes within the 1000-line limit.

use super::AnnounceEvent;
use super::ConversationSession;
use closeclaw_common::ContentBlock;

/// Pending messages and announce queue.
impl ConversationSession {
    /// Pushes a pending message onto the queue.
    pub fn push_pending(&mut self, msg: crate::persistence::PendingMessage) {
        self.pending_messages.push_back(msg);
    }

    /// Pops the oldest pending message, if any.
    pub fn pop_pending(&mut self) -> Option<crate::persistence::PendingMessage> {
        self.pending_messages.pop_front()
    }

    /// Clears all pending messages from the queue.
    /// Returns the number of messages that were cleared.
    pub fn clear_pending(&mut self) -> usize {
        let n = self.pending_messages.len();
        self.pending_messages.clear();
        n
    }

    /// Returns whether there are any pending messages.
    pub fn has_pending(&self) -> bool {
        !self.pending_messages.is_empty()
    }

    /// Returns the number of pending messages.
    pub fn pending_count(&self) -> usize {
        self.pending_messages.len()
    }

    /// Returns a clone of all pending messages without consuming the queue.
    pub fn get_pending_messages(&self) -> Vec<crate::persistence::PendingMessage> {
        self.pending_messages.iter().cloned().collect()
    }

    /// Restores pending messages from checkpoint data.
    /// Only pushes messages where `sent == false` back into the queue.
    pub fn restore_pending_messages(&mut self, messages: Vec<crate::persistence::PendingMessage>) {
        for msg in messages {
            if !msg.sent {
                self.pending_messages.push_back(msg);
            }
        }
    }

    /// Push an announce event onto the in-memory announce queue.
    ///
    /// Events are inserted in priority order (`Now` > `Next` > `Later`).
    /// Within the same priority level, FIFO insertion order is preserved
    /// (stable sort).
    pub fn push_announce_to_queue(&mut self, event: AnnounceEvent) {
        // Deduplication: skip if an event with the same child_session_id
        // already exists in the queue.
        if self
            .announce_queue
            .iter()
            .any(|e| e.child_session_id == event.child_session_id)
        {
            tracing::debug!(
                child_session_id = %event.child_session_id,
                "skipping duplicate announce event"
            );
            return;
        }
        let pos = self
            .announce_queue
            .iter()
            .position(|e| e.priority < event.priority)
            .unwrap_or(self.announce_queue.len());
        self.announce_queue.insert(pos, event);
    }

    /// Drain all queued announce events, returning them in priority order.
    pub fn drain_announce_queue(&mut self) -> Vec<AnnounceEvent> {
        std::mem::take(&mut self.announce_queue)
    }

    /// Persist a user message into the conversation history.
    pub fn append_user_message(&mut self, content: &str) {
        self.append_transcript("user", vec![ContentBlock::Text(content.to_string())]);
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
