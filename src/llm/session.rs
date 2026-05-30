//! Session layer for LLM conversations.
//!
//! Provides `SessionMessage`, `ChatSession` trait and `ConversationSession`
//! for managing conversation state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::llm::turn::TurnCounter;
use crate::llm::types::{ContentBlock, InternalMessage, InternalRequest, UnifiedResponse};
use crate::session::persistence::ReasoningLevel;

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
    reasoning_level: ReasoningLevel,
    workdir: PathBuf,
}

impl ConversationSession {
    /// Creates a new session with the given model and working directory.
    pub fn new(session_id: String, model: String, workdir: PathBuf) -> Self {
        Self {
            session_id,
            messages: Vec::new(),
            system_prompt: None,
            turn_counter: TurnCounter::new(),
            model,
            compaction_state: None,
            is_llm_busy: Arc::new(AtomicBool::new(false)),
            pending_messages: VecDeque::new(),
            reasoning_level: ReasoningLevel::default(),
            workdir,
        }
    }

    /// Returns the current working directory.
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// Sets the working directory.
    pub fn set_workdir(&mut self, path: PathBuf) {
        self.workdir = path;
    }

    /// Sets the system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Sets the reasoning level.
    pub fn with_reasoning_level(mut self, level: ReasoningLevel) -> Self {
        self.reasoning_level = level;
        self
    }

    /// Replace the system prompt on an existing session.
    /// Used by `SessionManager::rebuild_system_prompt` after compaction.
    pub fn replace_system_prompt(&mut self, prompt: impl Into<String>) {
        self.system_prompt = Some(prompt.into());
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

    /// Returns the model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Replaces all session messages with the given list.
    pub fn replace_messages(&mut self, new_messages: Vec<SessionMessage>) {
        self.messages = new_messages;
    }

    /// Returns a clone of all pending messages without consuming the queue.
    pub fn get_pending_messages(&self) -> Vec<crate::session::persistence::PendingMessage> {
        self.pending_messages.iter().cloned().collect()
    }

    /// Restores pending messages from checkpoint data.
    /// Only pushes messages where `sent == false` back into the queue.
    pub fn restore_pending_messages(
        &mut self,
        messages: Vec<crate::session::persistence::PendingMessage>,
    ) {
        for msg in messages {
            if !msg.sent {
                self.pending_messages.push_back(msg);
            }
        }
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
            reasoning_level: self.reasoning_level,
        }
    }

    fn is_llm_busy(&self) -> bool {
        self.is_llm_busy.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests;
