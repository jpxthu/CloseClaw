//! LLM invocation logic for `ConversationSession`.
//!
//! Provides [`invoke_llm`](ConversationSession::invoke_llm) which encapsulates
//! the LLM call flow previously living in the Gateway layer
//! (`SessionMessageHandler::call_llm`). The session owns the
//! [`LlmCaller`] reference and the memory-injection consumption.
//!
//! # Where things live
//!
//! The invoke entry points and the shared invocation cores are in
//! this file; session state flags and accessors are in the first
//! `impl` block below. Consumed by the invoke family in this file:
//! per-turn skill listing preparation in
//! `super::session_llm_skill_prep` and message assembly / request
//! building in `super::session_llm_request_build`.

use closeclaw_common::{CallTarget, InternalRequest, LLMError, LlmCaller, UnifiedResponse};

use super::streaming_assembly::SessionStream;
use super::ConversationSession;

/// Session state flags and accessors: compaction / sub-agent
/// markers, mode-transition slot, and plan file path.
impl ConversationSession {
    /// Consume and return the pending mode transition, if any.
    pub fn take_mode_transition(&self) -> Option<closeclaw_common::system_prompt::ModeTransition> {
        self.pending_mode_transition
            .lock()
            .expect("pending_mode_transition lock poisoned")
            .take()
    }

    /// Inject a [`DynamicPromptBuilder`](closeclaw_common::DynamicPromptBuilder)
    /// for per-request dynamic-layer injection.
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
        self.pending_compaction_listing_reset = true;
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

    /// Returns the plan file path associated with this session, if any.
    pub fn plan_file_path(&self) -> Option<&str> {
        self.plan_file_path.as_deref()
    }

    /// Sets the plan file path for this session.
    pub fn set_plan_file_path(&mut self, path: Option<String>) {
        self.plan_file_path = path
    }
}

/// Non-streaming LLM invocation entry points.
impl ConversationSession {
    /// Make a non-streaming LLM call via the injected
    /// [`LlmCaller`].
    ///
    /// Test-only convenience wrapper (Step 1.8 ③): production call sites
    /// must use [`invoke_llm_with_target`](Self::invoke_llm_with_target) so the unified
    /// advancement point receives the actually-used call target; this
    /// wrapper discards it. Kept as a thin delegation because the test
    /// call surface is wide (100+ sites across session/gateway test
    /// modules) and migrating adds churn without behavior change.
    pub async fn invoke_llm(&mut self, content: &str) -> Result<UnifiedResponse, LLMError> {
        self.invoke_llm_with_target(content)
            .await
            .map(|(response, _)| response)
    }

    /// Make a non-streaming LLM call via the injected
    /// [`LlmCaller`] and
    /// return the actually-used [`CallTarget`] with the response.
    ///
    /// Corresponds to the design doc's injection flow: prepares the
    /// skill listing via `Self::prepare_turn_skill_listing`, injects it as
    /// the instruction block via `Self::build_llm_messages_with_listing`,
    /// then delegates to the LLM caller.
    ///
    /// Builds an [`InternalRequest`],
    /// consuming any pending
    /// memory-injection slot, and delegates to the caller. Returns
    /// an error if no [`LlmCaller`] has been injected.
    ///
    /// The target (when the caller reports one) travels with this call's
    /// return value — no shared state — so concurrent sessions can never
    /// observe another session's target; `None` (test fakes / stub paths)
    /// degrades to session-config semantics.
    pub async fn invoke_llm_with_target(
        &mut self,
        content: &str,
    ) -> Result<(UnifiedResponse, Option<CallTarget>), LLMError> {
        self.invoke_llm_with_target_inner(content, false).await
    }

    /// Pre-appended variant of [`invoke_llm_with_target`](Self::invoke_llm_with_target).
    ///
    /// Declares that the current user message was already appended to
    /// history before this invoke — in plain text form or in structured
    /// blocks form (embedded small-image messages). The declaration
    /// (`current_turn_pre_appended`) lets the build-side push dedup
    /// guard recognize the already-appended message and skip the
    /// duplicate push: plain-text tails match by string equality, while
    /// blocks-form tails flatten to `content=""` (Image blocks are
    /// excluded from the text flatten, so string equality with the
    /// gateway-flattened `[image: key] …\nbase64` invoke content is
    /// structurally unreachable) and are matched by the declaration
    /// branch. Skipping the push keeps the sent list a tail-append
    /// extension of the previous round.
    pub async fn invoke_llm_with_target_pre_appended_blocks(
        &mut self,
        content: &str,
    ) -> Result<(UnifiedResponse, Option<CallTarget>), LLMError> {
        self.invoke_llm_with_target_inner(content, true).await
    }
}

/// Non-streaming invocation core (shared by the entry points above).
impl ConversationSession {
    async fn invoke_llm_with_target_inner(
        &mut self,
        content: &str,
        current_turn_pre_appended: bool,
    ) -> Result<(UnifiedResponse, Option<CallTarget>), LLMError> {
        // ── Shutdown gate: reject LLM calls when daemon is shutting down ──
        if let Some(sh) = self.get_shutdown_handle() {
            if sh.is_shutting_down() {
                tracing::warn!(
                    session_id = %self.session_id,
                    "rejecting non-streaming LLM call: daemon is shutting down"
                );
                return Err(LLMError::Cancelled);
            }
        }

        let (caller, request) =
            self.prepare_llm_turn_request(content, current_turn_pre_appended, false)?;

        // ── Busy count: increment before LLM call, decrement after ──
        if let Some(sh) = self.get_shutdown_handle() {
            sh.increment_busy();
        }
        let result = caller.call_with_target(request).await;
        if let Some(sh) = self.get_shutdown_handle() {
            sh.decrement_busy();
        }
        result
    }
}

