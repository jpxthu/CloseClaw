//! `ChatSession` trait and its `ConversationSession` implementation.
//!
//! This is the public session API used by the rest of the project
//! (e.g. `legacy_session`, the gateway, integration tests). It was
//! extracted from the old monolithic `src/llm/session.rs` in
//! Step 1.8 of issue #858 so the file could stay under the
//! CONTRIBUTING.md 500-line hard cap.
//!
//! The associated function [`ConversationSession::clean_thinking_content`]
//! is also kept here because it is a private helper of
//! [`ConversationSession::build_api_request`] (the only `ChatSession`
//! method that needs it) and is exercised by
//! `src/llm/session/tests/thinking_clean_tests.rs`.

use std::sync::Arc;

use super::{ConversationSession, SessionMessage};
use crate::llm::session_state::SessionExecStatus;
use crate::llm::streaming::StreamingSink;
use crate::llm::types::{ContentBlock, InternalMessage, InternalRequest, UnifiedResponse};

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

    /// Sets the streaming sink for this session.
    fn set_streaming_sink(&mut self, sink: Arc<dyn StreamingSink>);

    /// Returns whether streaming is enabled for this session.
    fn stream_enabled(&self) -> bool;

    /// Enables or disables streaming for this session.
    fn set_stream_enabled(&mut self, enabled: bool);

    /// Returns whether the LLM is currently busy (processing a request).
    fn is_llm_busy(&self) -> bool;
}

impl ConversationSession {
    /// Cleans thinking-only artifacts from a message list.
    ///
    /// Two passes:
    /// 1. Remove assistant messages whose content blocks are *all* Thinking
    ///    (orphaned thinking).
    /// 2. Trim trailing Thinking blocks from the last assistant message;
    ///    if the result is empty, insert an empty Text placeholder.
    pub(crate) fn clean_thinking_content(messages: &[SessionMessage]) -> Vec<SessionMessage> {
        // Pass 1: filter out orphaned thinking messages.
        let mut cleaned: Vec<SessionMessage> = messages
            .iter()
            .filter(|msg| {
                if msg.role == "assistant" {
                    !msg.content_blocks
                        .iter()
                        .all(|b| matches!(b, ContentBlock::Thinking(_)))
                } else {
                    true
                }
            })
            .cloned()
            .collect();

        // Pass 2: trim trailing Thinking blocks from the last assistant message.
        if let Some(last_assistant) = cleaned.iter_mut().rev().find(|m| m.role == "assistant") {
            while last_assistant
                .content_blocks
                .last()
                .is_some_and(|b| matches!(b, ContentBlock::Thinking(_)))
            {
                last_assistant.content_blocks.pop();
            }
            if last_assistant.content_blocks.is_empty() {
                last_assistant
                    .content_blocks
                    .push(ContentBlock::Text(String::new()));
            }
        }

        cleaned
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
        let cleaned = Self::clean_thinking_content(&self.messages);
        for msg in &cleaned {
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
            stream: self.stream_enabled,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: self.reasoning_level,
        }
    }

    fn set_streaming_sink(&mut self, sink: Arc<dyn StreamingSink>) {
        self.streaming_sink = Some(sink);
    }

    fn stream_enabled(&self) -> bool {
        self.stream_enabled
    }

    fn set_stream_enabled(&mut self, enabled: bool) {
        self.stream_enabled = enabled;
    }

    fn is_llm_busy(&self) -> bool {
        // Delegate to the three-dimensional execution state model.
        // Preserves the legacy "LLM or foreground tool active" semantics.
        self.exec_status() == SessionExecStatus::Busy
    }
}
