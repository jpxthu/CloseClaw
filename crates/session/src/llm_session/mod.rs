//! Session layer for LLM conversations.
//!
//! Provides `SessionMessage`, `ChatSession` trait and `ConversationSession`.
//! See [`crate::session_handles`] for cancel/cascade-stop and
//! [`super::session_chat`] for the `ChatSession` impl.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::persistence::{PendingOperationDetail, ReasoningLevel, SessionMode};
use crate::run_health::{RunHealthChecker, RuntimeSnapshotManager};
use crate::spawn::CommunicationConfig;
use closeclaw_common::{
    ChildCompletionStatus, ChildSessionState, LlmState, ModeTransition, SkillListingProvider,
    ToolExecState,
};
use closeclaw_common::{ContentBlock, UnifiedUsage};
use closeclaw_common::{LlmCaller, PromptOverrides, SystemPromptBuilder};
use closeclaw_common::{RunningStats, StreamingSink, TurnCounter, VerbosityLevel};
use closeclaw_tasks::NotificationPriority;

/// Max length of an append-section item (chars).
pub const APPEND_SECTION_MAX_LEN: usize = 500;

// Re-export `KillHandle` from common so call sites that
// `use closeclaw_session::KillHandle` keep working.
pub use closeclaw_common::tool_session::KillHandle;

// `ChatSession` trait + `impl ChatSession for ConversationSession` live in
// the sibling file `session_chat.rs`. Re-exported here so existing
// `use closeclaw_session::ChatSession;` call sites keep working.
mod memory_injection;
pub use memory_injection::{InjectionPosition, MemoryInjection};

mod progress_notifier;
pub use progress_notifier::PROGRESS_APPEND_PREFIX;

mod session_chat;
pub use session_chat::ChatSession;

mod session_exec;
pub mod session_handles;
mod session_health;
mod session_llm;
mod session_pending;
mod session_pending_queue;
mod skill_listing;
pub mod streaming_assembly;
pub mod transcript_ops;
pub use streaming_assembly::SessionStream;

/// A single message in a conversation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    /// Role of the message sender (e.g., "user", "assistant", "system").
    pub role: String,
    /// Ordered list of content blocks.
    pub content_blocks: Vec<ContentBlock>,
    /// When the message was created.
    pub timestamp: DateTime<Utc>,
}

/// Announce event pushed by a child session to its parent.
///
/// Produced when a run-mode child completes; the parent injects
/// the result as a `role="system"` SessionMessage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceEvent {
    /// ID of the child session that completed.
    pub child_session_id: String,
    /// Agent ID of the child that completed.
    pub child_agent_id: String,
    /// Concatenated Text content blocks from the child's final assistant
    /// message. Thinking blocks are filtered out.
    pub result_text: String,
    /// When the child session finished. Used for logging / debug.
    pub completed_at: DateTime<Utc>,
    /// Delivery priority. Controls insertion order in the announce queue
    /// so higher-priority events are drained first.
    pub priority: NotificationPriority,
    /// Completion status of the child session. Controls the injection
    /// text so the parent session knows whether the child completed
    /// successfully, errored, or was terminated.
    pub status: ChildCompletionStatus,
}

