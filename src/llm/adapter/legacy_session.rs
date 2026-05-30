//! Adapter that wraps legacy `Message` history into the new `ChatSession` trait.
//!
//! Use [`LegacySessionAdapter::from_legacy_messages`] to create an adapter from
//! a `Vec<Message>` (the old `LLMProvider` format), then interact with it
//! through the [`ChatSession`] trait.

use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::llm::session::{ChatSession, SessionMessage};
use crate::llm::turn::TurnCounter;
use crate::llm::types::{ContentBlock, InternalMessage, InternalRequest, UnifiedResponse};
use crate::llm::Message;
use crate::session::persistence::ReasoningLevel;

/// Adapter implementing [`ChatSession`] over a legacy message history.
///
/// Wraps the old `Vec<Message>` representation so that existing code can be
/// migrated to the new session trait incrementally.
#[derive(Debug)]
pub struct LegacySessionAdapter {
    messages: Vec<SessionMessage>,
    system_prompt: Option<String>,
    model: String,
    turn_counter: TurnCounter,
    is_llm_busy: Arc<AtomicBool>,
    reasoning_level: ReasoningLevel,
}

impl LegacySessionAdapter {
    /// Creates an adapter from a model name and a list of legacy `Message`s.
    ///
    /// Each legacy `Message` is converted into a `SessionMessage` with a
    /// single `ContentBlock::Text` block.
    pub fn from_legacy_messages(model: String, messages: Vec<Message>) -> Self {
        let session_messages = messages
            .into_iter()
            .map(|m| SessionMessage {
                role: m.role,
                content_blocks: vec![ContentBlock::Text(m.content)],
                timestamp: Utc::now(),
            })
            .collect();

        Self {
            messages: session_messages,
            system_prompt: None,
            model,
            turn_counter: TurnCounter::new(),
            is_llm_busy: Arc::new(AtomicBool::new(false)),
            reasoning_level: ReasoningLevel::default(),
        }
    }

    /// Sets the system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Appends a message to the internal list.
    fn push_message(&mut self, role: &str, content_blocks: Vec<ContentBlock>) {
        self.messages.push(SessionMessage {
            role: role.to_string(),
            content_blocks,
            timestamp: Utc::now(),
        });
    }
}

impl ChatSession for LegacySessionAdapter {
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
        if response.content_blocks.is_empty() {
            return;
        }
        self.push_message("assistant", response.content_blocks);
    }

    fn append_tool_result(&mut self, tool_call_id: String, result: String) {
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
                    ContentBlock::Thinking(t) => {
                        vec![format!("<thinking>{}</thinking>", t)]
                    }
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
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: self.reasoning_level,
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

    // ── from_legacy_messages ─────────────────────────────────────────────────

    #[test]
    fn test_from_legacy_messages_converts_correctly() {
        let legacy = vec![
            Message {
                role: "user".into(),
                content: "Hello".into(),
            },
            Message {
                role: "assistant".into(),
                content: "Hi there!".into(),
            },
        ];
        let adapter = LegacySessionAdapter::from_legacy_messages("gpt-4o".into(), legacy);

        assert_eq!(adapter.messages().len(), 2);
        assert_eq!(adapter.messages()[0].role, "user");
        assert_eq!(adapter.messages()[1].role, "assistant");
        assert_eq!(adapter.turn_count(), 0);
    }

    // ── with_system_prompt ──────────────────────────────────────────────────

    #[test]
    fn test_with_system_prompt_sets_prompt() {
        let adapter = LegacySessionAdapter::from_legacy_messages(
            "gpt-4o".into(),
            vec![Message {
                role: "user".into(),
                content: "Hi".into(),
            }],
        )
        .with_system_prompt("You are helpful.");

        assert_eq!(adapter.system_prompt(), Some("You are helpful."));
    }

    // ── append_response ──────────────────────────────────────────────────────

    #[test]
    fn test_append_response_adds_message() {
        let mut adapter = LegacySessionAdapter::from_legacy_messages("gpt-4o".into(), vec![]);

        adapter.append_response(UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("Hello!".into())],
            usage: UnifiedUsage {
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: Some(3),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: Some("stop".into()),
        });

        assert_eq!(adapter.messages().len(), 1);
        assert_eq!(adapter.messages()[0].role, "assistant");
    }

    // ── append_response empty does NOT add message ───────────────────────────

    #[test]
    fn test_append_response_empty_does_nothing() {
        let mut adapter = LegacySessionAdapter::from_legacy_messages("gpt-4o".into(), vec![]);

        adapter.append_response(UnifiedResponse {
            content_blocks: vec![],
            usage: UnifiedUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: Some(0),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        });

        assert_eq!(adapter.messages().len(), 0);
        assert_eq!(adapter.turn_count(), 0);
    }

    // ── append_tool_result increments turn ──────────────────────────────────

    #[test]
    fn test_append_tool_result_increments_turn() {
        let mut adapter = LegacySessionAdapter::from_legacy_messages("gpt-4o".into(), vec![]);

        adapter.append_response(UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("Using tool...".into())],
            usage: UnifiedUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: Some(2),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: Some("stop".into()),
        });
        assert_eq!(adapter.turn_count(), 0);

        adapter.append_tool_result("call_x".into(), "tool output".into());
        assert_eq!(adapter.turn_count(), 1);
    }

    // ── build_api_request with system prompt ────────────────────────────────

    #[test]
    fn test_build_api_request_includes_system_prompt() {
        let adapter = LegacySessionAdapter::from_legacy_messages(
            "gpt-4o".into(),
            vec![Message {
                role: "user".into(),
                content: "Hi".into(),
            }],
        )
        .with_system_prompt("You are a chatbot.");

        let req = adapter.build_api_request();
        assert!(req
            .messages
            .iter()
            .any(|m| m.role == "system" && m.content.contains("chatbot")));
    }

    // ── build_api_request without system prompt ────────────────────────────

    #[test]
    fn test_build_api_request_no_system_message_when_not_set() {
        let adapter = LegacySessionAdapter::from_legacy_messages(
            "gpt-4o".into(),
            vec![Message {
                role: "user".into(),
                content: "Hi".into(),
            }],
        );

        let req = adapter.build_api_request();
        assert!(!req.messages.iter().any(|m| m.role == "system"));
    }
}
