//! Session layer for LLM conversations.
//!
//! Provides `SessionMessage`, `ChatSession` trait and `ConversationSession`
//! for managing conversation state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::llm::turn::TurnCounter;
use crate::llm::types::{ContentBlock, InternalMessage, InternalRequest, UnifiedResponse};

/// A single message in a conversation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    /// Role of the message sender (e.g., "user", "assistant", "system").
    pub role: String,
    /// Ordered list of content blocks.
    pub content_blocks: Vec<ContentBlock>,
    /// When the message was created.
    pub timestamp: DateTime<Utc>,
}

/// Trait for managing conversation session state.
///
/// All implementations must be thread-safe (`Send + Sync`) to allow
/// use across async contexts.
pub trait ChatSession: Send + Sync {
    /// Returns the current list of messages in the session.
    fn messages(&self) -> &[SessionMessage];

    /// Returns the system prompt, if one is set.
    fn system_prompt(&self) -> Option<&str>;

    /// Returns the current turn count.
    ///
    /// A turn typically corresponds to one user → assistant exchange.
    fn turn_count(&self) -> u32;

    /// Appends an assistant response to the session.
    ///
    /// The response's content blocks are converted into a new `SessionMessage`
    /// with role "assistant".
    fn append_response(&mut self, response: UnifiedResponse);

    /// Appends a tool result to the session and increments the turn count.
    ///
    /// The tool result is added as a `ContentBlock::ToolResult` to the
    /// last assistant message, and `turn_count` is incremented.
    fn append_tool_result(&mut self, tool_call_id: String, result: String);

    /// Builds an `InternalRequest` from the current session state.
    ///
    /// Includes system prompt (if set), all messages, model, temperature
    /// and max_tokens.
    fn build_api_request(&self) -> InternalRequest;

    /// Returns whether the LLM is currently busy (processing a request).
    fn is_llm_busy(&self) -> bool;
}

/// A simple in-memory implementation of `ChatSession`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ConversationSession {
    session_id: String,
    messages: Vec<SessionMessage>,
    system_prompt: Option<String>,
    turn_counter: TurnCounter,
    model: String,
    compaction_state: Option<String>,
    is_llm_busy: Arc<AtomicBool>,
    pending_messages: VecDeque<crate::session::persistence::PendingMessage>,
}

impl ConversationSession {
    /// Creates a new session with the given model.
    pub fn new(session_id: String, model: String) -> Self {
        Self {
            session_id,
            messages: Vec::new(),
            system_prompt: None,
            turn_counter: TurnCounter::new(),
            model,
            compaction_state: None,
            is_llm_busy: Arc::new(AtomicBool::new(false)),
            pending_messages: VecDeque::new(),
        }
    }

    /// Sets the system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Appends a message to the session.
    fn push_message(&mut self, role: &str, content_blocks: Vec<ContentBlock>) {
        self.messages.push(SessionMessage {
            role: role.to_string(),
            content_blocks,
            timestamp: Utc::now(),
        });
    }

    /// Sets the LLM busy state.
    pub fn set_llm_busy(&self, busy: bool) {
        self.is_llm_busy.store(busy, Ordering::SeqCst);
    }

    /// Pushes a pending message onto the queue.
    pub fn push_pending(&mut self, msg: crate::session::persistence::PendingMessage) {
        self.pending_messages.push_back(msg);
    }

    /// Pops the oldest pending message, if any.
    pub fn pop_pending(&mut self) -> Option<crate::session::persistence::PendingMessage> {
        self.pending_messages.pop_front()
    }

    /// Returns whether there are any pending messages.
    pub fn has_pending(&self) -> bool {
        !self.pending_messages.is_empty()
    }

    /// Returns the number of pending messages.
    pub fn pending_count(&self) -> usize {
        self.pending_messages.len()
    }
}

impl ChatSession for ConversationSession {
    fn messages(&self) -> &[SessionMessage] {
        &self.messages
    }

    fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    fn turn_count(&self) -> u32 {
        self.turn_counter.count()
    }

    fn append_response(&mut self, response: UnifiedResponse) {
        // Empty content_blocks means keepalive — do not add a message or increment turn.
        if response.content_blocks.is_empty() {
            return;
        }
        self.push_message("assistant", response.content_blocks);
    }

    fn append_tool_result(&mut self, tool_call_id: String, result: String) {
        // Find the last assistant message and append a ToolResult block to it.
        // If no assistant message exists, we still increment turn (tool call implies a turn).
        let tool_block = ContentBlock::ToolResult {
            tool_call_id,
            content: result,
        };
        if let Some(last) = self.messages.iter_mut().find(|m| m.role == "assistant") {
            last.content_blocks.push(tool_block);
        }
        self.turn_counter.increment();
    }

