//! LLM request building for `ConversationSession`.
//!
//! Message assembly
//! ([`ConversationSession::build_llm_messages_with_listing`]), history
//! conversion ([`ConversationSession::convert_history_to_internal`]),
//! and [`InternalRequest`] construction
//! ([`ConversationSession::build_llm_request`] with its system-prompt
//! part derivation). Consumed by the invoke family in
//! `super::session_llm` and by the `ChatSession` drain path in
//! `session_chat`.

use closeclaw_common::{
    split_static_dynamic, ContentBlock, DynamicPromptContext, InternalMessage, InternalRequest,
};

use super::{ConversationSession, SessionMessage};

impl ConversationSession {
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
    ///    incremental diff from the
    ///    [`closeclaw_common::SkillListingProvider`] when non-empty.
    ///    Prepared by
    ///    [`ConversationSession::prepare_turn_skill_listing`].
    /// 2. Memory injection (tool role) — positioned per
    ///    [`super::InjectionPosition::AfterCurrent`] or `BeforeNext`.
    /// 3. User message — dedup-guarded: skipped when the history already
    ///    ends with the identical user message (the gateway pre-appends
    ///    the current message to history before invoking; pushing again
    ///    would send a duplicate copy that never enters history, breaking
    ///    the tail-append message-list semantics the cache prefix check
    ///    relies on).
    ///
    /// `skill_listing` is the pre-computed listing content to inject.
    /// Pass `None` to skip skill listing injection.
    ///
    /// `user_blocks_pre_appended` is the caller's declaration that the
    /// current user message was already pre-appended to history in
    /// structured blocks form (gateway production path); see
    /// [`Self::push_current_turn_user_message`] for the dedup-guard
    /// semantics.
    pub(crate) fn build_llm_messages_with_listing(
        &self,
        content: &str,
        skill_listing: Option<String>,
        user_blocks_pre_appended: bool,
    ) -> Vec<InternalMessage> {
        let cleaned = Self::clean_thinking_content(&self.messages);
        let mut messages = Self::convert_history_to_internal(&cleaned);
        Self::push_current_turn_user_message(&mut messages, content, user_blocks_pre_appended);
        self.attach_skill_listing(&mut messages, skill_listing);
        self.apply_pending_memory_injection(&mut messages);
        messages
    }
}

/// Current-turn user message assembly (dedup-guarded push).
impl ConversationSession {
    /// Append the current-turn user message to `messages`, unless the
    /// history already ends with it (dedup guard).
    ///
    /// The gateway production path pre-appends the user message to
    /// history (persistence/recovery semantics) before invoking, so
    /// the converted history already ends with this exact message.
    /// Pushing again would send a temporary duplicate copy that never
    /// enters history, so adjacent rounds stop being tail-append
    /// extensions and the cache prefix check reports PrefixMismatch
    /// every round. The guard skips the push when the history tail is
    /// a user message that equals the invoke content on role +
    /// content, or — when the caller declared
    /// `user_blocks_pre_appended` — when the tail carries content
    /// blocks **with empty flattened text**. The declaration is the
    /// caller's explicit statement that the current user message was
    /// already appended to history in structured blocks form (gateway
    /// production path for embedded small-image messages); the
    /// declaration branch then matches the image-round pre-append
    /// shape, where Image blocks are excluded from the text flatten
    /// so `content` is `""` (`content.is_empty()`). A blocks tail
    /// with non-empty text does **not** match the declaration (the
    /// flattened text could differ from the invoke content — e.g.
    /// image + caption), so the push happens and the message is never
    /// lost. Callers that did not pre-append (pending drain, tests)
    /// still get the push.
    ///
    /// Residual risk: a blocks tail with empty flattened text that is
    /// *not* the declared message (e.g. compaction rewrote the history
    /// between append and invoke into another image-only message) would
    /// still be deduped and the invoke content dropped from the sent
    /// list. When the tail does not match the declaration at all (not a
    /// blocks-carrying user message, or its flattened text is
    /// non-empty), the guard falls back to the string equality check and
    /// the push still happens.
    fn push_current_turn_user_message(
        messages: &mut Vec<InternalMessage>,
        content: &str,
        user_blocks_pre_appended: bool,
    ) {
        let duplicates_history_tail = messages.last().is_some_and(|tail| {
            tail.role == "user"
                && (tail.content == content
                    || (user_blocks_pre_appended
                        && tail.content.is_empty()
                        && tail.content_blocks.is_some()))
        });
        if !duplicates_history_tail {
            messages.push(InternalMessage {
                role: "user".to_string(),
                content: content.to_string(),
                content_blocks: None,
                tool_call_id: None,
            });
        }
    }
}

/// Skill-listing attachment and memory-injection application.
impl ConversationSession {
    /// Attach the skill listing as a system-role message at position 0.
    ///
    /// Non-empty listings only; an empty listing is skipped entirely.
    fn attach_skill_listing(
        &self,
        messages: &mut Vec<InternalMessage>,
        skill_listing: Option<String>,
    ) {
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
    }

    /// Consume any pending memory-injection slot and insert the tool
    /// message per its position mode.
    fn apply_pending_memory_injection(&self, messages: &mut Vec<InternalMessage>) {
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
    }
}

/// History conversion.
impl ConversationSession {
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
}

/// Format a single [`ContentBlock`] into zero or more string fragments.
///
/// The sole call site is
/// [`ConversationSession::convert_history_to_internal`], which both the
/// production `build_llm_messages_with_listing` path and the legacy
/// `build_api_request` path flow through. Tool results are excluded here
/// (they are appended as independent `role="tool"` messages by the caller).
fn format_content_block(b: &ContentBlock) -> Vec<String> {
    match b {
        ContentBlock::Text(t) => vec![t.clone()],
        ContentBlock::Thinking { thinking: t, .. } => {
            vec![format!("<thinking>{}</thinking>", t)]
        }
        ContentBlock::ToolUse { name, input, .. } => {
            vec![format!("[tool:{}] {}", name, input)]
        }
        // Currently unreachable: the sole caller `convert_history_to_internal`
        // filters out images before calling this function and preserves them as
        // structured `content_blocks` (see its doc). This arm is kept to satisfy
        // match exhaustiveness over all `ContentBlock` variants, and remains
        // available to future call paths. See issue #3764.
        ContentBlock::Image { name, .. } => vec![format!("[image: {}]", name)],
        ContentBlock::Audio { name, .. } => vec![format!("[audio: {}]", name)],
        ContentBlock::File { name, .. } => vec![format!("[file: {}]", name)],
        ContentBlock::ToolResult { .. } => vec![],
    }
}

/// Request construction.
impl ConversationSession {
    /// Build an [`InternalRequest`] from a pre-built messages list.
    pub(super) fn build_llm_request(
        &self,
        messages: Vec<InternalMessage>,
        stream: bool,
    ) -> InternalRequest {
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
}
