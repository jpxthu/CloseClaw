//! Convenience compaction method for [`SessionManager`].
//!
//! Provides `SessionManager::compact(session_id, instruction, chat_fn)`
//! which implements the design-doc call chain:
//!   Gateway → session.compact(instruction)
//!
//! Also contains shared compaction helpers (`flatten_content_blocks`,
//! `build_compact_messages`, `load_compact_inputs`) used by
//! `session_handler.rs` and `session_handler_dispatch.rs`.

use closeclaw_common::RunningStats;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::compaction::{
    ChatFn, CompactParams, CompactionMessage, CompactionResult, CompactionService,
};
use closeclaw_session::llm_session::ChatSession;
use tracing::warn;

use super::SessionManager;

/// Pre-loaded compaction inputs to avoid redundant session reads.
///
/// When the caller (e.g. `check_and_run_auto_compact`) has already
/// loaded session data for threshold estimation, passing it here
/// avoids a second redundant `load_compact_inputs` call.
pub(crate) struct PreloadedCompactInputs {
    pub model: String,
    pub llm_messages: Vec<closeclaw_llm::Message>,
    pub stats: RunningStats,
}

impl SessionManager {
    /// Execute a compaction for the given session.
    ///
    /// Loads session messages, calls [`CompactionService::compact`],
    /// applies the result to the session transcript, and persists the
    /// checkpoint. This implements the design-doc call chain:
    /// `Gateway → session.compact(instruction)`.
    ///
    /// If `preloaded` is provided, it is used instead of calling
    /// `load_compact_inputs` again — avoids redundant session reads
    /// when the caller has already loaded the data (e.g. for
    /// threshold estimation).
    pub(crate) async fn compact(
        &self,
        session_id: &str,
        instruction: Option<&str>,
        is_auto: bool,
        compaction_service: &mut CompactionService,
        chat_fn: &ChatFn,
        preloaded: Option<PreloadedCompactInputs>,
    ) -> Result<CompactionResult, closeclaw_session::compaction::CompactionError> {
        let (model, llm_messages, stats) = match preloaded {
            Some(p) => (p.model, p.llm_messages, p.stats),
            None => load_compact_inputs(self, session_id).await.ok_or_else(|| {
                closeclaw_session::compaction::CompactionError::SessionNotFound(
                    session_id.to_string(),
                )
            })?,
        };

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
            .compact(CompactParams {
                messages: &compaction_msgs,
                model: &model,
                instruction,
                is_auto,
                stats: Some(&stats),
                chat_fn,
            })
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
pub(crate) async fn load_compact_inputs(
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
pub(crate) fn build_compact_messages(
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