/// A simple in-memory implementation of `ChatSession`.
#[derive(Clone)]
#[allow(dead_code, clippy::type_complexity)]
pub struct ConversationSession {
    session_id: String,
    /// Conversation messages (transcript_ops).
    pub(crate) messages: Vec<SessionMessage>,
    system_prompt: Option<String>,
    turn_counter: TurnCounter,
    model: String,
    compaction_state: Option<String>,
    /// Whether the session has been compacted (context summarized).
    /// When `true`, sparse prompt variants are injected instead of
    /// the full mode instruction. See design doc §8.
    is_compacted: bool,
    /// Whether this session is a sub-agent.
    /// When `true`, the sub-agent sparse prompt variant is injected
    /// instead of the full mode instruction. See design doc §5, §8.
    is_sub_agent: bool,
    is_llm_busy: Arc<AtomicBool>,
    pending_messages: VecDeque<crate::persistence::PendingMessage>,
    reasoning_level: ReasoningLevel,
    workdir: PathBuf,
    stats: RunningStats,
    streaming_sink: Option<Arc<dyn StreamingSink>>,
    stream_enabled: bool,
    /// In-memory queue of announce events. Drained at the start of
    /// each parent turn. Not persisted across process restarts.
    announce_queue: Vec<AnnounceEvent>,
    /// Per-session append-section items, managed by `/system` subcommand.
    /// Persisted in `SessionCheckpoint::system_appends`.
    system_appends: Vec<String>,
    /// LLM interaction state. See [`super::session_state`].
    pub llm_state: Arc<RwLock<LlmState>>,
    /// Per-call tool states. See [`super::session_state`].
    pub tool_states: Arc<RwLock<HashMap<String, (ToolExecState, Option<PendingOperationDetail>)>>>,
    /// Per-child session states. See [`super::session_state`].
    pub child_states:
        Arc<RwLock<HashMap<String, (ChildSessionState, Option<PendingOperationDetail>)>>>,
    /// When this session was created (Unix timestamp, seconds).
    /// Used by `build_dynamic_sections` as the ChannelContext timestamp.
    created_at: i64,
    /// Tool process kill handles. See [`super::session_handles`].
    pub tool_handles: Arc<RwLock<HashMap<String, Arc<dyn KillHandle>>>>,
    /// Spawned child sessions. See [`super::session_handles`].
    pub child_handles:
        Arc<RwLock<HashMap<String, std::sync::Weak<tokio::sync::RwLock<ConversationSession>>>>>,
    /// Cancellation token. See [`super::session_handles`].
    pub cancel_token: CancellationToken,
    /// `stop()` idempotency flag. See [`super::session_handles`].
    pub stopped: Arc<AtomicBool>,
    /// Per-session snapshot manager for transcript rollback safety, created lazily.
    snapshot_manager: Option<RuntimeSnapshotManager>,
    /// Per-session health checker (Arc<Mutex> for Clone compat).
    health_checker: Option<Arc<tokio::sync::Mutex<RunHealthChecker>>>,
    /// Active-yield flag. When `true`, the session is in主动 Waiting state
    /// (entered via `sessions_yield`). User messages are queued until resume.
    is_yielding: Arc<AtomicBool>,
    /// Communication configuration for spawned child sessions.
    /// When set, restricts which agents the child may communicate with.
    communication_config: Option<CommunicationConfig>,
    /// Bootstrap mode cached from AgentRegistry at session creation.
    /// Defaults to [`BootstrapMode::Full`].
    bootstrap_mode: crate::bootstrap::loader::BootstrapMode,
    /// Per-session memory-injection slot, managed by active-searcher.
    /// Not persisted across process restarts.
    memory_injection: Arc<Mutex<Option<MemoryInjection>>>,
    /// Last activity timestamp (Unix seconds) — updated on every mutation.
    last_activity_at: i64,
    /// Skill listing provider for per-turn skill injection.
    /// Injected by Gateway at session creation. When set, each LLM turn
    /// prepends a tool-role attachment with the agent's skill listing.
    pub(crate) skill_listing_provider: Option<Arc<dyn SkillListingProvider>>,
    /// Snapshot of the last skill listing (excluding conditional skills)
    /// used for incremental diff computation. `None` on the first turn.
    pub(crate) skill_listing_snapshot: Option<String>,
    /// Names of conditional skills that have been activated during this
    /// session's lifetime via file-path matching. Activated skills are
    /// included in subsequent turn listings as incremental additions.
    pub(crate) activated_conditional_skills: HashSet<String>,
    /// Agent-level skill whitelist filter. When set, only skills whose
    /// names appear in this list are included in the injected listing.
    /// A list containing `"*"` means no filtering.
    pub(crate) agent_skills: Option<Vec<String>>,
    /// Shutdown handle for busy-count tracking during tool execution.
    shutdown_handle: Option<Arc<dyn closeclaw_common::ShutdownSignal>>,
    /// Runtime-only execution progress appends. Entries are tagged with
    /// [`PROGRESS_APPEND_PREFIX`] and managed by
    /// [`PlanStateNotifier::on_progress_changed`]. Merged into
    /// [`system_appends()`](Self::system_appends) at read time so the
    /// system prompt builder sees them automatically.
    progress_appends: Arc<Mutex<Vec<String>>>,
    /// Verbosity level controlling outbound content filtering.
    verbosity_level: VerbosityLevel,
    /// Session mode controlling session-level behavior constraints.
    /// Orthogonal to `ReasoningMode` — see [`SessionMode`] docs.
    session_mode: Arc<Mutex<SessionMode>>,
    /// Pending mode transition notification. When set, the next
    /// system prompt build will include a `Section::ModeTransition`
    /// and then clear this slot (one-shot injection).
    pending_mode_transition: Arc<Mutex<Option<ModeTransition>>>,
    /// Per-request context for dynamic-layer injection.
    request_context: Arc<Mutex<closeclaw_common::RequestContext>>,
    /// LLM caller injected by Gateway for delegating LLM requests.
    /// Set via [`set_llm_caller`](Self::set_llm_caller) after construction.
    llm_caller: Option<Arc<dyn LlmCaller>>,
    /// System prompt builder injected by Gateway for prompt rebuilds.
    /// Set via [`set_system_prompt_builder`](Self::set_system_prompt_builder) after construction.
    system_prompt_builder: Option<Arc<dyn SystemPromptBuilder>>,
    /// Prompt overrides injected by Gateway for prompt rebuilds.
    /// Set via [`set_prompt_overrides`](Self::set_prompt_overrides) after construction.
    prompt_overrides: Option<PromptOverrides>,
    dynamic_prompt_builder: Option<Arc<dyn closeclaw_common::DynamicPromptBuilder>>,
    /// Manual backgrounding signal. When notified, foreground commands
    /// being executed should be moved to background.
    pub manual_background_signal: Arc<tokio::sync::Notify>,
    /// Optional persistence service for `persist_pending_checkpoint`.
    ///
    /// Injected by the Gateway after session creation so that
    /// `ToolSession::persist_pending_checkpoint` can persist the
    /// current pending operations without requiring a reference to
    /// the Gateway's `CheckpointManager`.
    checkpoint_storage: Option<Arc<dyn crate::persistence::PersistenceService>>,
}
// `impl ConversationSession` is split across multiple blocks so each
// block stays under the CONTRIBUTING.md 100-line cap. Block A
// (below): construction and basic setters/getters. Block B (further
// down): pending messages and announce queue.

