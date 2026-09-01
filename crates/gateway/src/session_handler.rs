//! SessionMessageHandler - Gateway-layer LLM session manager with busy/pending state.
//!
//! This component implements the complete busy/pending messaging loop:
//! - idle message  → set busy → LLM call → clear busy → drain pending
//! - busy message  → enqueue pending
//!
//! `LlmCaller` trait is used for LLM calls (non-streaming and streaming),
//! going through the full five-layer architecture (CacheAdapter → PluginPipeline →
//! Interpreter → Protocol → Provider).
//! The `output_tx` channel is used to surface LLM response text to callers.

use super::Gateway;
use crate::session_manager::SessionManager;
use crate::shutdown_handle::ShutdownHandle;
use closeclaw_llm::types::ContentBlock;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::ProviderModelKnowledge;
use closeclaw_session::compaction::{CompactConfig, CompactionResult, CompactionService};
use closeclaw_session::run_health::TranscriptOp;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use super::OutputTx;

/// Metadata about an inbound message, passed through the handling pipeline.
#[derive(Clone)]
pub struct MessageMetadata {
    /// Open ID of the message sender.
    pub sender_id: String,
    /// Channel identifier (e.g. "feishu", "telegram").
    pub channel: String,
    /// Unix timestamp (seconds) when the message was created.
    pub timestamp: i64,
    /// Actual chat/group name (e.g. Feishu group title), or empty.
    pub chat_name: String,
    /// Trace ID for debug-log correlation (inbound message chain).
    pub trace_id: Option<String>,
    /// Session key for debug-log correlation.
    pub session_key: Option<String>,
    /// Root span ID for debug-log child span derivation.
    /// Set by inbound_queue when the root TraceContext is created.
    pub span_id: Option<String>,
}

impl MessageMetadata {
    pub fn default_meta() -> Self {
        Self {
            sender_id: String::new(),
            channel: String::new(),
            timestamp: chrono::Utc::now().timestamp(),
            chat_name: String::new(),
            trace_id: None,
            session_key: None,
            span_id: None,
        }
    }

    /// Convert into a [`RequestContext`](closeclaw_common::RequestContext)
    /// for session-side dynamic-layer injection.
    pub fn to_request_context(&self) -> closeclaw_common::RequestContext {
        closeclaw_common::RequestContext {
            sender_id: self.sender_id.clone(),
            channel: self.channel.clone(),
            timestamp: self.timestamp,
            chat_name: self.chat_name.clone(),
        }
    }
}

/// Single source of truth for the queuing notification text.
pub(crate) const QUEUING_NOTIFICATION_TEXT: &str = "⏳ 正在排队...";

/// Outcome of handling an inbound message.
#[derive(Debug)]
pub enum HandleResult {
    MessageQueued(String), // enqueued (session busy), carries notification text
    /// An LLM call has been spawned and will run asynchronously.
    LlmStarted,
    /// An approval command was processed (approve/deny).
    ApprovalProcessed,
    SlashHandled, // slash command dispatched
    /// An error occurred during message handling, carries error description.
    Error(String),
}

