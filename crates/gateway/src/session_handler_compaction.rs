//! Compaction helper functions for `SessionMessageHandler`.
//!
//! Extracted from `session_handler.rs` to keep the handler file under
//! the 500-line project limit. These helpers support the
//! `/compact` slash command and the auto-compaction check that runs
//! before each LLM call:
//! - `flatten_content_blocks` / `build_compact_messages` — flatten
//!   a session's message history into the LLM-friendly
//!   `ChatMessage` list used as compaction input.
//! - `apply_compact_result` — replace the session's messages with
//!   the compaction boundary message and rebuild the system prompt.
//! - `send_output` — push a string reply onto the handler's
//!   `output_tx` channel.
//! - `load_compact_inputs` — read the model + flattened messages
//!   for a session in a single read lock.
//! - `run_manual_compact` / `finalize_auto_compact` — drive the
//!   two compaction entry points and apply their results.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::session_manager::SessionManager;
use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::session::ChatSession;
use closeclaw_llm::types::ContentBlock;
use closeclaw_llm::Message as ChatMessage;
use closeclaw_session::compaction::{execute_compact, CompactionResult, CompactionService};

fn flatten_content_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.as_str()),
            ContentBlock::Thinking { thinking: t, .. } => Some(t.as_str()),
            ContentBlock::ToolUse { input, .. } => Some(input.as_str()),
            ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
            _ => Some(""),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_compact_messages(messages: &[closeclaw_llm::session::SessionMessage]) -> Vec<ChatMessage> {
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
    let boundary = closeclaw_llm::session::SessionMessage {
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

/// Push a reply onto the handler's `output_tx` channel. The reply
/// carries an empty `content_blocks` list because compaction
/// feedback is plain text, not a structured LLM response.
pub(super) async fn send_output(
    output_tx: &Arc<RwLock<Option<mpsc::Sender<(String, Vec<ContentBlock>)>>>>,
    text: &str,
) {
    let guard = output_tx.read().await;
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send((text.to_string(), vec![])).await;
    }
}

/// Load compaction inputs: (model, llm_messages). Returns None if session not found.
pub(super) async fn load_compact_inputs(
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
pub(super) async fn run_manual_compact(
    sm: Arc<SessionManager>,
    fc: Arc<FallbackClient>,
    output_tx: Arc<RwLock<Option<mpsc::Sender<(String, Vec<ContentBlock>)>>>>,
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
pub(super) async fn finalize_auto_compact(
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
            svc.lock()
                .expect("compaction_service poisoned")
                .record_failure();
        }
    }
}