/// Construction and basic setters/getters.
impl ConversationSession {
    /// Creates a new session with the given model and working directory.
    pub fn new(session_id: String, model: String, workdir: PathBuf) -> Self {
        Self {
            session_id,
            messages: Vec::new(),
            system_prompt: None,
            turn_counter: TurnCounter::new(),
            model,
            compaction_state: None,
            is_compacted: false,
            is_sub_agent: false,
            is_llm_busy: Arc::new(AtomicBool::new(false)),
            pending_messages: VecDeque::new(),
            reasoning_level: ReasoningLevel::default(),
            workdir,
            stats: RunningStats::new(),
            created_at: Utc::now().timestamp(),
            streaming_sink: None,
            stream_enabled: false,
            announce_queue: Vec::new(),
            system_appends: Vec::new(),
            llm_state: Arc::new(RwLock::new(LlmState::Idle)),
            tool_states: Arc::new(RwLock::new(HashMap::new())),
            child_states: Arc::new(RwLock::new(HashMap::new())),
            tool_handles: Arc::new(RwLock::new(HashMap::new())),
            child_handles: Arc::new(RwLock::new(HashMap::new())),
            cancel_token: CancellationToken::new(),
            stopped: Arc::new(AtomicBool::new(false)),
            snapshot_manager: None,
            health_checker: None,
            is_yielding: Arc::new(AtomicBool::new(false)),
            communication_config: None,
            bootstrap_mode: crate::bootstrap::loader::BootstrapMode::Full,
            memory_injection: Arc::new(Mutex::new(None)),
            last_activity_at: Utc::now().timestamp(),
            skill_listing_provider: None,
            skill_listing_snapshot: None,
            activated_conditional_skills: HashSet::new(),
            agent_skills: None,
            shutdown_handle: None,
            verbosity_level: VerbosityLevel::default(),
            session_mode: Arc::new(Mutex::new(SessionMode::default())),
            pending_mode_transition: Arc::new(Mutex::new(None)),
            request_context: Arc::new(Mutex::new(closeclaw_common::RequestContext::default())),
            progress_appends: Arc::new(Mutex::new(Vec::new())),
            llm_caller: None,
            system_prompt_builder: None,
            prompt_overrides: None,
            dynamic_prompt_builder: None,
            manual_background_signal: Arc::new(tokio::sync::Notify::new()),
            checkpoint_storage: None,
        }
    }

