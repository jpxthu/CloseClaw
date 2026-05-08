//! Session layer for LLM conversations.
//!
//! Provides `SessionMessage`, `ChatSession` trait and `ConversationSession`
//! for managing conversation state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
}

/// A simple in-memory implementation of `ChatSession`.
#[derive(Debug, Clone)]
pub struct ConversationSession {
    session_id: String,
    messages: Vec<SessionMessage>,
    system_prompt: Option<String>,
    turn_counter: TurnCounter,
    model: String,
    compaction_state: Option<String>,
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
        let mut msgs = Vec::with_capacity(self.messages.len());
        for msg in &self.messages {
            let content = msg
                .content_blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text(t) => Some(t.clone()),
                    ContentBlock::Thinking(t) => Some(format!("<thinking>{}</thinking>", t)),
                    ContentBlock::ToolUse { name, input, .. } => {
                        Some(format!("[tool:{}] {}", name, input))
                    }
                    ContentBlock::ToolResult { content, .. } => Some(content.clone()),
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
}
