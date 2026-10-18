//! Basic accessors for `ConversationSession`.
//!
//! Extracted from `mod.rs` (issue #3629): the getter/setter half of
//! the former "construction and basic setters/getters" impl block,
//! grouped by concern into sub-100-line `impl` blocks — basic
//! properties, reasoning / verbosity, session mode & request context,
//! gateway injection seams, memory-injection slot, system prompt, and
//! message-transcript helpers. Construction (`new` /
//! `with_cancel_token`) stays in `mod.rs`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::persistence::{ReasoningLevel, SessionMode};
use crate::run_health::RuntimeSnapshotManager;
use crate::spawn::CommunicationConfig;
use closeclaw_common::{
    ContentBlock, InjectionParams, LlmCaller, PromptOverrides, SessionRole, SkillListingProvider,
    SystemPromptBuilder, ToolRegistryQuery, VerbosityLevel,
};

use super::mode_transition::{detect, ModeChangeSource};
use super::{ConversationSession, MemoryInjection, SessionMessage};

/// Basic property accessors: paths, timestamps, bootstrap and
/// communication config.
impl ConversationSession {
    /// Returns the current working directory.
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }
    /// Sets the working directory.
    pub fn set_workdir(&mut self, path: PathBuf) {
        self.workdir = path;
    }
    /// Returns the Unix timestamp (seconds) when this session was created.
    pub fn session_created_at(&self) -> i64 {
        self.created_at
    }
    /// Returns the Unix timestamp (seconds) of the last activity.
    /// Updated on every message push or significant state mutation.
    pub fn last_activity_at(&self) -> i64 {
        self.last_activity_at
    }
    /// Sets the bootstrap mode for this session.
    pub fn with_bootstrap_mode(mut self, mode: crate::bootstrap::loader::BootstrapMode) -> Self {
        self.bootstrap_mode = mode;
        self
    }
    /// Returns the cached bootstrap mode for this session.
    pub fn bootstrap_mode(&self) -> crate::bootstrap::loader::BootstrapMode {
        self.bootstrap_mode
    }
    /// Sets whether this session runs without bootstrap files
    /// (`noBootstrap: true` from the agent config). Takes priority over
    /// `bootstrap_mode`; orthogonal to it.
    pub fn with_no_bootstrap(mut self, no_bootstrap: bool) -> Self {
        self.no_bootstrap = no_bootstrap;
        self
    }
    /// Returns whether this session runs without bootstrap files.
    pub fn no_bootstrap(&self) -> bool {
        self.no_bootstrap
    }
    /// Sets the communication configuration.
    pub fn with_communication_config(mut self, config: CommunicationConfig) -> Self {
        self.communication_config = Some(config);
        self
    }
    /// Sets the communication configuration on an existing session.
    pub fn set_communication_config(&mut self, config: CommunicationConfig) {
        self.communication_config = Some(config);
    }
    /// Returns the communication configuration, if set.
    pub fn communication_config(&self) -> Option<&CommunicationConfig> {
        self.communication_config.as_ref()
    }
}

/// Reasoning level and verbosity accessors.
impl ConversationSession {
    /// Sets the reasoning level.
    pub fn with_reasoning_level(mut self, level: ReasoningLevel) -> Self {
        self.reasoning_level = level;
        self
    }
    /// Returns the current reasoning level.
    pub fn reasoning_level(&self) -> ReasoningLevel {
        self.reasoning_level
    }
    /// Effective reasoning level (post-provider-downgrade), falls back to `reasoning_level`.
    pub fn effective_reasoning_level(&self) -> ReasoningLevel {
        self.effective_reasoning_level
            .unwrap_or(self.reasoning_level)
    }
    /// Sets the effective reasoning level after provider downgrade detection.
    pub fn set_effective_reasoning_level(&mut self, level: ReasoningLevel) {
        self.effective_reasoning_level = Some(level);
    }
    /// Overrides the reasoning level at runtime.
    pub fn set_reasoning_level(&mut self, level: ReasoningLevel) {
        self.reasoning_level = level;
        // Reset effective level — will be re-evaluated on next LLM call.
        self.effective_reasoning_level = None;
    }
    /// Returns the current verbosity level.
    pub fn verbosity_level(&self) -> VerbosityLevel {
        self.verbosity_level
    }
    /// Overrides the verbosity level at runtime.
    pub fn set_verbosity_level(&mut self, level: VerbosityLevel) {
        self.verbosity_level = level;
    }
}

