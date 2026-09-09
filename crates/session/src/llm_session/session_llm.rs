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
    /// Handles conditional activation detection, state update, and
    /// listing computation. Newly activated skills are NOT injected
    /// in the current turn — only base-listing changes are included;
    /// the activation entries will naturally appear in the next
    /// turn's diff.
    ///
    /// Returns the listing content to inject (`None` when nothing
    /// to inject).
    fn prepare_turn_skill_listing(&mut self, content: &str) -> Option<String> {
        self.reset_compaction_snapshot_if_pending();
        let newly_activated = self.detect_conditional_activations(content);
        self.apply_conditional_activations(&newly_activated);
        let (listing, new_snapshot) = self.compute_skill_listing_for_turn();
        self.maybe_update_snapshot(&newly_activated, new_snapshot);
        self.filter_listing_on_activation_turns(listing, &newly_activated)
    }

    /// Reset the skill listing snapshot after compaction.
    fn reset_compaction_snapshot_if_pending(&mut self) {
        if self.pending_compaction_listing_reset && self.skill_listing_snapshot.is_some() {
            tracing::debug!(
                session_id = %self.session_id,
                "prepare_turn_skill_listing: clearing snapshot after compaction"
            );
            self.skill_listing_snapshot = None;
            self.pending_compaction_listing_reset = false;
        }
    }

    /// Extract file paths from the content and find newly activated
    /// conditional skills.
    fn detect_conditional_activations(&self, content: &str) -> std::collections::HashSet<String> {
        let paths = Self::extract_file_paths(content);
        let mut newly_activated = std::collections::HashSet::new();
        if !paths.is_empty() {
            if let Some(provider) = self.skill_listing_provider.as_ref() {
                let matches = provider.find_conditional_matches(&paths);
                for m in matches {
                    if !self.is_skill_already_activated(&m.name) {
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
        newly_activated
    }

    /// Check if a skill name is not yet in the activated set.
    fn is_skill_already_activated(&self, name: &str) -> bool {
        self.activated_conditional_skills.contains(name)
    }

    /// Apply newly activated conditional skills to session state.
    ///
    /// Passes `None` for the snapshot to avoid updating it — the
    /// next turn's diff will pick up the new entries.
    fn apply_conditional_activations(
        &mut self,
        newly_activated: &std::collections::HashSet<String>,
    ) {
        if !newly_activated.is_empty() {
            self.apply_skill_listing_update(None, newly_activated);
        }
    }

    /// Conditionally update the snapshot based on activation state.
    ///
    /// On activation turns, skip saving so the next turn's diff
    /// naturally includes the newly activated entry.
    fn maybe_update_snapshot(
        &mut self,
        newly_activated: &std::collections::HashSet<String>,
        new_snapshot: Option<String>,
    ) {
        if newly_activated.is_empty() {
            if let Some(snapshot) = new_snapshot {
                self.skill_listing_snapshot = Some(snapshot);
            }
        }
    }

    /// Strip newly activated entries from the listing on activation
    /// turns so only base-listing changes are injected this turn.
    fn filter_listing_on_activation_turns(
        &self,
        listing: Option<String>,
        newly_activated: &std::collections::HashSet<String>,
    ) -> Option<String> {
        if newly_activated.is_empty() {
            return listing;
        }
        listing.and_then(|l| Self::filter_listing_excluding_entries(l, newly_activated))
    }

    /// Filter a skill listing to exclude entries matching given skill
    /// names.
    ///
    /// Removes lines that are formatted skill entries (starting with
    /// `- **`) for the specified skill names, and returns `None` if
    /// the result is empty.
    fn filter_listing_excluding_entries(
        listing: String,
        exclude_names: &std::collections::HashSet<String>,
    ) -> Option<String> {
        let filtered: Vec<&str> = listing
            .lines()
            .filter(|line| !line.is_empty() && !Self::is_entry_for_skill(line, exclude_names))
            .collect();
        let result = filtered.join("\n");
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Check if a listing line is an entry for one of the given skill
    /// names.
    ///
    /// Matches the pattern `- **{name**:` to avoid substring false
    /// matches on partial skill names.
    fn is_entry_for_skill(line: &str, skill_names: &std::collections::HashSet<String>) -> bool {
        line.starts_with("- **")
            && skill_names
                .iter()
                .any(|name| line.contains(&format!("**{}**:", name)))
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

        let listing = self.prepare_turn_skill_listing(content);
        let messages = self.build_llm_messages_with_listing(content, listing);

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

        let listing = self.prepare_turn_skill_listing(content);
        let messages = self.build_llm_messages_with_listing(content, listing);

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
    pub(crate) fn build_llm_messages_with_listing(
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
        if let Some(listing) = skill_listing {
            if !listing.is_empty() {
                let entry_count = listing.lines().filter(|l| !l.is_empty()).count();
                let first_entry = listing
                    .lines()
                    .find(|l| !l.is_empty())
                    .map(|l| l.to_string())
                    .unwrap_or_default();
                messages.insert(
                    0,
                    InternalMessage {
                        role: "system".to_string(),
                        content: listing,
                        content_blocks: None,
                        tool_call_id: None,
                    },
                );
                tracing::info!(
                    session_id = %self.session_id,
                    event = "skill_listing_injection",
                    entry_count,
                    first_entry = %first_entry,
                    "injecting skill listing as system message"
                );
            }
        }

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
                    // Insert before the last user message (the new message
                    // for this turn), matching the design doc:
                    // [history..., tool: memory摘要, 用户: new_message]
                    //
                    // `messages.len() - 1` always points to the last
                    // element, which is the user message we just pushed
                    // above (nothing else modifies `messages` between
                    // the push and this insert).
                    let insert_pos = messages.len() - 1;
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
    ///
    /// When a message contains [`ContentBlock::Image`] blocks, these are
    /// preserved in [`InternalMessage::content_blocks`] as structured
    /// content blocks (not flattened to text). Non-image blocks are still
    /// flattened to the `content` string for backward compatibility.
    pub(crate) fn convert_history_to_internal(messages: &[SessionMessage]) -> Vec<InternalMessage> {
        let mut result: Vec<InternalMessage> = messages
            .iter()
            .map(|msg| {
                let non_tool_blocks: Vec<&ContentBlock> = msg
                    .content_blocks
                    .iter()
                    .filter(|b| !matches!(b, ContentBlock::ToolResult { .. }))
                    .collect();

                // Flatten non-image blocks to text for the content string.
                let text_content = non_tool_blocks
                    .iter()
                    .filter(|b| !matches!(b, ContentBlock::Image { .. }))
                    .flat_map(|b| format_content_block(b))
                    .collect::<Vec<_>>()
                    .join("\n");

                // Preserve image blocks as structured content_blocks.
                let image_blocks: Vec<ContentBlock> = non_tool_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Image { name, url } => Some(ContentBlock::Image {
                            name: name.clone(),
                            url: url.clone(),
                        }),
                        _ => None,
                    })
                    .collect();

                let content_blocks = if image_blocks.is_empty() {
                    None
                } else {
                    Some(image_blocks)
                };

                InternalMessage {
                    role: msg.role.clone(),
                    content: text_content,
                    content_blocks,
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
            reasoning_level: self.effective_reasoning_level(),
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
        _messages: &[InternalMessage],
    ) -> (Option<String>, Option<String>) {
        if let Some(ref builder) = self.dynamic_prompt_builder {
            let ctx = self.request_context();
            let context = DynamicPromptContext {
                system_prompt: self.system_prompt.as_deref(),
                ctx: &ctx,
                workdir: &self.workdir,
                system_appends: &self.system_appends(),
                session_created_at: self.created_at,
                session_mode: self.session_mode(),
                overrides: self.prompt_overrides.as_ref(),
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
                Some(prompt) => {
                    let (s, d) = split_static_dynamic(prompt);
                    (s, d)
                }
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
