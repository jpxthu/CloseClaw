//! LLM invocation logic for `ConversationSession`.
//!
//! Provides [`ConversationSession::invoke_llm`] which encapsulates
//! the LLM call flow previously living in the Gateway layer
//! (`SessionMessageHandler::call_llm`). The session owns the
//! [`LlmCaller`] reference and the memory-injection consumption.

use closeclaw_common::LLMError;
use closeclaw_common::{
    split_static_dynamic, ContentBlock, DynamicPromptContext, InternalMessage, InternalRequest,
    UnifiedResponse,
};

use super::streaming_assembly::SessionStream;
use super::{ConversationSession, SessionMessage};

/// Format a single [`ContentBlock`] into zero or more string fragments.
///
/// This is the shared formatting logic used by both the legacy
/// `build_api_request` path and the production `build_llm_messages_with_listing`
/// path. Tool results are excluded here (they are appended as
/// independent `role="tool"` messages by the caller).
pub(crate) fn format_content_block(b: &ContentBlock) -> Vec<String> {
    match b {
        ContentBlock::Text(t) => vec![t.clone()],
        ContentBlock::Thinking { thinking: t, .. } => {
            vec![format!("<thinking>{}</thinking>", t)]
        }
        ContentBlock::ToolUse { name, input, .. } => {
            vec![format!("[tool:{}] {}", name, input)]
        }
        ContentBlock::Image { name, .. } => vec![format!("[image: {}]", name)],
        ContentBlock::Audio { name, .. } => vec![format!("[audio: {}]", name)],
        ContentBlock::File { name, .. } => vec![format!("[file: {}]", name)],
        ContentBlock::ToolResult { .. } => vec![],
    }
}

impl ConversationSession {
    /// Consume and return the pending mode transition, if any.
    pub fn take_mode_transition(&self) -> Option<closeclaw_common::system_prompt::ModeTransition> {
        self.pending_mode_transition
            .lock()
            .expect("pending_mode_transition lock poisoned")
            .take()
    }

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

    /// Prepare the skill listing for the current turn.
    ///
    /// Corresponds to the design doc's "增量更新" section
    /// (`docs/design/skills/skill-listing-injection.md`), which
    /// specifies the processing order: "先更新文件变更引起的增量，
    /// 再处理条件激活的增量" (first update increments caused by
    /// file changes, then process conditional activation increments).
    ///
    /// This function handles the conditional activation step:
    /// extracts file paths from the user message, finds new
    /// conditional matches, computes the incremental listing using
    /// only the currently activated skills (newly activated skills
    /// are applied AFTER this turn via [`apply_skill_listing_update`]),
    /// and returns the listing to inject plus the updated state for
    /// the caller to apply.
    ///
    /// The ordering is guaranteed by the daemon's file listener,
    /// which completes cache invalidation and re-scan *before* this
    /// turn executes (see design doc's "文件监听与热重载" section).
    /// The incremental diff in [`compute_skill_listing_for_turn`]
    /// then naturally captures both file-change increments and
    /// conditional activation increments in the correct order.
    ///
    /// Returns `(listing, new_snapshot, newly_activated_names)`.
    fn prepare_turn_skill_listing(
        &mut self,
        content: &str,
    ) -> (
        Option<String>,
        Option<String>,
        std::collections::HashSet<String>,
    ) {
        // Compaction detection: if the session was compacted, clear
        // the snapshot so that compute_skill_listing_for_turn enters
        // the "first turn" branch and injects the full listing.
        // After injection, apply_skill_listing_update sets a new
        // snapshot, restoring the normal incremental diff path.
        if self.pending_compaction_listing_reset && self.skill_listing_snapshot.is_some() {
            tracing::debug!(
                session_id = %self.session_id,
                "prepare_turn_skill_listing: clearing snapshot after compaction"
            );
            self.skill_listing_snapshot = None;
            self.pending_compaction_listing_reset = false;
        }

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
        if !newly_activated.is_empty() {
            tracing::info!(
                session_id = %self.session_id,
                event = "conditional_skill_activation",
                activated = ?newly_activated.iter().collect::<Vec<_>>(),
                "conditionally activated skills for current turn"
            );
        }

        // 2. Compute listing using ONLY current activation set
        //    (newly activated skills are applied after this turn)
        let (listing, new_snapshot) = self.compute_skill_listing_for_turn();

        (listing, new_snapshot, newly_activated)
    }

    /// Make a non-streaming LLM call via the injected [`LlmCaller`].
    ///
    /// Corresponds to the design doc's injection flow: prepares the
    /// skill listing via [`prepare_turn_skill_listing`], injects it as
    /// the instruction block via [`build_llm_messages_with_listing`],
    /// then delegates to the LLM caller.
    ///
    /// Builds an [`InternalRequest`], consuming any pending
    /// memory-injection slot, and delegates to the caller. Returns
    /// an error if no [`LlmCaller`] has been injected.
    pub async fn invoke_llm(&mut self, content: &str) -> Result<UnifiedResponse, LLMError> {
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

        let Some(caller) = self.llm_caller.clone() else {
            return Err(LLMError::InvalidRequest(
                "no LlmCaller injected into session".to_string(),
            ));
        };

        let (listing, new_snapshot, newly_activated) = self.prepare_turn_skill_listing(content);
        let messages = self.build_llm_messages_with_listing(content, listing);
        self.apply_skill_listing_update(new_snapshot, &newly_activated);

        let request = self.build_llm_request(messages, false);

        // ── Busy count: increment before LLM call, decrement after ──
        if let Some(sh) = self.get_shutdown_handle() {
            sh.increment_busy();
        }
        let result = caller.call(request).await;
        if let Some(sh) = self.get_shutdown_handle() {
            sh.decrement_busy();
        }
        result
    }