/// Session mode and per-request context accessors.
impl ConversationSession {
    /// Sets the session mode.
    pub fn with_session_mode(self, mode: SessionMode) -> Self {
        *self
            .session_mode
            .lock()
            .expect("session_mode lock poisoned") = mode;
        self
    }
    /// Returns the current session mode, applying any pending deferred mode.
    pub fn session_mode(&self) -> SessionMode {
        self.apply_pending_session_mode_if_needed();
        *self
            .session_mode
            .lock()
            .expect("session_mode lock poisoned")
    }
    /// Overrides the session mode at runtime.
    pub fn set_session_mode(&mut self, mode: SessionMode, source: ModeChangeSource) {
        let prev = {
            let sm = &self.session_mode;
            let mut lock = sm.lock().expect("session_mode lock poisoned");
            let p = *lock;
            *lock = mode;
            p
        };
        let has_been = self.has_been_in_plan.load(Ordering::Relaxed);
        if mode == SessionMode::Plan {
            self.has_been_in_plan.store(true, Ordering::Relaxed);
        }
        let transition = detect(prev, mode, has_been, source);
        if let Some(t) = transition {
            let pmt = &self.pending_mode_transition;
            *pmt.lock().expect("pending_mode_transition lock poisoned") = Some(t);
        }
        if prev != mode {
            self.spawn_mode_checkpoint_writeback(mode);
        }
    }
    /// Set per-request context for dynamic-layer injection.
    pub fn set_request_context(&self, ctx: closeclaw_common::RequestContext) {
        *self.request_context.lock().expect("rc poisoned") = ctx;
    }
    /// Returns a clone of the current per-request context.
    pub fn request_context(&self) -> closeclaw_common::RequestContext {
        self.request_context.lock().expect("rc poisoned").clone()
    }
}

/// Shutdown handle, skill-listing and manual-background accessors.
impl ConversationSession {
    /// Set the shutdown handle for busy-count tracking during tool execution.
    pub fn set_shutdown_handle(&mut self, handle: Arc<dyn closeclaw_common::ShutdownSignal>) {
        self.shutdown_handle = Some(handle);
    }
    /// Get a clone of the shutdown handle, if set.
    pub fn get_shutdown_handle(&self) -> Option<Arc<dyn closeclaw_common::ShutdownSignal>> {
        self.shutdown_handle.clone()
    }
    /// Inject a [`SkillListingProvider`] for per-turn skill listing injection.
    ///
    /// Called by Gateway after session creation so each LLM turn can
    /// prepend a system-role attachment with the agent's available skills.
    pub fn set_skill_listing_provider(&mut self, provider: Arc<dyn SkillListingProvider>) {
        self.skill_listing_provider = Some(provider);
    }
    /// Returns a reference to the injected [`SkillListingProvider`], if any.
    pub fn skill_listing_provider(&self) -> Option<&Arc<dyn SkillListingProvider>> {
        self.skill_listing_provider.as_ref()
    }
    /// Returns the last skill listing snapshot, if any.
    pub fn skill_listing_snapshot(&self) -> Option<&str> {
        self.skill_listing_snapshot.as_deref()
    }
    /// Returns a reference to the set of activated conditional skill names.
    pub fn activated_conditional_skills(&self) -> &HashSet<String> {
        &self.activated_conditional_skills
    }
    /// Returns a clone of the manual backgrounding signal.
    ///
    /// Callers (e.g. `BashTool::execute_command`) can await on
    /// `signal.notified()` inside a `tokio::select!` to react to
    /// a manual backgrounding request.
    pub fn manual_background_notify(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.manual_background_signal)
    }
}