    fn build_api_request(&self) -> InternalRequest {
        let mut msgs = Vec::new();
        // Prepend system prompt if set.
        if let Some(prompt) = &self.system_prompt {
            msgs.push(InternalMessage {
                role: "system".into(),
                content: prompt.clone(),
            });
        }
        for msg in &self.messages {
            let content = msg
                .content_blocks
                .iter()
                .flat_map(|b| match b {
                    ContentBlock::Text(t) => vec![t.clone()],
                    ContentBlock::Thinking(t) => vec![format!("<thinking>{}</thinking>", t)],
                    ContentBlock::ToolUse { name, input, .. } => {
                        vec![format!("[tool:{}] {}", name, input)]
                    }
                    ContentBlock::ToolResult { content, .. } => vec![content.clone()],
                })
                .collect::<Vec<_>>()
                .join("\n");
            msgs.push(InternalMessage {
                role: msg.role.clone(),
                content,
            });
        }
        InternalRequest {
            model: self.model.clone(),
            messages: msgs,
            temperature: 0.0,
            max_tokens: None,
            stream: false,
            extra_body: Default::default(),
        }
    }

    fn is_llm_busy(&self) -> bool {
        self.is_llm_busy.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::UnifiedUsage;
    use crate::session::persistence::PendingMessage;
    use std::sync::Arc;
    use std::thread;

    // ── llm_busy state ─────────────────────────────────────────────────────────

    #[test]
    fn test_is_llm_busy_default_false() {
        let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into());
        assert!(!session.is_llm_busy());
    }

