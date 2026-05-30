//! SessionMessageHandler - Gateway-layer LLM session manager with busy/pending state.
//!
//! This component implements the complete busy/pending messaging loop:
//! - idle message  → set busy → LLM call → clear busy → drain pending
//! - busy message  → enqueue pending
//!
//! `FallbackClient::chat()` (non-streaming) is used for all LLM calls.
//! The `output_tx` channel is used to surface LLM response text to callers.

use crate::gateway::session_manager::SessionManager;
use crate::gateway::system_prompt_inject::{build_dynamic_sections, build_full_system_prompt};
use crate::llm::fallback::FallbackClient;
use crate::llm::session::ChatSession;
use crate::llm::types::ContentBlock;
use crate::llm::{ChatRequest, ChatResponse, Message as ChatMessage};
use crate::session::compaction::{
    execute_compact, CompactConfig, CompactionResult, CompactionService,
};
use crate::session::persistence::PendingMessage;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

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
    /// Create a default `MessageMetadata` with empty sender/channel and current time.
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
    /// The message was enqueued because the session is busy.
    MessageQueued,
    /// An LLM call has been spawned and will run asynchronously.
    LlmStarted,
}

/// Gateway-layer LLM session handler with busy/pending state management.
pub struct SessionMessageHandler {
    session_manager: Arc<SessionManager>,
    fallback_client: Arc<FallbackClient>,
    output_tx: Arc<RwLock<Option<mpsc::Sender<String>>>>,
    compaction_service: Arc<std::sync::Mutex<CompactionService>>,
}

// ── Construction ───────────────────────────────────────────────────────────

impl SessionMessageHandler {
    /// Create a new handler with an output channel for streaming responses.
    pub fn new(
        session_manager: Arc<SessionManager>,
        fallback_client: Arc<FallbackClient>,
        output_tx: mpsc::Sender<String>,
    ) -> Self {
        Self {
            session_manager,
            fallback_client,
            output_tx: Arc::new(RwLock::new(Some(output_tx))),
            compaction_service: Arc::new(std::sync::Mutex::new(CompactionService::new(
                CompactConfig::default(),
            ))),
        }
    }

    /// Create a new handler without an output channel (used in tests).
    pub fn new_no_output(
        session_manager: Arc<SessionManager>,
        fallback_client: Arc<FallbackClient>,
    ) -> Self {
        Self {
            session_manager,
            fallback_client,
            output_tx: Arc::new(RwLock::new(None)),
            compaction_service: Arc::new(std::sync::Mutex::new(CompactionService::new(
                CompactConfig::default(),
            ))),
        }
    }
}

// ── Message dispatch ───────────────────────────────────────────────────────

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
        if content.starts_with("/compact") {
            return self.handle_compact_command(session_id, &content).await;
        }
        if self.session_manager.is_session_busy(session_id).await {
            self.enqueue_pending(session_id, content).await;
            return HandleResult::MessageQueued;
        }
        self.check_and_run_auto_compact(session_id).await;
        self.dispatch_llm_call(session_id, content, meta).await
    }

    async fn dispatch_llm_call(
        &self,
        session_id: &str,
        content: String,
        meta: MessageMetadata,
    ) -> HandleResult {
        self.set_busy(session_id, true).await;
        let session_id = session_id.to_string();
        let content_for_task = content;
        let sm = Arc::clone(&self.session_manager);
        let fc = Arc::clone(&self.fallback_client);
        let output_tx = Arc::clone(&self.output_tx);

        tokio::spawn(async move {
            let result = Self::call_llm(&fc, &content_for_task, &meta, &sm, &session_id).await;
            Self::finish_llm(&sm, &session_id, result, &fc, &output_tx).await;
        });

        HandleResult::LlmStarted
    }

    async fn set_busy(&self, session_id: &str, busy: bool) {
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            let cs = cs.write().await;
            cs.set_llm_busy(busy);
        }
    }

    async fn enqueue_pending(&self, session_id: &str, content: String) {
        let msg = PendingMessage::new(
            format!("pending-{}", chrono::Utc::now().timestamp_millis()),
            content,
        );
        if let Err(e) = self
            .session_manager
            .push_pending_message(session_id, msg)
            .await
        {
            tracing::warn!(session_id, error = %e, "failed to enqueue pending message");
        }
    }
}

// ── Compaction ─────────────────────────────────────────────────────────────

