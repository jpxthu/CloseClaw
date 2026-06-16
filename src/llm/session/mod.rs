//! Session layer for LLM conversations.
//!
//! Provides `SessionMessage`, `ChatSession` trait and `ConversationSession`
//! for managing conversation state. The handle / cancel / cascade-stop
//! surface lives in [`super::session_handles`]; the public
//! [`ChatSession`] trait and its `ConversationSession` impl live in
//! [`super::session_chat`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::gateway::session_manager::communication::CommunicationConfig;
use crate::llm::session_state::{ChildSessionState, LlmState, ToolExecState};
use crate::llm::stats::RunningStats;
use crate::llm::streaming::StreamingSink;
use crate::llm::turn::TurnCounter;
use crate::llm::types::{ContentBlock, UnifiedUsage};
use crate::session::persistence::ReasoningLevel;

/// Maximum length of an individual append-section item (in characters).
///
/// Used by [`ConversationSession::add_system_append`] to truncate
/// user-supplied content. Previously lived in `crate::system_prompt`
/// alongside the now-removed global `APPEND_SECTION` static;
/// migrated here as part of issue #862 since the per-session
/// `system_appends` list is the only remaining production consumer.
pub const APPEND_SECTION_MAX_LEN: usize = 500;

// Re-export `KillHandle` so call sites that `use crate::llm::session::KillHandle`
// keep working after the trait moved to `session_handles` in Step 1.8.
pub(crate) use crate::llm::session_handles::KillHandle;

// `ChatSession` trait + `impl ChatSession for ConversationSession` live in
// the sibling file `session_chat.rs`. Re-exported here so existing
// `use crate::llm::session::ChatSession;` call sites keep working.
mod session_chat;
pub use session_chat::ChatSession;

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
    pending_messages: VecDeque<crate::session::persistence::PendingMessage>,
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
    pub(crate) llm_state: Arc<RwLock<LlmState>>,
    /// Per-call tool states. See [`super::session_state`].
    pub(crate) tool_states: Arc<RwLock<HashMap<String, ToolExecState>>>,
    /// Per-child session states. See [`super::session_state`].
    pub(crate) child_states: Arc<RwLock<HashMap<String, ChildSessionState>>>,
    /// When this session was created (Unix timestamp, seconds).
    /// Used by `build_dynamic_sections` as the ChannelContext timestamp
    /// so that system-prompt KV-cache prefix stays stable across turns.
    created_at: i64,
    /// Tool process kill handles. See [`super::session_handles`].
    pub(crate) tool_handles: Arc<RwLock<HashMap<String, Arc<dyn KillHandle>>>>,
    /// Spawned child sessions. See [`super::session_handles`].
    pub(crate) child_handles:
        Arc<RwLock<HashMap<String, std::sync::Weak<tokio::sync::RwLock<ConversationSession>>>>>,
    /// Cancellation token. See [`super::session_handles`].
    pub(crate) cancel_token: CancellationToken,
    /// `stop()` idempotency flag. See [`super::session_handles`].
    pub(crate) stopped: Arc<AtomicBool>,
    /// Communication configuration for spawned child sessions.
    /// When set, restricts which agents the child may communicate with.
    communication_config: Option<CommunicationConfig>,
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

    /// Sets the reasoning level.
    pub fn with_reasoning_level(mut self, level: ReasoningLevel) -> Self {
        self.reasoning_level = level;
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

    /// Returns the current reasoning level.
    pub fn reasoning_level(&self) -> ReasoningLevel {
        self.reasoning_level
    }

    /// Overrides the reasoning level at runtime.
    pub fn set_reasoning_level(&mut self, level: ReasoningLevel) {
        self.reasoning_level = level;
    }

    /// Replace the system prompt on an existing session.
    /// Used by `SessionManager::rebuild_system_prompt` after compaction.
    pub fn replace_system_prompt(&mut self, prompt: impl Into<String>) {
        self.system_prompt = Some(prompt.into());
    }

    /// Appends a message to the session.
    fn push_message(&mut self, role: &str, content_blocks: Vec<ContentBlock>) {
        self.messages.push(SessionMessage {
            role: role.to_string(),
            content_blocks,
            timestamp: Utc::now(),
        });
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
    ) -> Option<crate::llm::stats::CacheBreakInfo> {
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
    pub fn push_pending(&mut self, msg: crate::session::persistence::PendingMessage) {
        self.pending_messages.push_back(msg);
    }

    /// Pops the oldest pending message, if any.
    pub fn pop_pending(&mut self) -> Option<crate::session::persistence::PendingMessage> {
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
    pub fn get_pending_messages(&self) -> Vec<crate::session::persistence::PendingMessage> {
        self.pending_messages.iter().cloned().collect()
    }

    /// Restores pending messages from checkpoint data.
    /// Only pushes messages where `sent == false` back into the queue.
    pub fn restore_pending_messages(
        &mut self,
        messages: Vec<crate::session::persistence::PendingMessage>,
    ) {
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

    /// Inject a system message into the conversation history.
    pub fn inject_system_message(&mut self, text: String) {
        self.push_message("system", vec![ContentBlock::Text(text)]);
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

    /// Read-only access to the append-section list in insertion order.
    pub fn system_appends(&self) -> &[String] {
        &self.system_appends
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
            .finish()
    }
}

#[cfg(test)]
#[allow(deprecated)]
/// Helper: create a temporary directory path for tests.
pub(crate) fn tmp_path() -> std::path::PathBuf {
    tempfile::tempdir().unwrap().into_path()
}

#[cfg(test)]
mod tests;
