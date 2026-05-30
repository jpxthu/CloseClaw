//! SessionMessageHandler - Gateway-layer LLM session manager with busy/pending state.
//!
//! This component implements the complete busy/pending messaging loop:
//! - idle message  → set busy → LLM call → clear busy → drain pending
//! - busy message  → enqueue pending
//!
//! `FallbackClient::chat()` (non-streaming) is used for all LLM calls.
//! The `output_tx` channel is used to surface LLM response text to callers.

use crate::gateway::session_manager::SessionManager;
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

/// Outcome of handling an inbound message.
#[derive(Debug)]
pub enum HandleResult {
    /// Message was queued behind an in-progress LLM call.
    MessageQueued,
    /// LLM call was started; response text will be sent to `output_tx`.
    LlmStarted,
}

/// Gateway-layer LLM session manager.
///
/// Manages `llm_busy` and `pending_messages` per session. Delegates LLM calls
/// to `FallbackClient` and surfaces responses via `output_tx`.
pub struct SessionMessageHandler {
    session_manager: Arc<SessionManager>,
    fallback_client: Arc<FallbackClient>,
    /// Channel for LLM response text. `None` means responses are discarded.
    output_tx: Arc<RwLock<Option<mpsc::Sender<String>>>>,
    /// Compaction service for auto-compact and manual /compact.
    compaction_service: Arc<std::sync::Mutex<CompactionService>>,
}

// ── Construction ───────────────────────────────────────────────────────────

impl SessionMessageHandler {
    /// Create a new handler with an output channel.
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

    /// Create a handler with no output channel (responses are silently dropped).
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
    /// Handle an inbound user message for a session.
    ///
    /// # Behaviour
    /// - `/compact [instruction]` → manual compaction, returns stats
    /// - idle → auto-compact check, then LLM call
    /// - busy → enqueues message
    pub async fn handle_message(&self, session_id: &str, content: String) -> HandleResult {
        if content.starts_with("/compact") {
            return self.handle_compact_command(session_id, &content).await;
        }
        if self.session_manager.is_session_busy(session_id).await {
            self.enqueue_pending(session_id, content).await;
            return HandleResult::MessageQueued;
        }
        self.check_and_run_auto_compact(session_id).await;
        self.dispatch_llm_call(session_id, content).await
    }

    /// Dispatch a normal LLM call (set busy → spawn → finish).
    async fn dispatch_llm_call(&self, session_id: &str, content: String) -> HandleResult {
        self.set_busy(session_id, true).await;
        let session_id = session_id.to_string();
        let content_for_task = content;
        let sm = Arc::clone(&self.session_manager);
        let fc = Arc::clone(&self.fallback_client);
        let output_tx = Arc::clone(&self.output_tx);

        tokio::spawn(async move {
            let result = Self::call_llm(&fc, &content_for_task).await;
            Self::finish_llm(&sm, &session_id, result, &fc, &output_tx).await;
        });

        HandleResult::LlmStarted
    }

    /// Set busy state on a conversation session.
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

    /// Enqueue a pending message for a busy session.
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
    /// Handle `/compact [instruction]` manual compaction command.
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

    /// Check auto-compact threshold and run compaction if needed.
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

// ── LLM calling ───────────────────────────────────────────────────────────

impl SessionMessageHandler {
    /// Make a single non-streaming LLM call and return the text content.
    async fn call_llm(
        fallback_client: &Arc<FallbackClient>,
        content: &str,
    ) -> Result<String, crate::llm::LLMError> {
        let request = ChatRequest {
            model: String::new(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: content.to_string(),
            }],
            temperature: 0.7,
            max_tokens: None,
        };
        let response: ChatResponse = fallback_client.chat(request).await?;
        Ok(response.content)
    }

    /// Called when an LLM call finishes (success or failure).
    /// Clears busy flag, sends output, and drains pending messages.
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

    /// Clear busy flag and send output (called for each LLM completion).
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

    /// Drain pending messages until the queue is empty (loop-based, no recursion).
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

            let result = Self::call_llm(fallback_client, &pending.content).await;
            Self::clear_busy_and_send(session_manager, session_id, result, output_tx).await;
        }
    }
}

// ── Compaction helpers ─────────────────────────────────────────────────────

/// Flatten content blocks into plain text.
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

/// Build the message list for compaction from session messages.
/// Filters out system-role messages and flattens content blocks.
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

/// Apply compaction result: replace session messages with boundary message.
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

/// Send text through the output channel (silently ignores closed/disconnected).
async fn send_output(output_tx: &Arc<RwLock<Option<mpsc::Sender<String>>>>, text: &str) {
    let guard = output_tx.read().await;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send(text.to_string()).await;
    }
}

/// Load the inputs needed for compaction from a session: (model, llm_messages).
/// Returns `None` when the session does not exist.
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

/// Run a manual `/compact` invocation: execute LLM, replace messages,
/// send stats or error to the output channel, and record success/failure.
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

/// Finalize an auto-compact result: replace messages on success, record
/// success/failure in the circuit-breaker.
/// Finalize an auto-compact result: replace messages on success, record
/// success/failure in the circuit-breaker.
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

#[cfg(test)]
mod session_handler_tests;