    #[test]
    fn test_set_llm_busy_true() {
        let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into());
        session.set_llm_busy(true);
        assert!(session.is_llm_busy());
    }

    #[test]
    fn test_set_llm_busy_false_recovers() {
        let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into());
        session.set_llm_busy(true);
        session.set_llm_busy(false);
        assert!(!session.is_llm_busy());
    }

    #[test]
    fn test_set_llm_busy_concurrent_no_panic() {
        let session = Arc::new(ConversationSession::new(
            "sess_concurrent".into(),
            "gpt-4o".into(),
        ));
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let s = Arc::clone(&session);
                thread::spawn(move || {
                    s.set_llm_busy(i % 2 == 0);
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread panicked");
        }
    }

    // ── pending_messages queue ──────────────────────────────────────────────────

    #[test]
    fn test_pending_initial_state() {
        let session = ConversationSession::new("sess_pending".into(), "gpt-4o".into());
        assert_eq!(session.pending_count(), 0);
        assert!(!session.has_pending());
    }

    #[test]
    fn test_push_pending_sets_has_pending_and_increments_count() {
        let mut session = ConversationSession::new("sess_pending".into(), "gpt-4o".into());
        assert_eq!(session.pending_count(), 0);
        session.push_pending(PendingMessage::new("msg_1".into(), "hello".into()));
        assert!(session.has_pending());
        assert_eq!(session.pending_count(), 1);
        session.push_pending(PendingMessage::new("msg_2".into(), "world".into()));
        assert_eq!(session.pending_count(), 2);
    }

    #[test]
    fn test_pop_pending_fifo_order() {
        let mut session = ConversationSession::new("sess_fifo".into(), "gpt-4o".into());
        session.push_pending(PendingMessage::new("msg_A".into(), "first".into()));
        session.push_pending(PendingMessage::new("msg_B".into(), "second".into()));
        let first = session.pop_pending();
        assert!(first.is_some());
        assert_eq!(first.unwrap().message_id, "msg_A");
        let second = session.pop_pending();
        assert!(second.is_some());
        assert_eq!(second.unwrap().message_id, "msg_B");
    }

    #[test]
    fn test_pop_pending_returns_none_when_empty() {
        let mut session = ConversationSession::new("sess_empty".into(), "gpt-4o".into());
        assert!(session.pop_pending().is_none());
    }

    // ── SessionMessage serde roundtrip ────────────────────────────────────────

    #[test]
    fn test_session_message_serde_roundtrip() {
        let msg = SessionMessage {
            role: "user".into(),
            content_blocks: vec![
                ContentBlock::Text("hello".into()),
                ContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "get_weather".into(),
                    input: r#"{"city":"Tokyo"}"#.into(),
                },
            ],
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SessionMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg.role, parsed.role);
        assert_eq!(msg.content_blocks, parsed.content_blocks);
    }

    // ── ConversationSession initial state ─────────────────────────────────────

    #[test]
    fn test_conversation_session_new() {
        let session = ConversationSession::new("sess_42".into(), "gpt-4o".into());
        assert_eq!(session.messages().len(), 0);
        assert_eq!(session.turn_count(), 0);
        assert!(session.system_prompt().is_none());
    }

    // ── append_response adds a message ─────────────────────────────────────────

    #[test]
    fn test_append_response_adds_message() {
        let mut session = ConversationSession::new("sess_1".into(), "gpt-4o".into());
        let response = UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("Hi there!".into())],
            usage: UnifiedUsage {
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: Some(3),
                reasoning_tokens: None,
            },
            finish_reason: Some("stop".into()),
        };
        session.append_response(response);
        assert_eq!(session.messages().len(), 1);
        assert_eq!(session.messages()[0].role, "assistant");
    }

    // ── append_tool_result increments turn ────────────────────────────────────

    #[test]
    fn test_append_tool_result_increments_turn() {
        let mut session = ConversationSession::new("sess_2".into(), "gpt-4o".into());
        // Need an assistant message first so tool_result has somewhere to attach.
        session.append_response(UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("Using tool...".into())],
            usage: UnifiedUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: Some(2),
                reasoning_tokens: None,
            },
            finish_reason: Some("stop".into()),
        });
        assert_eq!(session.turn_count(), 0);
        session.append_tool_result("call_x".into(), "tool output".into());
        assert_eq!(session.turn_count(), 1);
    }

    // ── append_response with empty blocks does NOT increment turn ──────────────

    #[test]
    fn test_append_response_empty_blocks_no_turn_increment() {
        let mut session = ConversationSession::new("sess_3".into(), "gpt-4o".into());
        session.append_response(UnifiedResponse {
            content_blocks: vec![],
            usage: UnifiedUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: Some(0),
                reasoning_tokens: None,
            },
            finish_reason: None,
        });
        assert_eq!(session.messages().len(), 0); // no message added
        assert_eq!(session.turn_count(), 0); // no turn incremented
    }

    // ── build_api_request with system_prompt ───────────────────────────────────

    #[test]
    fn test_build_api_request_includes_system_prompt() {
        let session = ConversationSession::new("sess_4".into(), "gpt-4o".into())
            .with_system_prompt("You are helpful.");
        let req = session.build_api_request();
        // system prompt should appear as a messages entry with role "system"
        assert!(req
            .messages
            .iter()
            .any(|m| m.role == "system" && m.content.contains("helpful")));
    }

    // ── build_api_request without system_prompt ────────────────────────────────

    #[test]
    fn test_build_api_request_without_system_prompt() {
        let mut session = ConversationSession::new("sess_5".into(), "gpt-4o".into());
        session.append_response(UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("Who are you?".into())],
            usage: UnifiedUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: Some(2),
                reasoning_tokens: None,
            },
            finish_reason: Some("stop".into()),
        });
        let req = session.build_api_request();
        assert!(!req.messages.is_empty());
        // No system prompt means no "system" role message
        assert!(!req.messages.iter().any(|m| m.role == "system"));
    }

    // ── Multiple turns ──────────────────────────────────────────────────────────

    #[test]
    fn test_conversation_session_multiple_turns() {
        let mut session = ConversationSession::new("sess_6".into(), "gpt-4o".into());

        // Turn 1: assistant response
        session.append_response(UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("First response".into())],
            usage: UnifiedUsage {
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: Some(3),
                reasoning_tokens: None,
            },
            finish_reason: Some("stop".into()),
        });
        assert_eq!(session.messages().len(), 1);
        assert_eq!(session.turn_count(), 0); // no tool call yet

        // Turn 2: tool call → increments turn
        session.append_tool_result("call_1".into(), "result A".into());
        assert_eq!(session.turn_count(), 1);

        // Turn 3: another assistant response
        session.append_response(UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("Second response".into())],
            usage: UnifiedUsage {
                prompt_tokens: 1,
                completion_tokens: 3,
                total_tokens: Some(4),
                reasoning_tokens: None,
            },
            finish_reason: Some("stop".into()),
        });
        assert_eq!(session.messages().len(), 2);
        assert_eq!(session.turn_count(), 1); // still 1; only tool_result increments

        // Turn 4: another tool call
        session.append_tool_result("call_2".into(), "result B".into());
        assert_eq!(session.turn_count(), 2);
        assert_eq!(session.messages().len(), 2); // no new message added by tool_result
    }
}