/// Gateway-layer LLM session handler with busy/pending state management.
pub struct SessionMessageHandler {
    pub(super) session_manager: Arc<SessionManager>,
    pub(super) fallback_client: Arc<UnifiedFallbackClient>,
    pub(super) output_tx: OutputTx,
    pub(super) compaction_service: Arc<tokio::sync::Mutex<CompactionService>>,
    /// Concrete [`ActiveSearcherLlmCaller`] for the active-searcher pipeline.
    ///
    /// The active-searcher uses its own narrow [`ActiveSearchLlm`][closeclaw_memory::active_searcher_llm::ActiveSearchLlm]
    /// trait (with `complete()`) rather than the main
    /// [`closeclaw_common::LlmCaller`] trait. This field provides the
    /// concrete wrapper needed by the searcher pipeline without
    /// exposing `UnifiedFallbackClient` as a direct dependency.
    pub(super) fallback_llm_caller: Arc<ActiveSearcherLlmCaller>,
    /// Optional back-reference to the owning [`Gateway`] (weak).
    ///
    /// When set, `handle_message_with_gateway` can route streaming LLM
    /// output through [`Gateway::send_outbound_streaming`]. When `None`
    /// (default in tests), the handler still works for non-streaming
    /// paths; `handle_message_with_gateway` is the only entry point that
    /// can consume a streaming session and it requires this ref.
    pub(super) gateway: Option<Arc<std::sync::Weak<Gateway>>>,
    /// Shutdown handle for busy-count tracking across components.
    ///
    /// Components increment the busy count before starting async work
    /// and decrement when complete. The shutdown drain waits for the
    /// count to reach zero before finalizing.
    pub(super) shutdown_handle: Option<Arc<ShutdownHandle>>,
    /// Path to the SQLite database file used by the active-searcher.
    /// When set, `dispatch_llm_call` spawns a background searcher task.
    pub(super) memory_db_path: Option<std::path::PathBuf>,
    /// Knowledge base for model context window lookups.
    ///
    /// When set, compaction threshold checks use the knowledge base's
    /// context window for the model instead of the hardcoded table.
    pub(super) model_knowledge: Option<ProviderModelKnowledge>,
    /// Metrics emitter for operational metrics (cache breaks, etc.).
    pub(super) metrics_emitter: Option<Arc<dyn closeclaw_common::MetricsEmitter>>,
    /// Dedup flag for token warning notifications.
    ///
    /// Set to `true` after the first warning message is sent in a
    /// warning interval, reset to `false` when the state returns to
    /// Normal. Prevents duplicate warning messages within the same
    /// threshold interval.
    pub(super) has_warned: Arc<std::sync::Mutex<bool>>,
    /// Dedup flag for circuit-breaker notification messages.
    ///
    /// Set to `true` after the first "auto compact suspended" message is
    /// sent when the breaker trips, reset to `false` when the breaker
    /// resets (manual compact success). Prevents duplicate messages
    /// across consecutive auto-compact checks while the breaker is active.
    pub(super) has_circuit_break_notified: Arc<std::sync::Mutex<bool>>,
}

// ── Construction ──
impl SessionMessageHandler {
    /// Create a new handler with an output channel for streaming responses.
    pub fn new(
        session_manager: Arc<SessionManager>,
        fallback_client: Arc<UnifiedFallbackClient>,
        output_tx: mpsc::Sender<(String, Vec<ContentBlock>)>,
        fallback_llm_caller: Arc<ActiveSearcherLlmCaller>,
        compact_config: CompactConfig,
    ) -> Self {
        Self {
            session_manager,
            fallback_client,
            output_tx: Arc::new(RwLock::new(Some(output_tx))),
            compaction_service: Arc::new(tokio::sync::Mutex::new(CompactionService::new(
                compact_config,
            ))),
            fallback_llm_caller,
            gateway: None,
            shutdown_handle: None,
            memory_db_path: None,
            model_knowledge: None,
            metrics_emitter: None,
            has_warned: Arc::new(std::sync::Mutex::new(false)),
            has_circuit_break_notified: Arc::new(std::sync::Mutex::new(false)),
        }
    }
    /// Create a new handler without an output channel (used in tests).
    pub fn new_no_output(
        session_manager: Arc<SessionManager>,
        fallback_client: Arc<UnifiedFallbackClient>,
        fallback_llm_caller: Arc<ActiveSearcherLlmCaller>,
        compact_config: CompactConfig,
    ) -> Self {
        Self {
            session_manager,
            fallback_client,
            output_tx: Arc::new(RwLock::new(None)),
            compaction_service: Arc::new(tokio::sync::Mutex::new(CompactionService::new(
                compact_config,
            ))),
            fallback_llm_caller,
            gateway: None,
            shutdown_handle: None,
            memory_db_path: None,
            model_knowledge: None,
            metrics_emitter: None,
            has_warned: Arc::new(std::sync::Mutex::new(false)),
            has_circuit_break_notified: Arc::new(std::sync::Mutex::new(false)),
        }
    }
    /// Attach a back-reference (weak) to the owning [`Gateway`].
    ///
    /// Once set, [`handle_message_with_gateway`](Self::handle_message_with_gateway)
    /// can route streaming LLM output through
    /// [`Gateway::send_outbound_streaming`].
    pub fn with_gateway_ref(mut self, gateway: std::sync::Weak<Gateway>) -> Self {
        self.gateway = Some(Arc::new(gateway));
        self
    }

