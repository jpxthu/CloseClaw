//! LLM invocation logic for `ConversationSession`.
//!
//! Provides [`ConversationSession::invoke_llm`] which encapsulates
//! the LLM call flow previously living in the Gateway layer
//! (`SessionMessageHandler::call_llm`). The session owns the
//! [`LlmCaller`] reference and the memory-injection consumption.

use std::pin::Pin;

use futures::Stream;

use closeclaw_common::processor::StreamEvent;
use closeclaw_common::LLMError;
use closeclaw_common::{InternalMessage, InternalRequest, UnifiedResponse};

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

        let messages = self.build_llm_messages(content);
        let request = self.build_llm_request(messages, false);
        caller.call(request).await
    }

    /// Make a streaming LLM call via the injected [`LlmCaller`].
    ///
    /// Returns a raw event stream. The caller (Gateway) is responsible
    /// for wrapping with [`SinkUpdater`], racing
    /// against a cancellation token, and dispatching through
    /// [`Gateway::send_outbound_streaming`].
    pub async fn invoke_llm_streaming(
        &self,
        content: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>, LLMError> {
        let Some(caller) = self.llm_caller.as_ref() else {
            return Err(LLMError::InvalidRequest(
                "no LlmCaller injected into session".to_string(),
            ));
        };

        let messages = self.build_llm_messages(content);
        let request = self.build_llm_request(messages, true);
        caller.call_streaming(request).await
    }

    /// Build the messages list for an LLM request, consuming any
    /// pending memory-injection slot.
    fn build_llm_messages(&self, content: &str) -> Vec<InternalMessage> {
        let mut messages = vec![InternalMessage {
            role: "user".to_string(),
            content: content.to_string(),
            tool_call_id: None,
        }];

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

        messages
    }

    /// Build an [`InternalRequest`] from a pre-built messages list.
    fn build_llm_request(&self, messages: Vec<InternalMessage>, stream: bool) -> InternalRequest {
        InternalRequest {
            model: String::new(),
            messages,
            temperature: 0.7,
            max_tokens: None,
            stream,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: self.reasoning_level,
            turn_count: None,
        }
    }
}