/// Gateway injection seams: LLM caller, prompt builder, prompt
/// overrides, tool registry, git-status switch, snapshot / checkpoint
/// storage.
impl ConversationSession {
    /// Inject an [`LlmCaller`] into this session.
    ///
    /// Called by Gateway after session creation so the session can
    /// delegate LLM requests without the Gateway holding the caller.
    pub fn set_llm_caller(&mut self, caller: Arc<dyn LlmCaller>) {
        self.llm_caller = Some(caller);
    }
    /// Returns a reference to the injected [`LlmCaller`], if any.
    pub fn llm_caller(&self) -> Option<&Arc<dyn LlmCaller>> {
        self.llm_caller.as_ref()
    }
    /// Inject a [`SystemPromptBuilder`] into this session.
    ///
    /// Called by Gateway after session creation so the session can
    /// rebuild its own system prompt without the Gateway holding the builder.
    pub fn set_system_prompt_builder(&mut self, builder: Arc<dyn SystemPromptBuilder>) {
        self.system_prompt_builder = Some(builder);
    }
    /// Returns `true` if a [`SystemPromptBuilder`] has been injected.
    pub fn has_system_prompt_builder(&self) -> bool {
        self.system_prompt_builder.is_some()
    }
    /// Inject prompt overrides into this session.
    ///
    /// Called by Gateway after session creation so the session can
    /// apply overrides when rebuilding its system prompt.
    pub fn set_prompt_overrides(&mut self, overrides: Option<PromptOverrides>) {
        self.prompt_overrides = overrides;
    }
    /// Inject a [`ToolRegistryQuery`] into this session.
    ///
    /// Called by Gateway after session creation so the builder can
    /// propagate the registry through [`closeclaw_common::FragmentContext`] to
    /// `ToolsFragmentProvider`.
    pub fn set_tool_registry(&mut self, registry: Arc<dyn ToolRegistryQuery>) {
        self.tool_registry = Some(registry);
    }
    /// Set the git_status config switch for this session.
    ///
    /// Called by Gateway after session creation so the dynamic builder
    /// can conditionally inject a GitStatus section.
    pub fn set_git_status(&mut self, is_git_status_enabled: bool) {
        self.is_git_status_enabled = is_git_status_enabled;
    }
    /// Set the snapshot meta store for persisting snapshot metadata.
    /// Creates the snapshot manager lazily if not already present.
    pub fn set_snapshot_meta_store(
        &mut self,
        store: Arc<dyn crate::run_health::SnapshotMetaStore>,
    ) {
        let mgr = self
            .snapshot_manager
            .get_or_insert_with(RuntimeSnapshotManager::new);
        mgr.set_meta_store(store);
    }
    /// Set the persistence service for `persist_pending_checkpoint`.
    ///
    /// Injected by the Gateway after session creation so that the
    /// `ToolSession::persist_pending_checkpoint` implementation can
    /// persist the current pending operations.
    pub fn set_checkpoint_storage(
        &mut self,
        storage: Arc<dyn crate::persistence::PersistenceService>,
    ) {
        self.checkpoint_storage = Some(storage);
    }
}

/// Memory-injection slot accessors.
impl ConversationSession {
    /// Returns a reference to the memory-injection Arc.
    pub fn memory_injection_arc(&self) -> &Arc<Mutex<Option<MemoryInjection>>> {
        &self.memory_injection
    }
    /// Write a memory-injection payload into the slot.
    ///
    /// Applies session-level dedup: if the injection carries a
    /// `task_id` that has already been injected this session, the
    /// write is skipped (returns `false`). Injections without a
    /// `task_id` are always accepted.
    ///
    /// Returns `true` if the injection was accepted, `false` if
    /// skipped due to dedup.
    pub fn set_memory_injection(&self, injection: MemoryInjection) -> bool {
        let mut slot = self
            .memory_injection
            .lock()
            .expect("memory_injection lock poisoned");
        if let Some(ref task_id) = injection.task_id {
            let mut ids = self
                .injected_task_ids
                .lock()
                .expect("injected_task_ids lock poisoned");
            if ids.contains(task_id.as_str()) {
                tracing::debug!(
                    task_id,
                    "session-level dedup: skipping already-injected task"
                );
                return false;
            }
            ids.insert(task_id.clone());
        }
        *slot = Some(injection);
        true
    }
    /// Take the current memory-injection payload, replacing the slot
    /// with `None`. Returns `None` if the slot was already empty.
    pub fn take_memory_injection(&self) -> Option<MemoryInjection> {
        let mut slot = self
            .memory_injection
            .lock()
            .expect("memory_injection lock poisoned");
        slot.take()
    }
    /// Record that `event_id` has been injected in the current session.
    /// If no injection exists yet, this is a no-op.
    pub fn add_injected_event_id(&self, event_id: i64) {
        let mut slot = self
            .memory_injection
            .lock()
            .expect("memory_injection lock poisoned");
        if let Some(ref mut inj) = *slot {
            inj.add_injected_event_id(event_id);
        }
    }
    /// Returns `true` if `event_id` was already injected in this session.
    pub fn is_event_injected(&self, event_id: i64) -> bool {
        let slot = self
            .memory_injection
            .lock()
            .expect("memory_injection lock poisoned");
        slot.as_ref()
            .map(|inj| inj.is_event_injected(event_id))
            .unwrap_or(false)
    }
}

