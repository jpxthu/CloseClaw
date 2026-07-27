//! Convenience compaction method for [`SessionManager`].
//!
//! Provides `SessionManager::compact(session_id, instruction, chat_fn)`
//! which implements the design-doc call chain:
//!   Gateway → session.compact(instruction)
//!
//! Extracted from `session_manager.rs` to stay under the 1000-line
//! file limit.

use closeclaw_common::RunningStats;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::compaction::{
    ChatFn, CompactionMessage, CompactionResult, CompactionService,
};
use closeclaw_session::llm_session::ChatSession;
use tracing::warn;

use super::SessionManager;

impl SessionManager {
    /// Execute a compaction for the given session.
    ///
    /// Loads session messages, calls [`CompactionService::compact`],
    /// applies the result to the session transcript, and persists the
    /// checkpoint. This implements the design-doc call chain:
    /// `Gateway → session.compact(instruction)`.
    ///
    /// # Arguments
    ///
    /// * `session_id` - Target session to compact.
    /// * `instruction` - Optional custom retention instruction.
    /// * `compaction_service` - Mutable reference to the compaction
    ///   service (circuit breaker state is updated on success/failure).
    /// * `chat_fn` - Async closure for LLM calls, injected to avoid
    ///   depending on the `llm` crate directly.
    ///
    /// # Returns
    ///
    /// `Ok(CompactionResult)` on success, or `Err(CompactionError)`.
    pub async fn compact(
        &self,
        session_id: &str,
        instruction: Option<&str>,
        is_auto: bool,
        compaction_service: &mut CompactionService,
        chat_fn: &ChatFn,
    ) -> Result<CompactionResult, closeclaw_session::compaction::CompactionError> {
        let (model, llm_messages, stats) =
            load_compact_inputs(self, session_id).await.ok_or_else(|| {
                closeclaw_session::compaction::CompactionError::LLMCallFailed(
                    "session not found".to_string(),
                )
            })?;

        if llm_messages.is_empty() {
            return Err(closeclaw_session::compaction::CompactionError::EmptyMessages);
        }

        let compaction_msgs: Vec<CompactionMessage> = llm_messages
            .iter()
            .map(|m| CompactionMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let snapshot_id = self.save_pre_compaction_snapshot(session_id).await;

        let result = compaction_service
            .compact(
                &compaction_msgs,
                &model,
                instruction,
                is_auto,
                Some(&stats),
                chat_fn,
            )
            .await;

        match result {
            Ok(r) => {
                crate::session_handler::apply_compact_result(
                    self,
                    session_id,
                    &r,
                    snapshot_id.as_deref(),
                )
                .await;
                Ok(r)
            }
            Err(e) => {
                warn!(
                    session_id = %session_id,
                    error = %e,
                    "SessionManager::compact failed"
                );
                self.rollback_compaction(session_id).await;
                Err(e)
            }
        }
    }
}

/// Load compaction inputs from the session.
///
/// Returns `(model, llm_messages, stats)` or `None` if the session
/// is not found.
async fn load_compact_inputs(
    sm: &SessionManager,
    session_id: &str,
) -> Option<(String, Vec<closeclaw_llm::Message>, RunningStats)> {
    let cs = sm.get_conversation_session(session_id).await?;
    let cs_read = cs.read().await;
    let model = cs_read.model().to_string();
    let llm_msgs = build_compact_messages(ChatSession::messages(&*cs_read));
    let stats = cs_read.stats().clone();
    Some((model, llm_msgs, stats))
}

/// Build compact-friendly messages from session messages.
///
/// Filters to user/assistant roles and flattens content blocks.
fn build_compact_messages(
    messages: &[closeclaw_session::llm_session::SessionMessage],
) -> Vec<closeclaw_llm::Message> {
    messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .map(|m| closeclaw_llm::Message {
            role: m.role.clone(),
            content: flatten_content_blocks(&m.content_blocks),
        })
        .collect()
}

/// Flatten content blocks into a single string.
fn flatten_content_blocks(blocks: &[ContentBlock]) -> String {
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