/// Shared request preparation for the invocation cores above: caller
/// extraction → per-turn skill listing preparation → message
/// assembly → [`InternalRequest`] construction (`is_streaming`
/// selects the streaming variant) → cache prefix snapshot staging.
///
/// The shutdown gate and busy-count ownership stay with each core —
/// their log wording and busy return timing differ.
impl ConversationSession {
    fn prepare_llm_turn_request(
        &mut self,
        content: &str,
        current_turn_pre_appended: bool,
        is_streaming: bool,
    ) -> Result<(std::sync::Arc<dyn LlmCaller>, InternalRequest), LLMError> {
        let Some(caller) = self.llm_caller.clone() else {
            return Err(LLMError::InvalidRequest(
                "no LlmCaller injected into session".to_string(),
            ));
        };

        let listing = self.prepare_turn_skill_listing(content);
        let messages =
            self.build_llm_messages_with_listing(content, listing, current_turn_pre_appended);

        let request = self.build_llm_request(messages, is_streaming);

        // Stage this round's prefix snapshot at the request-build
        // completion point (cache consistency check; consumed at the
        // unified advancement point once usage fully arrives).
        self.stage_cache_prefix_snapshot(
            request.system_static.as_deref(),
            request.system_dynamic.as_deref(),
            request.tools.as_deref(),
            &request.messages,
        );

        Ok((caller, request))
    }
}

/// Streaming LLM invocation entry points.
impl ConversationSession {
    /// Make a streaming LLM call via the injected
    /// [`LlmCaller`].
    pub async fn invoke_llm_streaming(&mut self, content: &str) -> Result<SessionStream, LLMError> {
        self.invoke_llm_streaming_with_target(content)
            .await
            .map(|(stream, _)| stream)
    }

    /// Make a streaming LLM call via the injected
    /// [`LlmCaller`] and return
    /// the actually-used [`CallTarget`] with the stream.
    ///
    /// Corresponds to the design doc's injection flow: prepares the
    /// skill listing via `Self::prepare_turn_skill_listing`, injects it as
    /// the instruction block via `Self::build_llm_messages_with_listing`,
    /// then delegates to the LLM caller.
    ///
    /// Returns a [`SessionStream`] that wraps the raw LLM event stream
    /// and accumulates
    /// [`ContentBlock`](closeclaw_common::ContentBlock)s
    /// as events pass through. After the stream is fully consumed,
    /// call [`into_content_blocks`](SessionStream::into_content_blocks) to extract the
    /// assembled result.
    ///
    /// The target (when the caller reports one) travels with this call's
    /// return value — no shared state — so concurrent sessions can never
    /// observe another session's target; `None` (test fakes / stub paths)
    /// degrades to session-config semantics.
    ///
    /// The caller (Gateway) is responsible for consuming the stream
    /// for real-time rendering via `Gateway::send_outbound_streaming`.
    pub async fn invoke_llm_streaming_with_target(
        &mut self,
        content: &str,
    ) -> Result<(SessionStream, Option<CallTarget>), LLMError> {
        self.invoke_llm_streaming_with_target_inner(content, false)
            .await
    }

    /// Pre-appended variant of
    /// [`invoke_llm_streaming_with_target`](Self::invoke_llm_streaming_with_target).
    ///
    /// Same explicit declaration as
    /// [`invoke_llm_with_target_pre_appended_blocks`][pre_appended_blocks], for
    /// call paths where the current user message was already appended
    /// to history before the invoke (plain text or structured blocks
    /// form), which the build-side push dedup guard must be told about.
    ///
    /// [pre_appended_blocks]: Self::invoke_llm_with_target_pre_appended_blocks
    pub async fn invoke_llm_streaming_with_target_pre_appended_blocks(
        &mut self,
        content: &str,
    ) -> Result<(SessionStream, Option<CallTarget>), LLMError> {
        self.invoke_llm_streaming_with_target_inner(content, true)
            .await
    }
}

/// Streaming invocation core (shared by the entry points above).
impl ConversationSession {
    async fn invoke_llm_streaming_with_target_inner(
        &mut self,
        content: &str,
        current_turn_pre_appended: bool,
    ) -> Result<(SessionStream, Option<CallTarget>), LLMError> {
        // ── Shutdown gate: reject LLM streaming calls when daemon is shutting down ──
        if let Some(sh) = self.get_shutdown_handle() {
            if sh.is_shutting_down() {
                tracing::warn!(
                    session_id = %self.session_id,
                    "rejecting streaming LLM call: daemon is shutting down"
                );
                return Err(LLMError::Cancelled);
            }
        }

        let (caller, request) =
            self.prepare_llm_turn_request(content, current_turn_pre_appended, true)?;

        // ── Busy count: increment before stream, decrement on stream end ──
        if let Some(sh) = self.get_shutdown_handle() {
            sh.increment_busy();
        }
        let (raw_stream, call_target) = match caller.call_streaming_with_target(request).await {
            Ok(s) => s,
            Err(e) => {
                // Stream creation failed — decrement busy count immediately.
                if let Some(sh) = self.get_shutdown_handle() {
                    sh.decrement_busy();
                }
                return Err(e);
            }
        };
        let stream = SessionStream::new(raw_stream);

        // Attach shutdown handle so SessionStream decrements busy count
        // when the stream finishes or errors.
        Ok(match self.get_shutdown_handle() {
            Some(sh) => (stream.with_shutdown_handle(sh), call_target),
            None => (stream, call_target),
        })
    }
}
