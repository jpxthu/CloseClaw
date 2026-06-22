//! SessionMessageHandler - Gateway-layer LLM session manager with busy/pending state.
//!
//! This component implements the complete busy/pending messaging loop:
//! - idle message  → set busy → LLM call → clear busy → drain pending
//! - busy message  → enqueue pending
//!
//! `UnifiedFallbackClient::chat()` (non-streaming) is used for non-streaming LLM calls,
//! going through the full five-layer architecture (CacheAdapter → PluginPipeline →
//! Interpreter → Protocol → Provider).
//! The `output_tx` channel is used to surface LLM response text to callers.

use super::Gateway;
use crate::daemon::shutdown::ShutdownHandle;
use crate::gateway::session_manager::SessionManager;
use crate::gateway::system_prompt_inject::{
    build_dynamic_sections, build_full_system_prompt, split_static_dynamic,
};
use crate::llm::fallback::FallbackClient;
use crate::llm::session::ChatSession;
use crate::llm::session_state::LlmState;
use crate::llm::types::ContentBlock;
use crate::llm::types::UnifiedResponse;
use crate::llm::unified_fallback::UnifiedFallbackClient;
use crate::llm::{LLMError, Message as ChatMessage};
use crate::session::compaction::{
    execute_compact, CompactConfig, CompactionResult, CompactionService,
};
use crate::session::persistence::ReasoningLevel;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use super::OutputTx;
use tokio_util::sync::CancellationToken;

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
    pub(super) unified_fallback_client: Arc<UnifiedFallbackClient>,
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
}