    /// Builder variant of `new` that wires the cancel token to a
    /// parent-derived child token. See [`super::session_handles`].
    pub fn with_cancel_token(
        session_id: String,
        model: String,
        workdir: PathBuf,
        cancel_token: CancellationToken,
    ) -> Self {
        let mut s = Self::new(session_id, model, workdir);
        s.cancel_token = cancel_token;
        s
    }

    /// Returns the current working directory.
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// Sets the working directory.
    pub fn set_workdir(&mut self, path: PathBuf) {
        self.workdir = path;
    }

    /// Sets the system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
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
    /// Sets the reasoning level.
    pub fn with_reasoning_level(mut self, level: ReasoningLevel) -> Self {
        self.reasoning_level = level;
        self
    }
    /// Sets the session mode.
    pub fn with_session_mode(self, mode: SessionMode) -> Self {
        *self
            .session_mode
            .lock()
            .expect("session_mode lock poisoned") = mode;
        self
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
    /// Sets the bootstrap mode for this session.
    pub fn with_bootstrap_mode(mut self, mode: crate::bootstrap::loader::BootstrapMode) -> Self {
        self.bootstrap_mode = mode;
        self
    }
    /// Returns the cached bootstrap mode for this session.
    pub fn bootstrap_mode(&self) -> crate::bootstrap::loader::BootstrapMode {
        self.bootstrap_mode
    }
    /// Returns the communication configuration, if set.
    pub fn communication_config(&self) -> Option<&CommunicationConfig> {
        self.communication_config.as_ref()
    }
    /// Set the shutdown handle for busy-count tracking during tool execution.
    pub fn set_shutdown_handle(&mut self, handle: Arc<dyn closeclaw_common::ShutdownSignal>) {
        self.shutdown_handle = Some(handle);
    }

    /// Inject a [`SkillListingProvider`] for per-turn skill listing injection.
    ///
    /// Called by Gateway after session creation so each LLM turn can
    /// prepend a tool-role attachment with the agent's available skills.
    pub fn set_skill_listing_provider(&mut self, provider: Arc<dyn SkillListingProvider>) {
        self.skill_listing_provider = Some(provider);
    }

    /// Returns a reference to the injected [`SkillListingProvider`], if any.
    pub fn skill_listing_provider(&self) -> Option<&Arc<dyn SkillListingProvider>> {
        self.skill_listing_provider.as_ref()
    }

    /// Set the agent-level skill whitelist filter.
    ///
    /// When set, only skills whose names appear in `skills` are included
    /// in the injected listing. A list containing `"*"` means no filtering.
    pub fn set_agent_skills(&mut self, skills: Vec<String>) {
        self.agent_skills = Some(skills);
    }

    /// Returns the agent-level skill whitelist, if any.
    pub fn agent_skills(&self) -> Option<&[String]> {
        self.agent_skills.as_deref()
    }
    /// Returns the last skill listing snapshot, if any.
    pub fn skill_listing_snapshot(&self) -> Option<&str> {
        self.skill_listing_snapshot.as_deref()
    }

    /// Returns a reference to the set of activated conditional skill names.
    pub fn activated_conditional_skills(&self) -> &HashSet<String> {
        &self.activated_conditional_skills
    }

    /// Compute the skill listing for the current turn without
    /// mutating session state.
    /// Returns a clone of the manual backgrounding signal.
    ///
    /// Callers (e.g. `BashTool::execute_command`) can await on
    /// `signal.notified()` inside a `tokio::select!` to react to
    /// a manual backgrounding request.
    pub fn manual_background_notify(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.manual_background_signal)
    }

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

