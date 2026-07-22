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

    /// Mark this session as compacted so that sparse prompt variants
    /// are injected on subsequent LLM calls.
    pub fn mark_compacted(&mut self) {
        self.is_compacted = true;
    }

    /// Returns whether this session has been compacted.
    pub fn is_compacted(&self) -> bool {
        self.is_compacted
    }

    /// Mark this session as a sub-agent so that the sub-agent
    /// sparse prompt variant is injected on subsequent LLM calls.
    pub fn set_sub_agent(&mut self, is_sub_agent: bool) {
        self.is_sub_agent = is_sub_agent;
    }

    /// Returns whether this session is a sub-agent.
    pub fn is_sub_agent(&self) -> bool {
        self.is_sub_agent
    }

    /// Prepare the skill listing for the current turn.
    ///
    /// Extracts file paths from the user message, finds new
    /// conditional matches, computes the incremental listing using only
    /// the currently activated skills (newly activated skills are
    /// applied AFTER this turn via [`apply_skill_listing_update`]),
    /// and returns the listing to inject plus the updated state for
    /// the caller to apply.
    ///
    /// Returns `(listing, new_snapshot, newly_activated_names)`.
    fn prepare_turn_skill_listing(
        &self,
        content: &str,
    ) -> (
        Option<String>,
        Option<String>,
        std::collections::HashSet<String>,
    ) {
        // 1. Extract file paths from user content and find newly
        //    activated conditionals.
        let paths = Self::extract_file_paths(content);
        let mut newly_activated = std::collections::HashSet::new();
        if !paths.is_empty() {
            if let Some(provider) = self.skill_listing_provider.as_ref() {
                let matches = provider.find_conditional_matches(&paths);
                for m in matches {
                    if !self.activated_conditional_skills.contains(&m.name) {
                        newly_activated.insert(m.name);
                    }
                }
            }
        }

        // 2. Compute listing using ONLY current activation set
        //    (newly activated skills are applied after this turn)
        let (listing, new_snapshot) = self.compute_skill_listing_for_turn();

        (listing, new_snapshot, newly_activated)
    }

    /// Make a non-streaming LLM call via the injected [`LlmCaller`].
    ///
    /// Builds an [`InternalRequest`], consuming any pending
    /// memory-injection slot, and delegates to the caller. Returns
    /// an error if no [`LlmCaller`] has been injected.
    pub async fn invoke_llm(&mut self, content: &str) -> Result<UnifiedResponse, LLMError> {
        let Some(caller) = self.llm_caller.clone() else {
            return Err(LLMError::InvalidRequest(
                "no LlmCaller injected into session".to_string(),
            ));
        };

        let (listing, new_snapshot, newly_activated) = self.prepare_turn_skill_listing(content);
        let messages = self.build_llm_messages_with_listing(content, listing);
        self.apply_skill_listing_update(new_snapshot, &newly_activated);

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
    pub async fn invoke_llm_streaming(&mut self, content: &str) -> Result<SessionStream, LLMError> {
        let Some(caller) = self.llm_caller.clone() else {
            return Err(LLMError::InvalidRequest(
                "no LlmCaller injected into session".to_string(),
            ));
        };

        let (listing, new_snapshot, newly_activated) = self.prepare_turn_skill_listing(content);
        let messages = self.build_llm_messages_with_listing(content, listing);
        self.apply_skill_listing_update(new_snapshot, &newly_activated);

        let request = self.build_llm_request(messages, true);
        let raw_stream = caller.call_streaming(request).await?;
        Ok(SessionStream::new(raw_stream))
    }

    /// Build the messages list for an LLM request, consuming any
    /// pending memory-injection slot.
    ///
    /// Message assembly order:
    /// 1. Skill listing attachment (tool role, position 0) — per-turn
    ///    incremental diff from the [`SkillListingProvider`] when
    ///    non-empty. Prepared by [`prepare_turn_skill_listing`].
    /// 2. Memory injection (tool role) — positioned per
    ///    [`InjectionPosition::AfterCurrent`] or `BeforeNext`.
    /// 3. User message.
    ///
    /// `skill_listing` is the pre-computed listing content to inject.
    /// Pass `None` to skip skill listing injection.
    fn build_llm_messages_with_listing(
        &self,
        content: &str,
        skill_listing: Option<String>,
    ) -> Vec<InternalMessage> {
        let mut messages = vec![InternalMessage {
            role: "user".to_string(),
            content: content.to_string(),
            tool_call_id: None,
        }];

        // 1. Skill listing attachment — at position 0 when non-empty.
        let skill_listing_inserted = if let Some(listing) = skill_listing {
            if !listing.is_empty() {
                messages.insert(
                    0,
                    InternalMessage {
                        role: "tool".to_string(),
                        content: listing,
                        tool_call_id: None,
                    },
                );
                true
            } else {
                false
            }
        } else {
            false
        };

        // 2. Memory injection — positioned per InjectionPosition.
        if let Some(injection) = self.take_memory_injection() {
            let tool_msg = InternalMessage {
                role: "tool".to_string(),
                content: injection.content.clone(),
                tool_call_id: None,
            };
            match injection.position_mode {
                super::InjectionPosition::AfterCurrent => {
                    // AfterCurrent means after the current (user)
                    // message, so push to the end.
                    messages.push(tool_msg);
                }
                super::InjectionPosition::BeforeNext => {
                    // Insert before user message. Skill listing
                    // occupies position 0 (if present), user message
                    // is at the end. Insert at position 1 (after skill
                    // listing) or at 0 (before user message, no skill
                    // listing).
                    let insert_pos = if skill_listing_inserted { 1 } else { 0 };
                    messages.insert(insert_pos, tool_msg);
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

    /// Derive `system_static` and `system_dynamic` for the current
    /// request.
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
                is_compacted: self.is_compacted,
                is_sub_agent: self.is_sub_agent,
            };
            builder.build_prompt_parts(&context)
        } else {
            // Legacy path: no builder injected — split the stored
            // prompt so static/dynamic separation still works for
            // cache adapters.
            match &self.system_prompt {
                Some(prompt) => split_static_dynamic(prompt),
                None => (None, None),
            }
        }
    }
}