/// System prompt accessors and rebuild.
impl ConversationSession {
    /// Sets the system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }
    /// Replace the system prompt on an existing session.
    /// Used by `SessionManager::rebuild_system_prompt` after compaction.
    pub fn replace_system_prompt(&mut self, prompt: impl Into<String>) {
        self.system_prompt = Some(prompt.into());
    }
    /// Returns the current system prompt, if any.
    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }
    /// Rebuild the system prompt via [`InjectionParams`] (§注入链路的参数契约).
    ///
    /// Assembles params from session state, delegates to the injected builder,
    /// and clears activated conditional skills after rebuild. When
    /// `bootstrap_mode_override` is `None`, the agent default is used.
    ///
    /// # Returns
    ///
    /// The rebuilt prompt string; empty string if no builder is configured
    /// (see `resolve.rs` for the typical call site).
    pub async fn rebuild_system_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        bootstrap_mode_override: Option<crate::bootstrap::loader::BootstrapMode>,
    ) -> String {
        let Some(builder) = self.system_prompt_builder.as_deref() else {
            tracing::debug!(
                session_id,
                "no system prompt builder configured, skipping rebuild"
            );
            return String::new();
        };
        let activated: Vec<String> = self.activated_conditional_skills.iter().cloned().collect();
        let session_role = if self.is_sub_agent {
            SessionRole::Sub
        } else {
            SessionRole::Main
        };
        let params = InjectionParams {
            session_id: session_id.to_owned(),
            agent_id: agent_id.to_owned(),
            overrides: self.prompt_overrides.clone(),
            bootstrap_mode_override,
            activated_skills: activated,
            session_role,
            no_bootstrap: self.no_bootstrap,
            tool_registry: self.tool_registry.clone(),
        };
        let prompt = builder.build_prompt_with_params(&params).await;
        self.replace_system_prompt(prompt.clone());
        // Clear the activation markers: merged into static layer during
        // rebuild, so they must not reappear in subsequent per-turn
        // incremental injection. (§条件激活 skill 消息注入: "标记并入
        // 静态层即清除，避免重复渲染；新激活的技能重新标记")
        let pre_clear_count = self.activated_conditional_skills.len();
        self.activated_conditional_skills.clear();
        tracing::debug!(
            session_id,
            cleared_count = pre_clear_count,
            "rebuilt SP: cleared activated_conditional_skills"
        );
        prompt
    }
}

/// Message transcript helpers, model and busy-flag accessors.
impl ConversationSession {
    pub(crate) fn push_message(&mut self, role: &str, content_blocks: Vec<ContentBlock>) {
        self.push_message_with_timestamp(role, content_blocks, chrono::Utc::now());
    }
    /// Like [`push_message`] but uses the provided `timestamp`.
    pub(crate) fn push_message_with_timestamp(
        &mut self,
        role: &str,
        content_blocks: Vec<ContentBlock>,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) {
        self.messages.push(SessionMessage {
            role: role.to_string(),
            content_blocks,
            timestamp,
        });
        self.last_activity_at = chrono::Utc::now().timestamp();
    }
    /// Sets the LLM busy state.
    pub fn set_llm_busy(&self, busy: bool) {
        self.is_llm_busy.store(busy, Ordering::SeqCst);
    }
    /// Returns the model name.
    pub fn model(&self) -> &str {
        &self.model
    }
    /// Clone messages from `source`, preserving original timestamps.
    pub(crate) fn clone_messages_from(&mut self, source: &[SessionMessage]) {
        for msg in source {
            self.append_transcript_preserving_timestamp(
                &msg.role,
                msg.content_blocks.clone(),
                msg.timestamp,
            );
        }
    }

    /// Walk messages in reverse, concatenate `ContentBlock::Text`
    /// blocks from the most recent `role="assistant"` message, and
    /// return `None` if no assistant message exists. `Thinking` blocks
    /// are intentionally excluded.
    pub fn collect_last_assistant_text(messages: &[SessionMessage]) -> Option<String> {
        let mut text_buf = String::new();
        for msg in messages.iter().rev() {
            if msg.role == "assistant" {
                for block in &msg.content_blocks {
                    if let ContentBlock::Text(t) = block {
                        if !text_buf.is_empty() {
                            text_buf.push('\n');
                        }
                        text_buf.push_str(t);
                    }
                }
                return Some(text_buf);
            }
        }
        None
    }

    /// Find assistant text matching `target` (reverse walk, first match).
    pub fn find_assistant_text_by_content(
        messages: &[SessionMessage],
        target: &str,
    ) -> Option<String> {
        for msg in messages.iter().rev() {
            if msg.role == "assistant" {
                let text: String = msg
                    .content_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if text == target {
                    return Some(text);
                }
            }
        }
        None
    }
}
