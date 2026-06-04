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
use tokio::sync::{mpsc, RwLock};

use super::session_handler::{MessageMetadata, SessionMessageHandler};
use crate::gateway::outbound::StreamResult;
use crate::gateway::session_manager::SessionManager;
use crate::llm::fallback::FallbackClient;
use crate::llm::session::ChatSession;
use crate::llm::session_state::LlmState;
use crate::llm::types::ContentBlock;

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
        result: Result<StreamResult, crate::llm::LLMError>,
        fallback_client: &Arc<FallbackClient>,
        output_tx: &Arc<RwLock<Option<mpsc::Sender<(String, Vec<ContentBlock>)>>>>,
    ) {
        Self::clear_busy_and_send(session_manager, session_id, result, output_tx).await;
        Self::drain_pending_loop(session_manager, session_id, fallback_client, output_tx).await;
    }

    async fn clear_busy_and_send(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        result: Result<StreamResult, crate::llm::LLMError>,
        output_tx: &Arc<RwLock<Option<mpsc::Sender<(String, Vec<ContentBlock>)>>>>,
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
                let unified: crate::llm::types::UnifiedResponse = stream_result.clone().into();
                if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                    let mut cs_write = cs.write().await;
                    cs_write.append_response(unified);
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
        Self::maybe_push_announce(session_manager, session_id).await; // Step 1.5: best-effort announce to parent (run-mode child).
    }

    async fn drain_pending_loop(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        fallback_client: &Arc<FallbackClient>,
        output_tx: &Arc<RwLock<Option<mpsc::Sender<(String, Vec<ContentBlock>)>>>>,
    ) {
        Self::drain_announce_events(session_manager, session_id).await; // Step 1.5: drain queued announces.
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
            let result: Result<StreamResult, crate::llm::LLMError> = Self::call_llm(
                fallback_client,
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
