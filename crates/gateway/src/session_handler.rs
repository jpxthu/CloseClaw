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
use crate::llm_caller_impl::execute_compact;
use crate::session_manager::SessionManager;
use crate::shutdown_handle::ShutdownHandle;
use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::types::ContentBlock;
use closeclaw_llm::Message as ChatMessage;
use closeclaw_session::compaction::{
    CompactConfig, CompactionMessage, CompactionResult, CompactionService, TokenWarningState,
};
use closeclaw_session::llm_session::ChatSession;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use super::OutputTx;

/// Metadata about an inbound message, passed through the handling pipeline.
pub struct MessageMetadata {
    /// Open ID of the message sender.
    pub sender_id: String,
    /// Channel identifier (e.g. "feishu", "telegram").
    pub channel: String,
    /// Unix timestamp (seconds) when the message was created.
    pub timestamp: i64,
}

impl MessageMetadata {
    pub fn default_meta() -> Self {
        Self {
            sender_id: String::new(),
            channel: String::new(),
            timestamp: chrono::Utc::now().timestamp(),
        }
    }
}

/// Outcome of handling an inbound message.
#[derive(Debug)]
pub enum HandleResult {
    MessageQueued, // enqueued (session busy)
    /// An LLM call has been spawned and will run asynchronously.
    LlmStarted,
    /// An approval command was processed (approve/deny).
    ApprovalProcessed,
    SlashHandled, // slash command dispatched
}

/// Gateway-layer LLM session handler with busy/pending state management.
pub struct SessionMessageHandler {
    pub(super) session_manager: Arc<SessionManager>,
    pub(super) fallback_client: Arc<FallbackClient>,
    pub(super) output_tx: OutputTx,
    pub(super) compaction_service: Arc<std::sync::Mutex<CompactionService>>,
    /// Concrete [`ActiveSearcherLlmCaller`] for the active-searcher pipeline.
    ///
    /// The active-searcher uses its own [`LlmCaller`][closeclaw_memory::active_searcher_llm::LlmCaller]
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
}

