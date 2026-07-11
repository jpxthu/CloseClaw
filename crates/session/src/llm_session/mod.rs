//! Session layer for LLM conversations.
//!
//! Provides `SessionMessage`, `ChatSession` trait and `ConversationSession`
//! for managing conversation state. The handle / cancel / cascade-stop
//! surface lives in [`crate::session_handles`]; the public
//! [`ChatSession`] trait and its `ConversationSession` impl live in
//! [`super::session_chat`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::persistence::{ReasoningLevel, SessionMode};
use closeclaw_agent::communication::CommunicationConfig;
use closeclaw_common::RunningStats;
use closeclaw_common::StreamingSink;
use closeclaw_common::TurnCounter;
use closeclaw_common::VerbosityLevel;
use closeclaw_common::{ChildSessionState, LlmState, ToolExecState};
use closeclaw_common::{ContentBlock, UnifiedUsage};
use closeclaw_common::{LlmCaller, PromptOverrides, SystemPromptBuilder};

/// Maximum length of an individual append-section item (in characters).
///
/// Used by [`ConversationSession::add_system_append`] to truncate
/// user-supplied content. Previously lived in `crate::system_prompt`
/// alongside the now-removed global `APPEND_SECTION` static;
/// migrated here as part of issue #862 since the per-session
/// `system_appends` list is the only remaining production consumer.
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
mod session_handles;
mod session_llm;
pub mod streaming_assembly;
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
/// Produced when a run-mode child session completes its task; the parent
/// session drains these at the start of its next turn and injects the
/// result text as a `role="system"` SessionMessage.
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
}

/// A simple in-memory implementation of `ChatSession`.
#[derive(Clone)]
#[allow(dead_code)]
pub struct ConversationSession {
    session_id: String,
    messages: Vec<SessionMessage>,
    system_prompt: Option<String>,
    turn_counter: TurnCounter,
    model: String,
    compaction_state: Option<String>,
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
    /// Persisted in `SessionCheckpoint::system_appends` so archived
    /// sessions restore their append list intact.
    system_appends: Vec<String>,
    /// LLM interaction state. See [`super::session_state`].
    pub llm_state: Arc<RwLock<LlmState>>,
    /// Per-call tool states. See [`super::session_state`].
    pub tool_states: Arc<RwLock<HashMap<String, ToolExecState>>>,
    /// Per-child session states. See [`super::session_state`].
    pub child_states: Arc<RwLock<HashMap<String, ChildSessionState>>>,
    /// When this session was created (Unix timestamp, seconds).
    /// Used by `build_dynamic_sections` as the ChannelContext timestamp
    /// so that system-prompt KV-cache prefix stays stable across turns.
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
    /// Communication configuration for spawned child sessions.
    /// When set, restricts which agents the child may communicate with.
    communication_config: Option<CommunicationConfig>,
    /// Per-session memory-injection slot.
    /// Written by the active-searcher async task, consumed and cleared
    /// by the session owner when assembling the next message list.
    /// Not persisted across process restarts.
    memory_injection: Arc<Mutex<Option<MemoryInjection>>>,
    /// Last activity timestamp (Unix seconds) — updated on every message
    /// push or state mutation.  Used by the shutdown progress card to
    /// display accurate "elapsed since last activity" instead of session age.
    last_activity_at: i64,
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
    /// LLM caller injected by Gateway for delegating LLM requests.
    /// Set via [`set_llm_caller`](Self::set_llm_caller) after construction.
    llm_caller: Option<Arc<dyn LlmCaller>>,
    /// System prompt builder injected by Gateway for prompt rebuilds.
    /// Set via [`set_system_prompt_builder`](Self::set_system_prompt_builder) after construction.
    system_prompt_builder: Option<Arc<dyn SystemPromptBuilder>>,
    /// Prompt overrides injected by Gateway for prompt rebuilds.
    /// Set via [`set_prompt_overrides`](Self::set_prompt_overrides) after construction.
    prompt_overrides: Option<PromptOverrides>,
    /// Manual backgrounding signal. When notified, foreground commands
    /// being executed should be moved to background. Triggered by the
    /// user via an interface action (e.g. keyboard shortcut).
    pub manual_background_signal: Arc<tokio::sync::Notify>,
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
            communication_config: None,
            memory_injection: Arc::new(Mutex::new(None)),
            last_activity_at: Utc::now().timestamp(),
            shutdown_handle: None,
            verbosity_level: VerbosityLevel::default(),
            session_mode: Arc::new(Mutex::new(SessionMode::default())),
            progress_appends: Arc::new(Mutex::new(Vec::new())),
            llm_caller: None,
            system_prompt_builder: None,
            prompt_overrides: None,
            manual_background_signal: Arc::new(tokio::sync::Notify::new()),
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

