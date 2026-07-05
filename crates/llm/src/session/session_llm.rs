//! LLM invocation logic for `ConversationSession`.
//!
//! Provides [`ConversationSession::invoke_llm`] which encapsulates
//! the LLM call flow previously living in the Gateway layer
//! (`SessionMessageHandler::call_llm`). The session owns the
//! [`LlmCaller`] reference and the memory-injection consumption.

use crate::types::{InternalMessage, InternalRequest, UnifiedResponse};
use closeclaw_common::LLMError;
use closeclaw_session::persistence::ReasoningLevel;

use super::ConversationSession;

impl ConversationSession {
    /// Make a non-streaming LLM call via the injected [`LlmCaller`].
    ///
    /// Builds an [`InternalRequest`], consuming any pending
    /// memory-injection slot, and delegates to the caller. Returns
    /// an error if no [`LlmCaller`] has been injected.
    pub async fn invoke_llm(&self, content: &str) -> Result<UnifiedResponse, LLMError> {
        let Some(caller) = self.llm_caller.as_ref() else {
            return Err(LLMError::InvalidRequest(
                "no LlmCaller injected into session".to_string(),
            ));
        };

        let mut messages = vec![InternalMessage {
            role: "user".to_string(),
            content: content.to_string(),
            tool_call_id: None,
        }];

        // Consume memory_injection slot if present.
        if let Some(injection) = self.take_memory_injection() {
            let tool_msg = InternalMessage {
                role: "tool".to_string(),
                content: injection.content.clone(),
                tool_call_id: None,
            };
            match injection.position_mode {
                super::InjectionPosition::AfterCurrent => {
                    messages.push(tool_msg);
                }
                super::InjectionPosition::BeforeNext => {
                    messages.insert(0, tool_msg);
                }
            }
        }

        let request = InternalRequest {
            model: String::new(),
            messages,
            temperature: 0.7,
            max_tokens: None,
            stream: false,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
            turn_count: None,
        };

        caller.call(request).await
    }
}