    /// Get a clone of the shutdown handle, if set.
    pub fn get_shutdown_handle(&self) -> Option<Arc<dyn closeclaw_common::ShutdownSignal>> {
        self.shutdown_handle.clone()
    }

    /// Returns the current reasoning level.
    pub fn reasoning_level(&self) -> ReasoningLevel {
        self.reasoning_level
    }

    /// Overrides the reasoning level at runtime.
    pub fn set_reasoning_level(&mut self, level: ReasoningLevel) {
        self.reasoning_level = level;
    }

    /// Returns the current verbosity level.
    pub fn verbosity_level(&self) -> VerbosityLevel {
        self.verbosity_level
    }

    /// Overrides the verbosity level at runtime.
    pub fn set_verbosity_level(&mut self, level: VerbosityLevel) {
        self.verbosity_level = level;
    }

    /// Returns the current session mode.
    pub fn session_mode(&self) -> SessionMode {
        *self
            .session_mode
            .lock()
            .expect("session_mode lock poisoned")
    }

    /// Overrides the session mode at runtime.
    pub fn set_session_mode(&mut self, mode: SessionMode) {
        *self
            .session_mode
            .lock()
            .expect("session_mode lock poisoned") = mode;
    }

    /// Set a pending mode transition to be injected into the next
    /// system prompt build. Overwrites any previously pending transition.
    pub fn set_pending_mode_transition(&self, transition: ModeTransition) {
        *self
            .pending_mode_transition
            .lock()
            .expect("pending_mode_transition lock poisoned") = Some(transition);
    }

    /// Take the pending mode transition, clearing the slot.
    /// Returns `None` if no transition was pending.
    pub fn take_pending_mode_transition(&self) -> Option<ModeTransition> {
        self.pending_mode_transition
            .lock()
            .expect("pending_mode_transition lock poisoned")
            .take()
    }

    /// Set per-request context for dynamic-layer injection.
    pub fn set_request_context(&self, ctx: closeclaw_common::RequestContext) {
        *self.request_context.lock().expect("rc poisoned") = ctx;
    }
    /// Returns a clone of the current per-request context.
    pub fn request_context(&self) -> closeclaw_common::RequestContext {
        self.request_context.lock().expect("rc poisoned").clone()
    }

    /// Returns a reference to the memory-injection Arc.
    pub fn memory_injection_arc(&self) -> &Arc<Mutex<Option<MemoryInjection>>> {
        &self.memory_injection
    }

    /// Write a memory-injection payload into the slot.
    pub fn set_memory_injection(&self, injection: MemoryInjection) {
        let mut slot = self
            .memory_injection
            .lock()
            .expect("memory_injection lock poisoned");
        *slot = Some(injection);
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

    /// Replace the system prompt on an existing session.
    /// Used by `SessionManager::rebuild_system_prompt` after compaction.
    pub fn replace_system_prompt(&mut self, prompt: impl Into<String>) {
        self.system_prompt = Some(prompt.into());
    }

    /// Returns the current system prompt, if any.
    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    /// Rebuild the system prompt using the session's own builder and overrides.
    ///
    /// This is the session-side entry point for prompt rebuilds after
    /// compaction or config changes. The session owns the builder and
    /// overrides; no external references are needed.
    ///
    /// * `bootstrap_mode_override` — optional override for the bootstrap mode
    ///   used when building the prompt. Pass `None` for standard rebuilds;
    ///   spawn callers should pass the child's bootstrap mode.
    ///
    /// Returns the rebuilt prompt string for callers that need it
    /// (e.g. initial session creation in `resolve.rs`).
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
        let prompt = builder
            .build_prompt(
                session_id,
                agent_id,
                self.prompt_overrides.as_ref(),
                bootstrap_mode_override,
            )
            .await;
        self.replace_system_prompt(prompt.clone());
        prompt
    }

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
        use closeclaw_common::ContentBlock;
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
}