    /// Returns the communication configuration, if set.
    pub fn communication_config(&self) -> Option<&CommunicationConfig> {
        self.communication_config.as_ref()
    }

    /// Set the shutdown handle for busy-count tracking during tool execution.
    pub fn set_shutdown_handle(&mut self, handle: Arc<dyn closeclaw_common::ShutdownSignal>) {
        self.shutdown_handle = Some(handle);
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

    /// Appends a message to the session.
    fn push_message(&mut self, role: &str, content_blocks: Vec<ContentBlock>) {
        self.messages.push(SessionMessage {
            role: role.to_string(),
            content_blocks,
            timestamp: Utc::now(),
        });
        self.last_activity_at = Utc::now().timestamp();
    }

    /// Sets the LLM busy state.
    pub fn set_llm_busy(&self, busy: bool) {
        self.is_llm_busy.store(busy, Ordering::SeqCst);
    }

    /// Returns the model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 将来源消息克隆后注入到自身 messages 列表，
    /// 保留原始时间戳。用于 Fork 模式注入父 session 的对话历史。
    pub fn clone_messages_from(&mut self, source: &[SessionMessage]) {
        self.messages.extend(source.iter().cloned());
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

/// Message replacement, stats, and streaming-sink accessors.
impl ConversationSession {
    /// Replaces all session messages with the given list.
    pub fn replace_messages(&mut self, new_messages: Vec<SessionMessage>) {
        self.messages = new_messages;
    }

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

/// Pending messages and announce queue.
impl ConversationSession {
    /// Pushes a pending message onto the queue.
    pub fn push_pending(&mut self, msg: crate::persistence::PendingMessage) {
        self.pending_messages.push_back(msg);
    }

    /// Pops the oldest pending message, if any.
    pub fn pop_pending(&mut self) -> Option<crate::persistence::PendingMessage> {
        self.pending_messages.pop_front()
    }

    /// Clears all pending messages from the queue.
    /// Returns the number of messages that were cleared.
    pub fn clear_pending(&mut self) -> usize {
        let n = self.pending_messages.len();
        self.pending_messages.clear();
        n
    }

    /// Returns whether there are any pending messages.
    pub fn has_pending(&self) -> bool {
        !self.pending_messages.is_empty()
    }

    /// Returns the number of pending messages.
    pub fn pending_count(&self) -> usize {
        self.pending_messages.len()
    }

    /// Returns a clone of all pending messages without consuming the queue.
    pub fn get_pending_messages(&self) -> Vec<crate::persistence::PendingMessage> {
        self.pending_messages.iter().cloned().collect()
    }

    /// Restores pending messages from checkpoint data.
    /// Only pushes messages where `sent == false` back into the queue.
    pub fn restore_pending_messages(&mut self, messages: Vec<crate::persistence::PendingMessage>) {
        for msg in messages {
            if !msg.sent {
                self.pending_messages.push_back(msg);
            }
        }
    }

    /// Push an announce event onto the in-memory announce queue.
    pub fn push_announce_to_queue(&mut self, event: AnnounceEvent) {
        self.announce_queue.push(event);
    }

    /// Drain all queued announce events, returning them in FIFO order.
    pub fn drain_announce_queue(&mut self) -> Vec<AnnounceEvent> {
        std::mem::take(&mut self.announce_queue)
    }

    /// Persist a user message into the conversation history.
    ///
    /// Writes the user input as a [`SessionMessage`] with `role="user"`
    /// so that [`build_compact_messages`] can extract complete
    /// user/assistant conversation history for compaction.
    pub fn append_user_message(&mut self, content: &str) {
        self.push_message("user", vec![ContentBlock::Text(content.to_string())]);
    }

    /// Inject a system message into the conversation history.
    pub fn inject_system_message(&mut self, text: String) {
        self.push_message("system", vec![ContentBlock::Text(text)]);
    }

    /// Inject a tool result into the conversation history.
    ///
    /// Used by the recovery path to inject failure results for pending
    /// tool calls so the LLM sees a natural tool-result response.
    pub fn inject_tool_result(&mut self, tool_call_id: &str, content: &str) {
        self.push_message(
            "tool",
            vec![ContentBlock::ToolResult {
                tool_call_id: tool_call_id.to_string(),
                content: content.to_string(),
            }],
        );
    }

    /// Extract pending tool calls from the last assistant message.
    ///
    /// Scans `self.messages` for the most recent assistant message, then
    /// collects every `ContentBlock::ToolUse` block into a
    /// [`PendingOperation`] list. This is used by the graceful-shutdown
    /// path to record tool calls that were requested but not yet executed.
    ///
    /// Unlike [`collect_pending_operations`](Self::collect_pending_operations)
    /// (which inspects tool_states / child_states), this method reads
    /// directly from the conversation history.
    pub fn extract_pending_tool_calls(&self) -> Vec<crate::persistence::PendingOperation> {
        use crate::persistence::{PendingOperation, PendingOperationType};

        let last_assistant = self.messages.iter().rev().find(|m| m.role == "assistant");

        let Some(msg) = last_assistant else {
            return Vec::new();
        };

        let now = Utc::now();
        msg.content_blocks
            .iter()
            .filter_map(|block| {
                if let ContentBlock::ToolUse { id, name, input } = block {
                    Some(PendingOperation {
                        op_id: id.clone(),
                        op_type: PendingOperationType::ToolCall,
                        name: name.clone(),
                        args: input.clone(),
                        created_at: now,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Collect pending operations from the current session state.
    ///
    /// Scans tool_states, child_states, and pending_messages to build a
    /// list of operations that were in progress when the session stopped.
    /// Used by the shutdown path to record what needs recovery on restart.
    pub fn collect_pending_operations(&self) -> Vec<crate::persistence::PendingOperation> {
        use crate::persistence::{PendingOperation, PendingOperationType};
        use chrono::Utc;
        use closeclaw_common::{ChildSessionState, ToolExecState};

        let mut ops = Vec::new();
        let now = Utc::now();

        // Tool calls in progress or pending
        {
            let tool_states = self.tool_states.read().expect("tool_states lock poisoned");
            for (tool_id, state) in tool_states.iter() {
                if matches!(
                    state,
                    ToolExecState::RunningForeground
                        | ToolExecState::RunningBackground
                        | ToolExecState::Pending
                ) {
                    ops.push(PendingOperation {
                        op_id: tool_id.clone(),
                        op_type: PendingOperationType::ToolCall,
                        name: tool_id.clone(),
                        args: String::new(),
                        created_at: now,
                    });
                }
            }
        }

        // Child sessions in progress
        {
            let child_states = self
                .child_states
                .read()
                .expect("child_states lock poisoned");
            for (child_id, state) in child_states.iter() {
                if matches!(state, ChildSessionState::Running) {
                    ops.push(PendingOperation {
                        op_id: child_id.clone(),
                        op_type: PendingOperationType::SubSessionSpawn,
                        name: child_id.clone(),
                        args: String::new(),
                        created_at: now,
                    });
                }
            }
        }

        // Unsold pending messages (outbound)
        for pm in &self.pending_messages {
            if !pm.sent {
                ops.push(PendingOperation {
                    op_id: pm.message_id.clone(),
                    op_type: PendingOperationType::OutboundMessage,
                    name: pm.message_id.clone(),
                    args: pm.content.clone(),
                    created_at: pm.created_at,
                });
            }
        }

        ops
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

impl std::fmt::Debug for ConversationSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationSession")
            .field("session_id", &self.session_id)
            .field("messages", &self.messages)
            .field("system_prompt", &self.system_prompt)
            .field("turn_counter", &self.turn_counter)
            .field("model", &self.model)
            .field("compaction_state", &self.compaction_state)
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
            .field("communication_config", &self.communication_config)
            .field("verbosity_level", &self.verbosity_level)
            .field(
                "session_mode",
                &*self
                    .session_mode
                    .lock()
                    .expect("session_mode lock poisoned"),
            )
            .field(
                "memory_injection",
                &*self
                    .memory_injection
                    .lock()
                    .expect("memory_injection lock poisoned"),
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
