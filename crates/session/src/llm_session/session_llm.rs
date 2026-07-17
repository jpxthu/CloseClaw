//! LLM invocation logic for `ConversationSession`.
//!
//! Provides [`ConversationSession::invoke_llm`] which encapsulates
//! the LLM call flow previously living in the Gateway layer
//! (`SessionMessageHandler::call_llm`). The session owns the
//! [`LlmCaller`] reference and the memory-injection consumption.

use closeclaw_common::LLMError;
use closeclaw_common::{
    split_static_dynamic, DynamicPromptContext, InternalMessage, InternalRequest, UnifiedResponse,
};

use super::streaming_assembly::SessionStream;
use super::ConversationSession;

impl ConversationSession {
    /// Inject a [`DynamicPromptBuilder`] for per-request dynamic-layer injection.
    pub fn set_dynamic_prompt_builder(
        &mut self,
        b: std::sync::Arc<dyn closeclaw_common::DynamicPromptBuilder>,
    ) {
        self.dynamic_prompt_builder = Some(b);
    }

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
    /// Returns a [`SessionStream`] that wraps the raw LLM event stream
    /// and accumulates [`ContentBlock`](closeclaw_common::ContentBlock)s
    /// as events pass through. After the stream is fully consumed,
    /// call [`SessionStream::into_content_blocks`] to extract the
    /// assembled result.
    ///
    /// The caller (Gateway) is responsible for consuming the stream
    /// for real-time rendering via
    /// [`Gateway::send_outbound_streaming`](crate::Gateway::send_outbound_streaming).
    pub async fn invoke_llm_streaming(&self, content: &str) -> Result<SessionStream, LLMError> {
        let Some(caller) = self.llm_caller.as_ref() else {
            return Err(LLMError::InvalidRequest(
                "no LlmCaller injected into session".to_string(),
            ));
        };

        let messages = self.build_llm_messages(content);
        let request = self.build_llm_request(messages, true);
        let raw_stream = caller.call_streaming(request).await?;
        Ok(SessionStream::new(raw_stream))
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
        let (system_static, system_dynamic) = self.build_system_prompt_parts(&messages);
        InternalRequest {
            model: String::new(),
            messages,
            temperature: 0.7,
            max_tokens: None,
            stream,
            extra_body: Default::default(),
            system_static,
            system_dynamic,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: self.reasoning_level,
            turn_count: None,
        }
    }

    /// Derive `system_static` and `system_dynamic` for the current request.
    ///
    /// When a [`DynamicPromptBuilder`](closeclaw_common::DynamicPromptBuilder)
    /// is injected, delegates to it for per-request dynamic-layer
    /// construction.  Otherwise falls back to the legacy behaviour
    /// (full prompt as `system_static`, no dynamic layer).
    fn build_system_prompt_parts(
        &self,
        messages: &[InternalMessage],
    ) -> (Option<String>, Option<String>) {
        if let Some(ref builder) = self.dynamic_prompt_builder {
            let ctx = self.request_context();
            let user_input = messages
                .iter()
                .rev()
                .find(|m| m.role == "user")
                .map(|m| m.content.as_str());
            let pending_transition = self.take_pending_mode_transition();
            let context = DynamicPromptContext {
                system_prompt: self.system_prompt.as_deref(),
                ctx: &ctx,
                workdir: &self.workdir,
                system_appends: &self.system_appends(),
                session_created_at: self.created_at,
                session_mode: self.session_mode(),
                overrides: self.prompt_overrides.as_ref(),
                user_input,
                pending_mode_transition: pending_transition,
                is_compacted: false,
                is_sub_agent: false,
            };
            builder.build_prompt_parts(&context)
        } else {
            // Legacy path: no builder injected — split the stored prompt
            // so static/dynamic separation still works for cache adapters.
            match &self.system_prompt {
                Some(prompt) => split_static_dynamic(prompt),
                None => (None, None),
            }
        }
    }
}
