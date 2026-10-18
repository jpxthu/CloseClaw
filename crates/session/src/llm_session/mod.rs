//! Session layer for LLM conversations.
//!
//! Provides `SessionMessage`, `ChatSession` trait and `ConversationSession`.
//! See [`session_handles`] for cancel/cascade-stop and
//! `session_chat` for the `ChatSession` impl.

use cache_prefix::RequestPrefixSnapshot;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;
use tokio_util::sync::CancellationToken;

use crate::persistence::{PendingOperationDetail, ReasoningLevel, SessionMode};
use crate::run_health::{RunHealthChecker, RuntimeSnapshotManager};
use crate::spawn::CommunicationConfig;
use closeclaw_common::{
    ChildCompletionStatus, ChildSessionState, ContentBlock, LlmCaller, LlmState,
    NotificationPriority, PromptOverrides, RunningStats, SkillListingProvider, StreamingSink,
    SystemPromptBuilder, ToolExecState, ToolRegistryQuery, TurnCounter, UnifiedUsage,
    VerbosityLevel,
};

/// Max length of an append-section item (chars).
pub const APPEND_SECTION_MAX_LEN: usize = 500;
// Re-export `KillHandle` from common so call sites that
// `use closeclaw_session::KillHandle` keep working.
pub use closeclaw_common::tool_session::KillHandle;
// The accessor methods that consumed `InjectionParams` moved to
// `session_accessors`; kept here (test-only) so the `use super::*`
// glob chain in `tests/` keeps resolving it.
#[cfg(test)]
use closeclaw_common::InjectionParams;

mod cache_prefix;
mod memory_injection;
pub use memory_injection::{InjectionPosition, MemoryInjection};

pub mod mode_transition;

mod session_accessors;
mod session_cache_check;
// `ChatSession` trait + `impl ChatSession for ConversationSession` live in
// the sibling file `session_chat.rs`. Re-exported here so existing
// `use closeclaw_session::ChatSession;` call sites keep working.
mod session_chat;
pub use session_chat::ChatSession;
mod session_debug;

mod session_exec;
mod session_graceful_stop;
pub mod session_handles;
mod session_health;
mod session_llm;
mod session_pending;
mod session_pending_queue;
pub use session_pending_queue::{NotificationAudience, QueueEntry};
pub use session_pending_queue::{QueuePriority, UnifiedMessageQueue};
mod session_stop;
mod session_tool_session;
mod skill_listing;
pub mod streaming_assembly;
pub mod transcript_ops;
mod workflow;
mod workflow_cleanup;
mod workflow_lifecycle;
mod workflow_transcript_cleanup;
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
    /// LLM interaction state. See [`closeclaw_common::session_state`].
    pub llm_state: Arc<RwLock<LlmState>>,
    /// Per-call tool states. See [`closeclaw_common::session_state`].
    pub tool_states: Arc<RwLock<HashMap<String, (ToolExecState, Option<PendingOperationDetail>)>>>,
    /// Per-child session states. See [`closeclaw_common::session_state`].
    pub child_states:
        Arc<RwLock<HashMap<String, (ChildSessionState, Option<PendingOperationDetail>)>>>,
    /// When this session was created (Unix seconds).
    created_at: i64,
    /// Tool process kill handles. See [`session_handles`].
    pub tool_handles: Arc<RwLock<HashMap<String, Arc<dyn KillHandle>>>>,
    /// Spawned child sessions. See [`session_handles`].
    pub child_handles:
        Arc<RwLock<HashMap<String, std::sync::Weak<tokio::sync::RwLock<ConversationSession>>>>>,
    /// Cancellation token. See [`session_handles`].
    pub cancel_token: CancellationToken,
    /// `stop()` idempotency flag. See [`session_handles`].
    pub stopped: Arc<AtomicBool>,
    /// Per-session snapshot manager for transcript rollback safety, created lazily.
    snapshot_manager: Option<RuntimeSnapshotManager>,
    /// Per-session health checker (`Arc<Mutex>` for Clone compat).
    health_checker: Option<Arc<tokio::sync::Mutex<RunHealthChecker>>>,
    /// Active-yield flag; when true, user messages are queued.
    is_yielding: Arc<AtomicBool>,
    /// Communication config for child sessions.
    communication_config: Option<CommunicationConfig>,
    /// Bootstrap mode cached from AgentRegistry.
    bootstrap_mode: crate::bootstrap::loader::BootstrapMode,
    /// Whether this session runs without bootstrap files (agent declared
    /// `noBootstrap: true`). Cached at state-establishment points
    /// (create / restore / spawn) alongside `bootstrap_mode`; consumed by
    /// `rebuild_system_prompt` when assembling `InjectionParams`.
    no_bootstrap: bool,
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
    /// Active workflow run state (Value-erased; persisted in SessionCheckpoint).
    workflow_run: Option<serde_json::Value>,
    /// Workflow handler for tool result processing and engine state.
    workflow_handler: Option<crate::workflow_handler::WorkflowHandler>,
    /// Workflow engine port (injected by the composition root).
    workflow_port: Option<Arc<dyn crate::workflow_port::WorkflowPort>>,
    plan_file_path: Option<String>,
    /// Previous round's actually-sent request prefix snapshot (cache
    /// consistency check runtime state; never persisted to checkpoint).
    cache_prefix_prev: Option<RequestPrefixSnapshot>,
    /// Current round's staged prefix snapshot, taken at the request-build
    /// completion point; consumed at the unified advancement point.
    cache_prefix_staging: Option<RequestPrefixSnapshot>,
}
// `impl ConversationSession` is split across multiple blocks so each
// block stays under the CONTRIBUTING.md 100-line cap. Block A
// (below): construction. Basic accessors live in `session_accessors`.
// Block B (further down): pending messages and announce queue.
/// Construction.
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
            no_bootstrap: false,
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
            workflow_port: None,
            cache_prefix_prev: None,
            cache_prefix_staging: None,
        }
    }

    /// Builder variant of `new` that wires the cancel token to a
    /// parent-derived child token. See [`session_handles`].
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
    /// Accumulates a single API call's usage into the session stats.
    ///
    /// `model` is the model identifier used for this call (advances
    /// `last_call_model` for later model-switch exemption checks).
    pub fn accumulate_usage(&mut self, usage: &UnifiedUsage, model: Option<&str>) {
        self.stats.accumulate(usage, model);
    }
    /// Returns a reference to the most recent cache anomaly event, if any.
    pub fn last_cache_anomaly(&self) -> Option<&closeclaw_common::CacheAnomalyInfo> {
        self.stats.last_cache_anomaly()
    }
}

/// System appends and progress notification methods.
impl ConversationSession {
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
    /// Restore system-injected items (checkpoint transient field).
    pub fn restore_system_injection_appends(&mut self, items: Vec<String>) {
        self.system_injection_appends = items;
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
mod cache_prefix_tests;
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