// ── Construction ──
impl SessionMessageHandler {
    /// Create a new handler with an output channel for streaming responses.
    pub fn new(
        session_manager: Arc<SessionManager>,
        fallback_client: Arc<FallbackClient>,
        output_tx: mpsc::Sender<(String, Vec<ContentBlock>)>,
        unified_fallback_client: Arc<UnifiedFallbackClient>,
    ) -> Self {
        Self {
            session_manager,
            fallback_client,
            output_tx: Arc::new(RwLock::new(Some(output_tx))),
            compaction_service: Arc::new(std::sync::Mutex::new(CompactionService::new(
                CompactConfig::default(),
            ))),
            unified_fallback_client,
            gateway: None,
            shutdown_handle: None,
        }
    }
    /// Create a new handler without an output channel (used in tests).
    pub fn new_no_output(
        session_manager: Arc<SessionManager>,
        fallback_client: Arc<FallbackClient>,
        unified_fallback_client: Arc<UnifiedFallbackClient>,
    ) -> Self {
        Self {
            session_manager,
            fallback_client,
            output_tx: Arc::new(RwLock::new(None)),
            compaction_service: Arc::new(std::sync::Mutex::new(CompactionService::new(
                CompactConfig::default(),
            ))),
            unified_fallback_client,
            gateway: None,
            shutdown_handle: None,
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
        let Some((model, llm_messages)) =
            load_compact_inputs(&self.session_manager, session_id).await
        else {
            return;
        };
        let should_run = self
            .compaction_service
            .lock()
            .expect("compaction_service poisoned")
            .should_auto_compact(&llm_messages, &model);
        if !should_run {
            return;
        }
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
}
// ── LLM calling ──
impl SessionMessageHandler {
    /// Make a non-streaming LLM call with full system prompt injection.
    pub(super) async fn call_llm(
        unified_fallback_client: &Arc<UnifiedFallbackClient>,
        content: &str,
        meta: &MessageMetadata,
        session_manager: &Arc<SessionManager>,
        session_id: &str,
    ) -> Result<UnifiedResponse, crate::llm::LLMError> {
        // ── Static layer ───────────────────────────────────────────────
        let (
            static_prompt_opt,
            session_timestamp,
            turn_count,
            workdir_path,
            system_appends,
            reasoning_level,
        ) = if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            let cs_read = cs.read().await;
            (
                cs_read.system_prompt().map(|s| s.to_string()),
                Some(cs_read.session_created_at()),
                cs_read.turn_count(),
                cs_read.workdir().to_string_lossy().into_owned(),
                cs_read.system_appends().to_vec(),
                cs_read.reasoning_level(),
            )
        } else {
            (
                None,
                None,
                0,
                String::new(),
                Vec::new(),
                ReasoningLevel::default(),
            )
        };

        // ── Dynamic sections ───────────────────────────────────────────
        let dynamic_sections = build_dynamic_sections(
            meta,
            Some(workdir_path.as_str()),
            &system_appends,
            session_timestamp,
        );

        // ── Compose full prompt ─────────────────────────────────────────
        let overrides = session_manager.get_prompt_overrides().await;
        let full_prompt = build_full_system_prompt(
            static_prompt_opt.as_deref(),
            &dynamic_sections,
            overrides.as_ref(),
        );

        // ── Split static/dynamic for cache adapter ─────────────────────
        let (system_static, system_dynamic) = split_static_dynamic(&full_prompt);

        // ── Build InternalRequest with system + user messages ───────────
        let mut messages = vec![];
        if !full_prompt.is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: full_prompt,
            });
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
        });

        let internal_request = crate::llm::types::InternalRequest {
            model: String::new(),
            messages: messages
                .iter()
                .map(|m| crate::llm::types::InternalMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                })
                .collect(),
            temperature: 0.7,
            max_tokens: None,
            stream: false,
            extra_body: Default::default(),
            system_static,
            system_dynamic,
            system_blocks: None,
            session_id: Some(session_id.to_string()),
            reasoning_level,
            turn_count: Some(turn_count),
        };

        // Acquire this session's cancellation token so an in-flight
        // request can be aborted by a cascade stop (parent or local).
        // If the conversation session is gone we fall back to a never-
        // cancelled token so the request still completes normally.
        let cancel_token: CancellationToken =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                cs.read().await.cancel_token().clone()
            } else {
                CancellationToken::new()
            };

        // Race the LLM call against the cancel signal.
        tokio::select! {
            res = unified_fallback_client.chat(internal_request) => res,
            _ = cancel_token.cancelled() => {
                // Restore idle state so the session can accept the next
                // request. The actual cascade-cleanup (tool/child
                // handles, states) is handled by `stop()`.
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    cs.read().await.set_llm_state(LlmState::Idle);
                }
                tracing::info!(session_id = %session_id, "LLM request cancelled");
                Err(LLMError::Cancelled)
            }
        }
    }
}
// ── Compaction helpers ──
fn flatten_content_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Text(t) => t.as_str(),
            ContentBlock::Thinking(t) => t.as_str(),
            ContentBlock::ToolUse { input, .. } => input.as_str(),
            ContentBlock::ToolResult { content, .. } => content.as_str(),
            _ => "",
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_compact_messages(messages: &[crate::llm::session::SessionMessage]) -> Vec<ChatMessage> {
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
async fn apply_compact_result(
    sm: &Arc<SessionManager>,
    session_id: &str,
    result: &CompactionResult,
) {
    let Some(cs) = sm.get_conversation_session(session_id).await else {
        return;
    };
    let boundary = crate::llm::session::SessionMessage {
        role: "assistant".to_string(),
        content_blocks: vec![ContentBlock::Text(result.boundary_message.clone())],
        timestamp: chrono::Utc::now(),
    };
    {
        let mut cs = cs.write().await;
        cs.replace_messages(vec![boundary]);
    }
    // Rebuild system prompt after compaction so skills stay fresh.
    // The write guard above is now dropped, so rebuild_system_prompt
    // can safely acquire its own write lock.
    sm.rebuild_system_prompt(session_id).await;
}

async fn send_output(output_tx: &OutputTx, text: &str) {
    let guard = output_tx.read().await;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send((text.to_string(), vec![])).await;
    }
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
    result: Result<CompactionResult, crate::session::compaction::CompactionError>,
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
            svc.lock()
                .expect("compaction_service poisoned")
                .record_failure();
        }
    }
}
