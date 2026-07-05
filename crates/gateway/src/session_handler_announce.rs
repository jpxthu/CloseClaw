//! Announce integration and LLM-finishing helpers for SessionMessageHandler.
//!
//! Extracted from `session_handler.rs` to keep the file under the
//! 500-line project limit. This module hosts two closely related
//! concerns that share the same call sites:
//!
//! 1. **Announce integration** (Step 1.5) — `maybe_push_announce` and
//!    `drain_announce_events` wrap the two `SessionManager` methods
//!    that let run-mode child sessions notify their parent and let
//!    parents drain queued announces before processing the next
//!    pending user message.
//! 2. **LLM finishing** — `finish_llm`, `clear_busy_and_send`, and
//!    `drain_pending_loop` are the post-LLM completion pipeline that
//!    clears the busy flag, surfaces the response, pushes the
//!    announce, and processes any queued pending messages. They were
//!    co-located with the announce calls and grew large enough that
//!    moving them together was the natural fix for the line-count
//!    constraint.

use std::sync::Arc;

use super::session_handler::{MessageMetadata, SessionMessageHandler};
use super::OutputTx;
use crate::outbound::StreamResult;
use crate::session_manager::SessionManager;
use closeclaw_llm::session::ChatSession;
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::types::ContentBlock;

impl SessionMessageHandler {
    /// Clear busy flag, send output, and drain pending messages.
    ///
    /// Accepts a [`StreamResult`] (returned by the streaming LLM call) or an
    /// `LLMError`. The non-streaming `call_llm` path converts its
    /// `UnifiedResponse` into a `StreamResult` via [`StreamResult::from`]
    /// before calling this entry point.
    pub(super) async fn finish_llm(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<StreamResult, closeclaw_llm::LLMError>,
        llm_caller: &Arc<dyn closeclaw_common::LlmCaller>,
        output_tx: &OutputTx,
    ) {
        Self::clear_busy_and_send(session_manager, session_id, result, output_tx).await;
        Self::drain_pending_loop(session_manager, session_id, llm_caller, output_tx).await;

        // NOTE: Decrement is handled by the caller (spawned task in
        // `session_handler_dispatch.rs`), NOT here. This avoids a
        // double-decrement when both `finish_llm` and the spawned task
        // call `decrement_busy()`.

        // NOTE: Cascade-termination of child sessions is NOT done here.
        // `finish_llm` is called after every LLM turn — cascading here
        // would prematurely kill session-mode children that are designed
        // to survive across turns. Cascade kill is handled by:
        // - The sweeper (idle→archive path) for normal parent session end
        // - `sessions_kill` tool for explicit parent-initiated kills
        // - `ArchiveSweeper::cascade_archive_impl` for timeout cleanup
        // See design-doc §生命周期联动 for the two correct trigger points.
    }

    async fn clear_busy_and_send(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<StreamResult, closeclaw_llm::LLMError>,
        output_tx: &OutputTx,
    ) {
        if let Some(cs) = session_manager.get_conversation_session(session_id).await {
            let cs = cs.write().await;
            cs.set_llm_busy(false);
            cs.set_llm_state(LlmState::Idle);
        }
        match result {
            Ok(stream_result) => {
                // Append response to session message history. `append_response`
                // takes a `UnifiedResponse`; convert via the existing
                // `From<StreamResult> for UnifiedResponse` impl.
                let unified: closeclaw_llm::types::UnifiedResponse = stream_result.clone().into();
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    let mut cs_write = cs.write().await;
                    cs_write.append_response(unified);
                    // Cache break detection (must run before accumulate_usage
                    // so that last_cache_read_tokens still holds the previous value).
                    if let Some(info) =
                        cs_write.detect_cache_break_for_usage(stream_result.usage.cache_read_tokens)
                    {
                        tracing::warn!(
                            session_id,
                            previous = info.previous_cache_read,
                            current = info.current_cache_read,
                            drop = info.drop_tokens,
                            ratio = info.drop_ratio,
                            "Cache break detected"
                        );
                    }
                    cs_write.accumulate_usage(&stream_result.usage);
                }
                let text = stream_result
                    .content_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let guard = output_tx.read().await;
                if let Some(tx) = guard.as_ref() {
                    let _ = tx.send((text, stream_result.content_blocks)).await;
                }
            }
            Err(err) => {
                tracing::warn!(session_id, error = %err, "LLM call failed");
            }
        }
        // Step 1.5: best-effort announce to parent (run-mode child).
        Self::maybe_push_announce(session_manager, session_id).await;
    }

    async fn drain_pending_loop(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        llm_caller: &Arc<dyn closeclaw_common::LlmCaller>,
        output_tx: &OutputTx,
    ) {
        // Step 1.5: drain queued announces.
        Self::drain_announce_events(session_manager, session_id).await;
        loop {
            // Get next pending message
            let Some(pending) = session_manager.pop_pending_message(session_id).await else {
                break;
            };

            // Set busy before calling LLM
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                let cs = cs.write().await;
                cs.set_llm_busy(true);
                cs.set_llm_state(LlmState::Requesting);
            }

            let meta = MessageMetadata::default_meta();
            // Non-streaming path: `call_llm` returns `UnifiedResponse`.
            // Convert to `StreamResult` for the unified `finish_llm` entry point.
            let result: Result<StreamResult, closeclaw_llm::LLMError> = Self::call_llm(
                llm_caller,
                &pending.content,
                &meta,
                session_manager,
                session_id,
            )
            .await
            .map(Into::into);
            Self::clear_busy_and_send(session_manager, session_id, result, output_tx).await;
        }
    }

    /// Step 1.5: best-effort announce to parent (run-mode child).
    ///
    /// Invoked at the end of `clear_busy_and_send` so a finished
    /// run-mode child session can notify its parent that new output
    /// is available. Wraps `SessionManager::try_push_announce`.
    pub(super) async fn maybe_push_announce(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
    ) {
        session_manager.try_push_announce(session_id).await;
    }

    /// Step 1.5: drain queued announces before processing the next
    /// pending message.
    ///
    /// Invoked at the start of `drain_pending_loop` so any announces
    /// queued during the previous LLM turn are injected into the
    /// session before the next pending user message is processed.
    /// Wraps `SessionManager::drain_and_inject_announces`.
    pub(super) async fn drain_announce_events(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
    ) {
        session_manager.drain_and_inject_announces(session_id).await;
    }
}