    /// Make a streaming LLM call via the injected [`LlmCaller`].
    ///
    /// Corresponds to the design doc's injection flow: prepares the
    /// skill listing via [`prepare_turn_skill_listing`], injects it as
    /// the instruction block via [`build_llm_messages_with_listing`],
    /// then delegates to the LLM caller.
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

        let Some(caller) = self.llm_caller.clone() else {
            return Err(LLMError::InvalidRequest(
                "no LlmCaller injected into session".to_string(),
            ));
        };

        let (listing, new_snapshot, newly_activated) = self.prepare_turn_skill_listing(content);
        let messages = self.build_llm_messages_with_listing(content, listing);
        self.apply_skill_listing_update(new_snapshot, &newly_activated);

        let request = self.build_llm_request(messages, true);

        // ── Busy count: increment before stream, decrement on stream end ──
        if let Some(sh) = self.get_shutdown_handle() {
            sh.increment_busy();
        }
        let raw_stream = match caller.call_streaming(request).await {
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
            Some(sh) => stream.with_shutdown_handle(sh),
            None => stream,
        })
    }

    /// Build the messages list for an LLM request, consuming any
    /// pending memory-injection slot.
    ///
    /// Corresponds to the design doc's "注入当前 turn 的 instruction
    /// block" section (`docs/design/skills/skill-listing-injection.md`).
    /// The skill listing is injected as a system-role message at position 0,
    /// which is the code-level implementation of the design doc's
    /// "instruction block" injection.
    ///
    /// Message assembly order:
    /// 1. Skill listing attachment (system role, position 0) — per-turn
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
        let cleaned = Self::clean_thinking_content(&self.messages);
        let mut messages = Self::convert_history_to_internal(&cleaned);

        // ── Append current-turn user message ─────────────────────
        messages.push(InternalMessage {
            role: "user".to_string(),
            content: content.to_string(),
            content_blocks: None,
            tool_call_id: None,
        });

        // ── Skill listing attachment — at position 0 when non-empty ──
        let skill_listing_inserted = if let Some(listing) = skill_listing {
            if !listing.is_empty() {
                messages.insert(
                    0,
                    InternalMessage {
                        role: "system".to_string(),
                        content: listing,
                        content_blocks: None,
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

        // ── Memory injection — positioned per InjectionPosition ────
        if let Some(injection) = self.take_memory_injection() {
            tracing::info!(
                session_id = %self.session_id,
                event = "memory_injection",
                position = ?injection.position_mode,
                "consuming memory_injection slot"
            );
            let tool_msg = InternalMessage {
                role: "tool".to_string(),
                content: injection.content.clone(),
                content_blocks: None,
                tool_call_id: None,
            };
            match injection.position_mode {
                super::InjectionPosition::AfterCurrent => {
                    messages.push(tool_msg);
                }
                super::InjectionPosition::BeforeNext => {
                    let insert_pos = if skill_listing_inserted { 1 } else { 0 };
                    messages.insert(insert_pos, tool_msg);
                }
            }
        }

        messages
    }

    /// Convert a cleaned list of [`SessionMessage`]s into
    /// [`InternalMessage`]s suitable for an LLM API request.
    ///
    /// Non-tool content blocks are formatted via [`format_content_block`]
    /// and joined with newlines. Tool results are appended as independent
    /// `role="tool"` messages at the end.
    pub(crate) fn convert_history_to_internal(messages: &[SessionMessage]) -> Vec<InternalMessage> {
        let mut result: Vec<InternalMessage> = messages
            .iter()
            .map(|msg| {
                let non_tool_blocks: Vec<&ContentBlock> = msg
                    .content_blocks
                    .iter()
                    .filter(|b| !matches!(b, ContentBlock::ToolResult { .. }))
                    .collect();
                let content = non_tool_blocks
                    .iter()
                    .flat_map(|b| format_content_block(b))
                    .collect::<Vec<_>>()
                    .join("\n");
                InternalMessage {
                    role: msg.role.clone(),
                    content,
                    content_blocks: None,
                    tool_call_id: None,
                }
            })
            .collect();

        // Append tool results as independent role="tool" messages.
        for msg in messages {
            for b in &msg.content_blocks {
                if let ContentBlock::ToolResult {
                    tool_call_id,
                    content,
                } = b
                {
                    result.push(InternalMessage {
                        role: "tool".into(),
                        content: content.clone(),
                        content_blocks: None,
                        tool_call_id: Some(tool_call_id.clone()),
                    });
                }
            }
        }

        result
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
            let context = DynamicPromptContext {
                system_prompt: self.system_prompt.as_deref(),
                ctx: &ctx,
                workdir: &self.workdir,
                system_appends: &self.system_appends(),
                session_created_at: self.created_at,
                session_mode: self.session_mode(),
                overrides: self.prompt_overrides.as_ref(),
                user_input,
                is_compacted: self.is_compacted,
                is_sub_agent: self.is_sub_agent,
                is_git_status_enabled: self.is_git_status_enabled,
                mode_transition: self.take_mode_transition(),
                plan_file_path: self.plan_file_path(),
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

    /// Returns the plan file path associated with this session, if any.
    pub fn plan_file_path(&self) -> Option<&str> {
        self.plan_file_path.as_deref()
    }

    /// Sets the plan file path for this session.
    pub fn set_plan_file_path(&mut self, path: Option<String>) {
        self.plan_file_path = path
    }
}