// ── Construction ──
impl SessionMessageHandler {
    /// Create a new handler with an output channel for streaming responses.
    pub fn new(
        session_manager: Arc<SessionManager>,
        fallback_client: Arc<FallbackClient>,
        output_tx: mpsc::Sender<(String, Vec<ContentBlock>)>,
        fallback_llm_caller: Arc<ActiveSearcherLlmCaller>,
    ) -> Self {
        Self {
            session_manager,
            fallback_client,
            output_tx: Arc::new(RwLock::new(Some(output_tx))),
            compaction_service: Arc::new(std::sync::Mutex::new(CompactionService::new(
                CompactConfig::default(),
            ))),
            fallback_llm_caller,
            gateway: None,
            shutdown_handle: None,
            memory_db_path: None,
        }
    }
    /// Create a new handler without an output channel (used in tests).
    pub fn new_no_output(
        session_manager: Arc<SessionManager>,
        fallback_client: Arc<FallbackClient>,
        fallback_llm_caller: Arc<ActiveSearcherLlmCaller>,
    ) -> Self {
        Self {
            session_manager,
            fallback_client,
            output_tx: Arc::new(RwLock::new(None)),
            compaction_service: Arc::new(std::sync::Mutex::new(CompactionService::new(
                CompactConfig::default(),
            ))),
            fallback_llm_caller,
            gateway: None,
            shutdown_handle: None,
            memory_db_path: None,
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
}
// ── Message dispatch ──
impl SessionMessageHandler {
    /// Handle an inbound message using default metadata.
    pub async fn handle_message(&self, session_id: &str, content: String) -> HandleResult {
        self.handle_message_with_meta(session_id, content, MessageMetadata::default_meta())
            .await
    }
    /// Handle an inbound message with explicit metadata.
    pub async fn handle_message_with_meta(
        &self,
        session_id: &str,
        content: String,
        meta: MessageMetadata,
    ) -> HandleResult {
        if self.session_manager.is_session_busy(session_id).await {
            self.enqueue_pending(session_id, content).await;
            return HandleResult::MessageQueued;
        }
        // Reject new requests when context window is nearly full.
        if is_blocking_state(&self.compaction_service, &self.session_manager, session_id).await {
            send_output(
                &self.output_tx,
                "Context window nearly full. Please run /compact to compress the session.",
            )
            .await;
            return HandleResult::MessageQueued;
        }
        self.check_and_run_auto_compact(session_id).await;
        self.dispatch_llm_call(session_id, content, meta, None, None)
            .await
    }
}
// ── Compaction ──
impl SessionMessageHandler {
    /// Handle `/compact [instruction]` manual compaction.
    pub async fn handle_compact_command(&self, session_id: &str, content: &str) -> HandleResult {
        let instruction: Option<String> = content
            .strip_prefix("/compact")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let Some((model, llm_messages)) =
            load_compact_inputs(&self.session_manager, session_id).await
        else {
            tracing::warn!(session_id, "session not found for /compact");
            return HandleResult::MessageQueued;
        };
        let fc = Arc::clone(&self.fallback_client);
        let output_tx = Arc::clone(&self.output_tx);
        let svc = Arc::clone(&self.compaction_service);
        let sm = Arc::clone(&self.session_manager);
        let sid = session_id.to_string();
        tokio::spawn(async move {
            run_manual_compact(
                sm,
                fc,
                output_tx,
                svc,
                sid,
                model,
                llm_messages,
                instruction,
            )
            .await;
        });
        HandleResult::LlmStarted
    }

    /// Send a text reply through the output channel (used by slash handlers).
    pub async fn send_reply(&self, text: String) {
        send_output(&self.output_tx, &text).await;
    }

    pub(super) async fn check_and_run_auto_compact(&self, session_id: &str) {
        let Some((model, mut llm_messages)) =
            load_compact_inputs(&self.session_manager, session_id).await
        else {
            return;
        };
        // Truncate history before token estimation if configured.
        {
            let svc = self
                .compaction_service
                .lock()
                .expect("compaction_service poisoned");
            if let Some(max) = svc.config().max_history_messages {
                if llm_messages.len() > max {
                    let drain = llm_messages.len() - max;
                    llm_messages.drain(..drain);
                }
            }
        }
        let compaction_msgs: Vec<CompactionMessage> = llm_messages
            .iter()
            .map(|m| CompactionMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();
        let tokens = closeclaw_session::compaction::estimate_messages_tokens(&compaction_msgs);
        let warning = {
            let svc = self
                .compaction_service
                .lock()
                .expect("compaction_service poisoned");
            svc.token_warning_state(tokens, &model)
        };
        match warning {
            TokenWarningState::Normal => (),
            TokenWarningState::Warning => {
                tracing::warn!(
                    session_id,
                    tokens,
                    model = %model,
                    "token warning: approaching context limit"
                );
            }
            TokenWarningState::AutoCompactTriggered => {
                {
                    let breaker = self
                        .compaction_service
                        .lock()
                        .expect("compaction_service poisoned");
                    if breaker.consecutive_failures() >= breaker.config().max_consecutive_failures {
                        return;
                    }
                }
                self.session_manager
                    .save_pre_compaction_snapshot(session_id)
                    .await;
                let result =
                    execute_compact(&llm_messages, &self.fallback_client, &model, None, true).await;
                finalize_auto_compact(
                    &self.session_manager,
                    &self.compaction_service,
                    session_id,
                    result,
                )
                .await;
            }
            TokenWarningState::Blocking => {
                tracing::warn!(
                    session_id,
                    "auto compact: blocking state, skipping (handled by caller)"
                );
            }
        }
    }
}

/// LlmCaller adapter for `UnifiedFallbackClient`.
///
/// Wraps the unified fallback client so it can be used as a trait object
/// by the active-searcher pipeline.
pub struct ActiveSearcherLlmCaller {
    #[allow(dead_code)]
    pub client: Arc<closeclaw_llm::unified_fallback::UnifiedFallbackClient>,
    #[allow(dead_code)]
    pub model: String,
}

#[async_trait::async_trait]
impl crate::memory::active_searcher_llm::LlmCaller for ActiveSearcherLlmCaller {
    async fn complete(
        &self,
        prompt: &str,
    ) -> Result<String, crate::memory::active_searcher::ActiveSearcherError> {
        use closeclaw_llm::types::InternalRequest;

        let request = InternalRequest {
            model: self.model.clone(),
            messages: vec![closeclaw_llm::types::InternalMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
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
            reasoning_level: closeclaw_session::persistence::ReasoningLevel::default(),
            turn_count: None,
        };

        match self.client.chat(request).await {
            Ok(response) => {
                let text = response
                    .content_blocks
                    .iter()
                    .filter_map(|b| match b {
                        closeclaw_llm::types::ContentBlock::Text(t) => Some(t.as_str()),
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
// ── Compaction helpers ──
pub(crate) fn flatten_content_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Text(t) => t.as_str(),
            ContentBlock::Thinking { thinking: t, .. } => t.as_str(),
            ContentBlock::ToolUse { input, .. } => input.as_str(),
            ContentBlock::ToolResult { content, .. } => content.as_str(),
            _ => "",
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_compact_messages(
    messages: &[closeclaw_session::llm_session::SessionMessage],
) -> Vec<ChatMessage> {
    messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .map(|m| ChatMessage {
            role: m.role.clone(),
            content: flatten_content_blocks(&m.content_blocks),
        })
        .collect()
}

/// Replace session messages with boundary message on compaction.
///
/// After replacing messages, persists the checkpoint to ensure
/// plan_state (and other checkpoint fields) survive a subsequent crash.
async fn apply_compact_result(
    sm: &Arc<SessionManager>,
    session_id: &str,
    result: &CompactionResult,
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
        cs.replace_messages(vec![boundary]);
    }
    // Persist checkpoint immediately after compaction to protect plan_state.
    // This ensures plan_state survives a crash before the next periodic flush.
    sm.save_checkpoint_after_compact(session_id).await;
    // Clear the pre-compaction snapshot — compaction succeeded, so
    // the backup is no longer needed.
    sm.clear_pre_compaction_snapshot(session_id).await;
    // Rebuild system prompt after compaction so skills stay fresh.
    // The write guard above is now dropped, so we can safely acquire
    // a write lock for the rebuild.
    sm.rebuild_system_prompt_for_session(session_id).await;
}

pub(crate) async fn send_output(output_tx: &OutputTx, text: &str) {
    let guard = output_tx.read().await;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send((text.to_string(), vec![])).await;
    }
}
/// Check if the session is in a blocking state (context window nearly full).
pub(crate) async fn is_blocking_state(
    svc: &Arc<std::sync::Mutex<CompactionService>>,
    sm: &Arc<SessionManager>,
    session_id: &str,
) -> bool {
    let Some((model, llm_messages)) = load_compact_inputs(sm, session_id).await else {
        return false;
    };
    let compaction_msgs: Vec<CompactionMessage> = llm_messages
        .iter()
        .map(|m| CompactionMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect();
    let tokens = closeclaw_session::compaction::estimate_messages_tokens(&compaction_msgs);
    matches!(
        svc.lock()
            .expect("compaction_service poisoned")
            .token_warning_state(tokens, &model),
        TokenWarningState::Blocking
    )
}

/// Load compaction inputs: (model, llm_messages). Returns None if session not found.
async fn load_compact_inputs(
    sm: &Arc<SessionManager>,
    session_id: &str,
) -> Option<(String, Vec<ChatMessage>)> {
    let cs = sm.get_conversation_session(session_id).await?;
    let cs_read = cs.read().await;
    let model = cs_read.model().to_string();
    let llm_msgs = build_compact_messages(ChatSession::messages(&*cs_read));
    Some((model, llm_msgs))
}
/// Run manual `/compact` invocation.
#[allow(clippy::too_many_arguments)]
async fn run_manual_compact(
    sm: Arc<SessionManager>,
    fc: Arc<FallbackClient>,
    output_tx: OutputTx,
    svc: Arc<std::sync::Mutex<CompactionService>>,
    sid: String,
    model: String,
    llm_messages: Vec<ChatMessage>,
    instruction: Option<String>,
) {
    // Save a snapshot before compaction so we can rollback on failure.
    sm.save_pre_compaction_snapshot(&sid).await;
    let result = execute_compact(&llm_messages, &fc, &model, instruction.as_deref(), false).await;
    match result {
        Ok(r) => {
            apply_compact_result(&sm, &sid, &r).await;
            send_output(&output_tx, &r.message).await;
            svc.lock()
                .expect("compaction_service poisoned")
                .record_success();
        }
        Err(e) => {
            tracing::warn!(session_id = %sid, error = %e, "manual compact failed");
            // Rollback to the pre-compaction snapshot.
            sm.rollback_compaction(&sid).await;
            svc.lock()
                .expect("compaction_service poisoned")
                .record_failure();
            send_output(&output_tx, &format!("Compact failed: {}", e)).await;
        }
    }
}
/// Finalize auto-compact result.
async fn finalize_auto_compact(
    sm: &Arc<SessionManager>,
    svc: &Arc<std::sync::Mutex<CompactionService>>,
    session_id: &str,
    result: Result<CompactionResult, closeclaw_session::compaction::CompactionError>,
) {
    match result {
        Ok(r) => {
            apply_compact_result(sm, session_id, &r).await;
            svc.lock()
                .expect("compaction_service poisoned")
                .record_success();
        }
        Err(e) => {
            tracing::warn!(session_id, error = %e, "auto compact failed");
            // Rollback to the pre-compaction snapshot.
            sm.rollback_compaction(session_id).await;
            svc.lock()
                .expect("compaction_service poisoned")
                .record_failure();
        }
    }
}
