//! Session layer for LLM conversations.
//!
//! Provides `SessionMessage`, `ChatSession` trait and `ConversationSession`.
//! See [`crate::session_handles`] for cancel/cascade-stop and
//! [`super::session_chat`] for the `ChatSession` impl.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;
use tokio_util::sync::CancellationToken;

use crate::persistence::{PendingOperationDetail, ReasoningLevel, SessionMode};
use crate::run_health::{RunHealthChecker, RuntimeSnapshotManager};
use crate::spawn::CommunicationConfig;
use closeclaw_common::{
    ChildCompletionStatus, ChildSessionState, LlmState, SkillListingProvider, ToolExecState,
};
use closeclaw_common::{ContentBlock, UnifiedUsage};
use closeclaw_common::{
    InjectionParams, LlmCaller, PromptOverrides, SessionRole, SystemPromptBuilder,
    ToolRegistryQuery,
};
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

pub mod mode_transition;

mod session_chat;
pub use session_chat::ChatSession;

mod session_exec;
pub mod session_handles;
mod session_health;
mod session_llm;
mod session_pending;
mod session_pending_queue;
pub use session_pending_queue::{QueueEntry, QueuePriority, UnifiedMessageQueue};
mod skill_listing;
pub mod streaming_assembly;
pub mod transcript_ops;
mod workflow;
mod workflow_cleanup;
mod workflow_lifecycle;
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
    /// Concatenated Text blocks from the child's final assistant message.
    pub result_text: String,
    /// When the child session finished. Used for logging / debug.
    pub completed_at: DateTime<Utc>,
    /// Delivery priority in the announce queue.
    pub priority: NotificationPriority,
    /// Completion status of the child session.
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
    /// Whether compacted; when true, sparse prompt variants inject (§8).
    is_compacted: bool,
    /// Whether this session is a sub-agent (§5, §8).
    is_sub_agent: bool,
    is_llm_busy: Arc<AtomicBool>,
    unified_queue: session_pending_queue::UnifiedMessageQueue,
    reasoning_level: ReasoningLevel,
    /// Effective reasoning level after provider downgrade.
    effective_reasoning_level: Option<ReasoningLevel>,
    workdir: PathBuf,
    stats: RunningStats,
    streaming_sink: Option<Arc<dyn StreamingSink>>,
    stream_enabled: bool,
    /// User-managed append-section items, managed by `/system` subcommand.
    /// Persisted in `SessionCheckpoint::user_appends` (alias: `system_appends`).
    user_appends: Vec<String>,
    /// System-injected append-section items (e.g. workflow context, recovery).
    /// Not persisted — re-injected at runtime by functional modules.
    system_injection_appends: Vec<String>,
    /// LLM interaction state. See [`super::session_state`].
    pub llm_state: Arc<RwLock<LlmState>>,
    /// Per-call tool states. See [`super::session_state`].
    pub tool_states: Arc<RwLock<HashMap<String, (ToolExecState, Option<PendingOperationDetail>)>>>,
    /// Per-child session states. See [`super::session_state`].
    pub child_states:
        Arc<RwLock<HashMap<String, (ChildSessionState, Option<PendingOperationDetail>)>>>,
    /// When this session was created (Unix seconds).
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
    /// Active-yield flag; when true, user messages are queued.
    is_yielding: Arc<AtomicBool>,
    /// Communication config for child sessions.
    communication_config: Option<CommunicationConfig>,
    /// Bootstrap mode cached from AgentRegistry.
    bootstrap_mode: crate::bootstrap::loader::BootstrapMode,
    /// Memory-injection slot, managed by active-searcher.
    memory_injection: Arc<Mutex<Option<MemoryInjection>>>,
    /// Last activity timestamp (Unix seconds) — updated on every mutation.
    last_activity_at: i64,
    /// Task IDs already injected this session (session-level dedup).
    injected_task_ids: Arc<Mutex<HashSet<String>>>,
    /// Skill listing provider for per-turn injection.
    pub(crate) skill_listing_provider: Option<Arc<dyn SkillListingProvider>>,
    /// Snapshot for incremental skill-listing diff; `None` on first turn.
    pub(crate) skill_listing_snapshot: Option<String>,
    /// Trigger full listing re-injection on next turn.
    pub(crate) pending_compaction_listing_reset: bool,
    /// Conditional skills activated via file-path matching.
    pub(crate) activated_conditional_skills: HashSet<String>,
    tool_registry: Option<Arc<dyn ToolRegistryQuery>>,
    /// Agent skill whitelist filter; `*` means no filtering.
    pub(crate) agent_skills: Option<Vec<String>>,
    /// Shutdown handle for busy-count tracking during tool execution.
    shutdown_handle: Option<Arc<dyn closeclaw_common::ShutdownSignal>>,
    /// File mtime tracking for staleness checks.
    file_mtimes: Arc<RwLock<HashMap<PathBuf, SystemTime>>>,
    /// Per-turn read range tracking for file dedup.
    file_read_ranges: Arc<RwLock<HashMap<PathBuf, closeclaw_common::FileReadCache>>>,
    /// Verbosity level controlling outbound content filtering.
    verbosity_level: VerbosityLevel,
    /// Session mode (§6).
    session_mode: Arc<Mutex<SessionMode>>,
    pending_session_mode: Arc<Mutex<Option<SessionMode>>>,
    pending_mode_transition: mode_transition::PendingTransition,
    /// Whether this session has ever entered Plan Mode.
    has_been_in_plan: Arc<AtomicBool>,
    /// Per-request context for dynamic-layer injection.
    request_context: Arc<Mutex<closeclaw_common::RequestContext>>,
    /// LLM caller injected by Gateway.
    llm_caller: Option<Arc<dyn LlmCaller>>,
    /// System prompt builder injected by Gateway.
    system_prompt_builder: Option<Arc<dyn SystemPromptBuilder>>,
    /// Prompt overrides injected by Gateway.
    prompt_overrides: Option<PromptOverrides>,
    dynamic_prompt_builder: Option<Arc<dyn closeclaw_common::DynamicPromptBuilder>>,
    /// Manual backgrounding signal. When notified, foreground commands
    /// being executed should be moved to background.
    pub manual_background_signal: Arc<tokio::sync::Notify>,
    /// Persistence service for persist_pending_checkpoint.
    checkpoint_storage: Option<Arc<dyn crate::persistence::PersistenceService>>,
    /// Whether git_status config switch is enabled.
    is_git_status_enabled: bool,
    /// Active workflow run state. Persisted in SessionCheckpoint.
    workflow_run: Option<closeclaw_workflow::run::WorkflowRun>,
    /// Workflow handler for tool result processing and engine state.
    workflow_handler: Option<crate::workflow_handler::WorkflowHandler>,
    plan_file_path: Option<String>,
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
            unified_queue: session_pending_queue::UnifiedMessageQueue::default(),
            reasoning_level: ReasoningLevel::default(),
            effective_reasoning_level: None,
            workdir,
            stats: RunningStats::new(),
            created_at: Utc::now().timestamp(),
            streaming_sink: None,
            stream_enabled: false,
            user_appends: Vec::new(),
            system_injection_appends: Vec::new(),
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
            injected_task_ids: Arc::new(Mutex::new(HashSet::new())),
            skill_listing_provider: None,
            skill_listing_snapshot: None,
            pending_compaction_listing_reset: false,
            activated_conditional_skills: HashSet::new(),
            tool_registry: None,
            agent_skills: None,
            shutdown_handle: None,
            verbosity_level: VerbosityLevel::default(),
            session_mode: Arc::new(Mutex::new(SessionMode::default())),
            pending_session_mode: Arc::new(Mutex::new(None)),
            pending_mode_transition: Arc::new(Mutex::new(None)),
            has_been_in_plan: Arc::new(AtomicBool::new(false)),
            request_context: Arc::new(Mutex::new(closeclaw_common::RequestContext::default())),
            file_mtimes: Arc::new(RwLock::new(HashMap::new())),
            file_read_ranges: Arc::new(RwLock::new(HashMap::new())),
            llm_caller: None,
            system_prompt_builder: None,
            prompt_overrides: None,
            plan_file_path: None,
            dynamic_prompt_builder: None,
            manual_background_signal: Arc::new(tokio::sync::Notify::new()),
            checkpoint_storage: None,
            is_git_status_enabled: false,
            workflow_run: None,
            workflow_handler: None,
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
    /// prepend a system-role attachment with the agent's available skills.
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
    /// Inject a [`ToolRegistryQuery`] into this session.
    ///
    /// Called by Gateway after session creation so the builder can
    /// propagate the registry through [`FragmentContext`] to
    /// [`ToolsFragmentProvider`](closeclaw_tools::ToolsFragmentProvider).
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
    /// Get a clone of the shutdown handle, if set.
    pub fn get_shutdown_handle(&self) -> Option<Arc<dyn closeclaw_common::ShutdownSignal>> {
        self.shutdown_handle.clone()
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
    /// Returns the current session mode, applying any pending deferred mode.
    pub fn session_mode(&self) -> SessionMode {
        self.apply_pending_session_mode_if_needed();
        *self
            .session_mode
            .lock()
            .expect("session_mode lock poisoned")
    }
    /// Overrides the session mode at runtime.
    pub fn set_session_mode(
        &mut self,
        mode: SessionMode,
        source: mode_transition::ModeChangeSource,
    ) {
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
        if let Some(t) = mode_transition::detect(prev, mode, has_been, source) {
            let pmt = &self.pending_mode_transition;
            *pmt.lock().expect("pending_mode_transition lock poisoned") = Some(t);
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
/// Stats and streaming-sink accessors.
impl ConversationSession {
    /// Returns a read-only reference to the running usage statistics.
    pub fn stats(&self) -> &RunningStats {
        &self.stats
    }
    /// Returns a mutable reference to the running usage statistics.
    pub fn stats_mut(&mut self) -> &mut RunningStats {
        &mut self.stats
    }

    /// Returns the streaming sink, if set.
    pub fn streaming_sink(&self) -> Option<&Arc<dyn StreamingSink>> {
        self.streaming_sink.as_ref()
    }
    /// Detects a cache break by comparing per-call hit rates.
    pub fn detect_cache_break_for_usage(
        &mut self,
        current_cache_read: Option<u32>,
        current_prompt_tokens: Option<u32>,
    ) -> Option<closeclaw_common::CacheBreakInfo> {
        self.stats
            .detect_cache_break_and_update(current_cache_read, current_prompt_tokens)
    }
    /// Accumulates a single API call's usage into the session stats.
    pub fn accumulate_usage(&mut self, usage: &UnifiedUsage) {
        self.stats.accumulate(usage);
    }
    /// Sets cache break detection thresholds on session stats.
    pub fn set_cache_break_thresholds(
        &mut self,
        thresholds: closeclaw_common::CacheBreakThresholds,
    ) {
        self.stats.set_cache_break_thresholds(thresholds);
    }
    /// Returns a reference to the most recent cache break event, if any.
    pub fn last_cache_break(&self) -> Option<&closeclaw_common::CacheBreakInfo> {
        self.stats.last_cache_break()
    }
}

/// System appends and progress notification methods.
impl ConversationSession {
    // ── System appends ──────────────────────────────────────────

    /// Append to user-managed list; returns new index.
    pub fn add_system_append(&mut self, content: String) -> usize {
        let next_index = self.user_appends.len();
        self.user_appends.push(content);
        next_index
    }

    /// Append to system-injected list; returns new index.
    pub fn add_system_injection_append(&mut self, content: String) -> usize {
        let next_index = self.system_injection_appends.len();
        self.system_injection_appends.push(content);
        next_index
    }

    /// Clear user-managed items only (system-injected unaffected).
    pub fn clear_system_appends(&mut self) -> usize {
        let n = self.user_appends.len();
        self.user_appends.clear();
        n
    }

    /// Clear system-injected items only.
    pub fn clear_system_injection_appends(&mut self) -> usize {
        let n = self.system_injection_appends.len();
        self.system_injection_appends.clear();
        n
    }

    /// Restore user-managed items (checkpoint restore path).
    pub fn restore_system_appends(&mut self, items: Vec<String>) {
        self.user_appends = items;
    }

    /// Merged list: user_appends followed by system_injection_appends.
    pub fn system_appends(&self) -> Vec<String> {
        let mut result = self.user_appends.clone();
        result.extend(self.system_injection_appends.iter().cloned());
        result
    }

    /// User-managed append-section items.
    pub fn user_system_appends(&self) -> &[String] {
        &self.user_appends
    }

    /// System-injected append-section items.
    pub fn system_injection_appends(&self) -> &[String] {
        &self.system_injection_appends
    }
}

/// Active-yield (Waiting state) methods.
impl ConversationSession {
    // ── Active-yield (Waiting state) methods ───────────────────

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
            .field("unified_queue", &self.unified_queue)
            .field("reasoning_level", &self.reasoning_level)
            .field("effective_reasoning_level", &self.effective_reasoning_level)
            .field("workdir", &self.workdir)
            .field("created_at", &self.created_at)
            .field("stats", &self.stats)
            .field(
                "streaming_sink",
                &self.streaming_sink.as_ref().map(|_| "<StreamingSink>"),
            )
            .field("stream_enabled", &self.stream_enabled)
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
                "skill_listing_provider",
                &self
                    .skill_listing_provider
                    .as_ref()
                    .map(|_| "<SkillListingProvider>"),
            )
            .field("skill_listing_snapshot", &self.skill_listing_snapshot)
            .field(
                "pending_compaction_listing_reset",
                &self.pending_compaction_listing_reset,
            )
            .field(
                "activated_conditional_skills",
                &self.activated_conditional_skills,
            )
            .field(
                "tool_registry",
                &self.tool_registry.as_ref().map(|_| "<TR>"),
            )
            .field("agent_skills", &self.agent_skills)
            .field(
                "injected_task_ids",
                &*self
                    .injected_task_ids
                    .lock()
                    .expect("injected_task_ids lock poisoned"),
            )
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
            .field(
                "file_mtimes",
                &self.file_mtimes.read().ok().map(|m| m.len()),
            )
            .field(
                "file_read_ranges",
                &self.file_read_ranges.read().ok().map(|m| m.len()),
            )
            .finish()
    }
}

impl Drop for ConversationSession {
    fn drop(&mut self) {
        self.stats.reset();
    }
}

#[cfg(test)]
#[allow(deprecated)]
/// Helper: create a temporary directory path for tests.
pub fn tmp_path() -> std::path::PathBuf {
    tempfile::tempdir().unwrap().into_path()
}
#[cfg(test)]
mod session_exec_tests;
#[cfg(test)]
mod session_pending_queue_tests;
#[cfg(test)]
mod streaming_assembly_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transcript_ops_tests;