/// Stats and streaming-sink accessors.
impl ConversationSession {
    /// Returns a read-only reference to the running usage statistics.
    pub fn stats(&self) -> &RunningStats {
        &self.stats
    }

    /// Records a pre-call fingerprint of prompt components.
    ///
    /// Computes a fingerprint from the system prompt, tools, and
    /// headers, then compares it against the previous fingerprint to
    /// detect pending changes. The resulting [`PendingChanges`][closeclaw_common::PendingChanges]
    /// are stored in [`RunningStats`] and consumed by the post-call
    /// cache-break attribution logic.
    ///
    /// Should be called **before** the LLM call while holding a
    /// write lock on the session.
    pub fn record_prompt_fingerprint(
        &mut self,
        system_static: Option<&str>,
        tools: Option<&[String]>,
        headers: Option<&[(&str, &str)]>,
    ) {
        self.stats.record_fingerprint(system_static, tools, headers);
    }

    /// Returns the streaming sink, if set.
    pub fn streaming_sink(&self) -> Option<&Arc<dyn StreamingSink>> {
        self.streaming_sink.as_ref()
    }

    /// Detects a cache break between the previous and current
    /// `cache_read_tokens`, then updates the tracked last value.
    ///
    /// Delegates to [`RunningStats::detect_cache_break_and_update`].
    pub fn detect_cache_break_for_usage(
        &mut self,
        current_cache_read: Option<u32>,
    ) -> Option<closeclaw_common::CacheBreakInfo> {
        self.stats.detect_cache_break_and_update(current_cache_read)
    }

    /// Accumulates a single API call's usage into the session stats.
    pub fn accumulate_usage(&mut self, usage: &UnifiedUsage) {
        self.stats.accumulate(usage);
    }
}

/// Per-session append-section items (managed by `/system` subcommand).
///
/// Replaces the previous global static `APPEND_SECTION` in
/// [`crate::system_prompt::sections`] so archived sessions can
/// restore their append list intact. The legacy global was removed
/// in #862 and is no longer present.
impl ConversationSession {
    /// Append `content` to the per-session append-section list.
    /// Truncates to `APPEND_SECTION_MAX_LEN` (500) chars if needed.
    /// Returns the index of the newly added item (0-based, sequential).
    pub fn add_system_append(&mut self, content: String) -> usize {
        let truncated: String = if content.chars().count() > APPEND_SECTION_MAX_LEN {
            content
                .chars()
                .take(APPEND_SECTION_MAX_LEN)
                .collect::<String>()
        } else {
            content
        };
        let next_index = self.system_appends.len();
        self.system_appends.push(truncated);
        next_index
    }

    /// Clear all append-section items. Returns the count cleared.
    pub fn clear_system_appends(&mut self) -> usize {
        let n = self.system_appends.len();
        self.system_appends.clear();
        n
    }

    /// Replace the current append-section list with `items`
    /// (typically called from a checkpoint restore path).
    pub fn restore_system_appends(&mut self, items: Vec<String>) {
        self.system_appends = items;
    }

    /// Read-only access to the append-section list in insertion order,
    /// with runtime progress appends merged at the end.
    pub fn system_appends(&self) -> Vec<String> {
        let mut combined = self.system_appends.clone();
        let progress = self
            .progress_appends
            .lock()
            .expect("progress_appends lock poisoned");
        combined.extend(progress.iter().cloned());
        combined
    }

    /// Returns only the user-managed append-section items (excludes
    /// runtime progress appends). Used by persistence layers that
    /// should not persist ephemeral progress state.
    pub fn user_system_appends(&self) -> &[String] {
        &self.system_appends
    }

    /// Returns only the runtime progress appends, if any.
    pub fn progress_appends(&self) -> Vec<String> {
        self.progress_appends
            .lock()
            .expect("progress_appends lock poisoned")
            .clone()
    }
}

/// Active-yield (Waiting state) methods.
///
/// Provides the runtime basis for `sessions_yield` (Step 1.5).
impl ConversationSession {
    /// Enter active Waiting state (set yielding flag).
    pub fn enter_waiting(&self) {
        self.is_yielding.store(true, Ordering::SeqCst);
        tracing::debug!(session_id = %self.session_id, "entered active Waiting");
    }