impl SessionMessageHandler {
    /// Handle `/compact [instruction]` manual compaction.
    async fn handle_compact_command(&self, session_id: &str, content: &str) -> HandleResult {
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

    async fn check_and_run_auto_compact(&self, session_id: &str) {
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
            execute_compact(&llm_messages, &*self.fallback_client, &model, None, true).await;
        finalize_auto_compact(
            &self.session_manager,
            &self.compaction_service,
            session_id,
            result,
        )
        .await;
    }
}

// ── Dynamic section building ──────────────────────────────────────────────

// ── LLM calling ───────────────────────────────────────────────────────────

impl SessionMessageHandler {
    /// Make a non-streaming LLM call with full system prompt injection.
    async fn call_llm(
        fallback_client: &Arc<FallbackClient>,
        content: &str,
        meta: &MessageMetadata,
        session_manager: &Arc<SessionManager>,
        session_id: &str,
    ) -> Result<String, crate::llm::LLMError> {
        // ── Static layer ───────────────────────────────────────────────
        let (static_prompt_opt, turn_count) =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                let cs_read = cs.read().await;
                (
                    cs_read.system_prompt().map(|s| s.to_string()),
                    cs_read.turn_count(),
                )
            } else {
                (None, 0)
            };

        // ── Dynamic sections ───────────────────────────────────────────
        let dynamic_sections = build_dynamic_sections(turn_count, meta);

        // ── Compose full prompt ─────────────────────────────────────────
        let full_prompt = build_full_system_prompt(static_prompt_opt.as_deref(), &dynamic_sections);

        // ── Build ChatRequest with system + user messages ───────────────
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

        let request = ChatRequest {
            model: String::new(),
            messages,
            temperature: 0.7,
            max_tokens: None,
        };
        let response: ChatResponse = fallback_client.chat(request).await?;
        Ok(response.content)
    }

    /// Clear busy flag, send output, and drain pending messages.
    async fn finish_llm(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<String, crate::llm::LLMError>,
        fallback_client: &Arc<FallbackClient>,
        output_tx: &Arc<RwLock<Option<mpsc::Sender<String>>>>,
    ) {
        Self::clear_busy_and_send(session_manager, session_id, result, output_tx).await;
        Self::drain_pending_loop(session_manager, session_id, fallback_client, output_tx).await;
    }

    async fn clear_busy_and_send(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<String, crate::llm::LLMError>,
        output_tx: &Arc<RwLock<Option<mpsc::Sender<String>>>>,
    ) {
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            let cs = cs.write().await;
            cs.set_llm_busy(false);
        }

        match result {
            Ok(text) => {
                let guard = output_tx.read().await;
                if let Some(tx) = guard.as_ref() {
                    let _ = tx.send(text).await;
                }
            }
            Err(err) => {
                tracing::warn!(session_id, error = %err, "LLM call failed");
            }
        }
    }

    async fn drain_pending_loop(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        fallback_client: &Arc<FallbackClient>,
        output_tx: &Arc<RwLock<Option<mpsc::Sender<String>>>>,
    ) {
        loop {
            // Get next pending message
            let Some(pending) = session_manager.pop_pending_message(session_id).await else {
                break;
            };

            // Set busy before calling LLM
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                let cs = cs.write().await;
                cs.set_llm_busy(true);
            }

            let meta = MessageMetadata::default_meta();
            let result = Self::call_llm(
                fallback_client,
                &pending.content,
                &meta,
                session_manager,
                session_id,
            )
            .await;
            Self::clear_busy_and_send(session_manager, session_id, result, output_tx).await;
        }
    }
}

// ── Compaction helpers ─────────────────────────────────────────────────────

fn flatten_content_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            ContentBlock::Thinking(t) => Some(t.as_str()),
            ContentBlock::ToolUse { input, .. } => Some(input.as_str()),
            ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
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
    let mut cs = cs.write().await;
    cs.replace_messages(vec![boundary]);
}

async fn send_output(output_tx: &Arc<RwLock<Option<mpsc::Sender<String>>>>, text: &str) {
    let guard = output_tx.read().await;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send(text.to_string()).await;
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
    output_tx: Arc<RwLock<Option<mpsc::Sender<String>>>>,
    svc: Arc<std::sync::Mutex<CompactionService>>,
    sid: String,
    model: String,
    llm_messages: Vec<ChatMessage>,
    instruction: Option<String>,
) {
    let result = execute_compact(&llm_messages, &*fc, &model, instruction.as_deref(), false).await;
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