    /// Set the shutdown handle for busy-count tracking.
    ///
    /// When set, the handler increments the busy count before starting
    /// async work and decrements when complete. The shutdown drain
    /// waits for the count to reach zero before finalizing.
    pub fn with_shutdown_handle(mut self, handle: Arc<ShutdownHandle>) -> Self {
        self.shutdown_handle = Some(handle);
        self
    }

    /// Set the SQLite database path for the active-searcher.
    ///
    /// When set, `dispatch_llm_call` spawns a background active-searcher
    /// task that writes query results to the session's `memory_injection`
    /// slot for the next turn to consume.
    pub fn with_memory_db_path(mut self, path: std::path::PathBuf) -> Self {
        self.memory_db_path = Some(path);
        self
    }

    /// Set the model knowledge base for context window lookups.
    ///
    /// When set, compaction threshold checks use the knowledge base's
    /// context window for the model instead of the hardcoded table.
    pub fn with_model_knowledge(mut self, knowledge: ProviderModelKnowledge) -> Self {
        self.model_knowledge = Some(knowledge);
        self
    }
    /// Returns a reference to the model knowledge base, if set.
    pub fn model_knowledge(&self) -> Option<&ProviderModelKnowledge> {
        self.model_knowledge.as_ref()
    }

    /// Set the metrics emitter for operational metrics.
    pub fn with_metrics_emitter(
        mut self,
        emitter: Arc<dyn closeclaw_common::MetricsEmitter>,
    ) -> Self {
        self.metrics_emitter = Some(emitter);
        self
    }
}
// ── Message dispatch ──
impl SessionMessageHandler {
    /// Inject active children summary and yield reminder into the
    /// conversation transcript *before* the user message.
    ///
    /// This ensures the parent LLM sees which children are still
    /// running (with agent_id + task_summary) ahead of the user's
    /// latest input, satisfying the design-doc position constraint:
    /// > 插入位置在用户消息之前
    pub(crate) async fn inject_active_children_summary_if_needed(&self, session_id: &str) {
        let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        else {
            return;
        };
        let mut cs_write = cs.write().await;
        let summary = cs_write.active_children_summary();
        let yield_reminder = cs_write.spawn_guard_reminder();
        if summary.is_some() || yield_reminder.is_some() {
            let mut text = summary.unwrap_or_default();
            if let Some(reminder) = yield_reminder {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&reminder);
            }
            tracing::info!(
                session_id = %session_id,
                "injecting active children summary"
            );
            cs_write.inject_system_message(text);
        }
    }

    /// Handle an inbound message using default metadata.
    pub async fn handle_message(&self, session_id: &str, content: String) -> HandleResult {
        self.handle_message_with_meta(session_id, content, MessageMetadata::default_meta())
            .await
    }
    /// Handle an inbound message with explicit metadata.
    ///
    /// During active Waiting (yielding) state, `is_session_busy`
    /// returns false (llm_active and foreground_tool_active are both
    /// false), so the message flows through the normal path: inject
    /// into conversation history and dispatch LLM. No queueing.
    /// See `docs/design/session/session-execution.md` §Yield 机制.
    pub async fn handle_message_with_meta(
        &self,
        session_id: &str,
        content: String,
        meta: MessageMetadata,
    ) -> HandleResult {
        if self.session_manager.is_session_busy(session_id).await {
            self.enqueue_pending(session_id, content).await;
            return HandleResult::MessageQueued(QUEUING_NOTIFICATION_TEXT.to_string());
        }
        // Inject active children summary BEFORE the user message
        // (design-doc position constraint).
        self.inject_active_children_summary_if_needed(session_id)
            .await;
        // Persist user message before auto-compact so threshold estimation
        // includes the current message (design-doc data-flow: write → truncate → estimate).
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            cs.write().await.append_user_message(&content);
        }
        // Update last_user_activity_at and last_message_at for the user message.
        // Only user messages (not LLM responses, tool results, or system messages)
        // trigger this update (design-doc §Sweeper: last_user_activity_at).
        self.session_manager
            .update_checkpoint_user_activity(session_id)
            .await;
        self.check_and_run_auto_compact(session_id).await;
        self.dispatch_llm_call(session_id, content, meta, None, None)
            .await
    }
}
// ── Compaction ──
impl SessionMessageHandler {
    /// Send a text reply through the output channel (used by slash handlers).
    pub async fn send_reply(&self, text: String) {
        super::session_handler_compact::send_output(&self.output_tx, &text).await;
    }