    /// Exit active Waiting state and resume normal processing.
    pub fn exit_waiting(&self) {
        self.is_yielding.store(false, Ordering::SeqCst);
        tracing::debug!(session_id = %self.session_id, "exited active Waiting");
    }

    /// Returns `true` if the session is in active Waiting (yielding).
    pub fn is_waiting(&self) -> bool {
        self.is_yielding.load(Ordering::SeqCst)
    }

    /// Returns `true` if any child session is still running.
    pub fn has_active_children(&self) -> bool {
        let states = self
            .child_states
            .read()
            .expect("child_states lock poisoned");
        states
            .values()
            .any(|(s, _)| *s == ChildSessionState::Running)
    }
}

impl std::fmt::Debug for ConversationSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationSession")
            .field("session_id", &self.session_id)
            .field("messages", &self.messages)
            .field("system_prompt", &self.system_prompt)
            .field("turn_counter", &self.turn_counter)
            .field("model", &self.model)
            .field("compaction_state", &self.compaction_state)
            .field("is_compacted", &self.is_compacted)
            .field("is_sub_agent", &self.is_sub_agent)
            .field("pending_messages", &self.pending_messages)
            .field("reasoning_level", &self.reasoning_level)
            .field("workdir", &self.workdir)
            .field("created_at", &self.created_at)
            .field("stats", &self.stats)
            .field(
                "streaming_sink",
                &self.streaming_sink.as_ref().map(|_| "<StreamingSink>"),
            )
            .field("stream_enabled", &self.stream_enabled)
            .field("announce_queue", &self.announce_queue)
            .field(
                "llm_state",
                &*self.llm_state.read().expect("llm_state lock poisoned"),
            )
            .field(
                "tool_states",
                &*self.tool_states.read().expect("tool_states lock poisoned"),
            )
            .field(
                "child_states",
                &*self
                    .child_states
                    .read()
                    .expect("child_states lock poisoned"),
            )
            .field(
                "tool_handles",
                &self
                    .tool_handles
                    .read()
                    .expect("tool_handles lock poisoned")
                    .len(),
            )
            .field(
                "child_handles",
                &self
                    .child_handles
                    .read()
                    .expect("child_handles lock poisoned")
                    .len(),
            )
            .field("cancel_token", &"<CancelToken>")
            .field("stopped", &self.stopped.load(Ordering::SeqCst))
            .field("is_yielding", &self.is_yielding.load(Ordering::SeqCst))
            .field("communication_config", &self.communication_config)
            .field("bootstrap_mode", &self.bootstrap_mode)
            .field("verbosity_level", &self.verbosity_level)
            .field(
                "session_mode",
                &*self
                    .session_mode
                    .lock()
                    .expect("session_mode lock poisoned"),
            )
            .field(
                "pending_mode_transition",
                &*self
                    .pending_mode_transition
                    .lock()
                    .expect("pending_mode_transition lock poisoned"),
            )
            .field(
                "skill_listing_provider",
                &self
                    .skill_listing_provider
                    .as_ref()
                    .map(|_| "<SkillListingProvider>"),
            )
            .field("skill_listing_snapshot", &self.skill_listing_snapshot)
            .field(
                "activated_conditional_skills",
                &self.activated_conditional_skills,
            )
            .field("agent_skills", &self.agent_skills)
            .field(
                "memory_injection",
                &*self
                    .memory_injection
                    .lock()
                    .expect("memory_injection lock poisoned"),
            )
            .field(
                "health_checker",
                &self.health_checker.as_ref().map(|_| "<HC>"),
            )
            .field("manual_background_signal", &"<Notify>")
            .finish()
    }
}
#[cfg(test)]
#[allow(deprecated)]
/// Helper: create a temporary directory path for tests.
pub fn tmp_path() -> std::path::PathBuf {
    tempfile::tempdir().unwrap().into_path()
}
#[cfg(test)]
mod streaming_assembly_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transcript_ops_tests;