    /// Reset the circuit-breaker notification dedup flag.
    ///
    /// Called after a successful manual compaction so that a subsequent
    /// auto-compact circuit-breaker trip re-injects the notification.
    pub fn reset_circuit_breaker_notification(&self) {
        *self
            .has_circuit_break_notified
            .lock()
            .expect("has_circuit_break_notified poisoned") = false;
    }
}

/// LlmCaller adapter for the active-searcher pipeline.
///
/// Wraps a [`closeclaw_common::LlmCaller`] so it can be used as a trait
/// object by the active-searcher pipeline in the memory crate.
pub struct ActiveSearcherLlmCaller {
    /// The common LLM caller used for prompt completion.
    pub caller: Arc<dyn closeclaw_common::LlmCaller>,
    /// Model identifier passed in the [`InternalRequest`].
    pub model: String,
}

#[async_trait::async_trait]
impl crate::memory::active_searcher_llm::ActiveSearchLlm for ActiveSearcherLlmCaller {
    async fn complete(
        &self,
        prompt: &str,
    ) -> Result<String, crate::memory::active_searcher::ActiveSearcherError> {
        use closeclaw_common::llm_types::{InternalMessage, InternalRequest};

        let request = InternalRequest {
            model: self.model.clone(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
                content_blocks: None,
                tool_call_id: None,
            }],
            temperature: 0.0,
            max_tokens: None,
            stream: false,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: closeclaw_common::ReasoningLevel::default(),
            turn_count: None,
        };

        match self.caller.call(request).await {
            Ok(response) => {
                let text = response
                    .content_blocks
                    .iter()
                    .filter_map(|b| match b {
                        closeclaw_common::processor::ContentBlock::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                Ok(text)
            }
            Err(e) => {
                let msg = e.to_string();
                Err(crate::memory::active_searcher::ActiveSearcherError::Llm(
                    msg,
                ))
            }
        }
    }
}

/// Replace session messages with boundary message on compaction.
///
/// Follows the design doc pipeline: replace messages → rebuild system
/// prompt → persist checkpoint → mark snapshot complete.
pub(crate) async fn apply_compact_result(
    sm: &SessionManager,
    session_id: &str,
    result: &CompactionResult,
    snapshot_id: Option<&str>,
) {
    let Some(cs) = sm.get_conversation_session(session_id).await else {
        return;
    };
    let boundary = closeclaw_session::llm_session::SessionMessage {
        role: "assistant".to_string(),
        content_blocks: vec![ContentBlock::Text(result.boundary_message.clone())],
        timestamp: chrono::Utc::now(),
    };
    {
        let mut cs = cs.write().await;
        cs.apply_transcript_op(TranscriptOp::Rewrite, vec![boundary]);
        cs.mark_compacted();
        // Explicitly preserve skill listing state across compaction.
        // Design doc: "对话压缩时受 Session 模块保护"
        // (see docs/design/skills/skill-listing-injection.md).
        // skill_listing_snapshot and activated_conditional_skills are
        // session-level fields that must survive the transcript rewrite
        // so the next turn can correctly recompute the incremental diff.
        cs.preserve_listing_on_compaction();
    }
    // Rebuild system prompt after compaction so skills stay fresh.
    // The write guard above is now dropped, so we can safely acquire
    // a write lock for the rebuild. This must happen before persisting
    // the checkpoint, per the design doc's compaction pipeline.
    tracing::info!(
        session_id = %session_id,
        event = "session_injection",
        trigger = "compaction",
        "rebuilding system prompt after compaction"
    );
    sm.rebuild_system_prompt_for_session(session_id).await;
    // Persist checkpoint after rebuild to protect plan_state.
    // system prompt is a runtime field not in the checkpoint, so
    // rebuilding first does not affect data consistency.
    sm.save_checkpoint_after_compact(session_id).await;
    // Mark the pre-compaction snapshot as complete — compaction
    // succeeded, so the snapshot is retained for potential rollback
    // rather than being cleared.
    if let Some(sid) = snapshot_id {
        sm.complete_pre_compaction_snapshot(session_id, sid).await;
    }
}
