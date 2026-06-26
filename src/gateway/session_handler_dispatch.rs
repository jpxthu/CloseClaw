//! LLM dispatch helpers for `SessionMessageHandler`.
//!
//! Extracted from `session_handler.rs` to keep the file under the
//! 500-line project limit. This module hosts the streaming-aware
//! dispatch path: [`SessionMessageHandler::handle_message_with_gateway`]
//! and [`SessionMessageHandler::dispatch_llm_call`], which route a
//! spawned LLM call to either the streaming pipeline (via
//! [`Gateway::send_outbound_streaming`]) or the non-streaming
//! fallback ([`SessionMessageHandler::call_llm`]).

use std::sync::Arc;

use super::session_handler::{MessageMetadata, SessionMessageHandler};
use super::Gateway;
use crate::gateway::HandleResult;
use crate::im::IMPlugin;
use crate::llm::session_state::LlmState;
use crate::llm::ChatSession;
use crate::session::persistence::PendingMessage;

impl SessionMessageHandler {
    /// Streaming-aware entry point used by [`Gateway::handle_inbound_message`].
    ///
    /// Same as [`handle_message_with_meta`](Self::handle_message_with_meta) but
    /// passes the [`Gateway`] reference and [`IMPlugin`] to the dispatch
    /// task so streaming LLM output can be routed through
    /// [`Gateway::send_outbound_streaming`].
    pub async fn handle_message_with_gateway(
        &self,
        session_id: &str,
        content: String,
        meta: MessageMetadata,
        gateway: &Arc<Gateway>,
        plugin: &Arc<dyn IMPlugin>,
    ) -> HandleResult {
        if self.session_manager.is_session_busy(session_id).await {
            self.enqueue_pending(session_id, content).await;
            return HandleResult::MessageQueued;
        }
        self.check_and_run_auto_compact(session_id).await;
        self.dispatch_llm_call(session_id, content, meta, Some(gateway), Some(plugin))
            .await
    }

    /// Dispatch an LLM call inside a `tokio::spawn` task.
    ///
    /// When both `gateway` and `plugin` are provided AND the session has
    /// `stream_enabled = true`, the streaming pipeline is used
    /// ([`Self::call_llm_streaming`]). Otherwise the non-streaming
    /// pipeline is used ([`Self::call_llm`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch_llm_call(
        &self,
        session_id: &str,
        content: String,
        meta: MessageMetadata,
        gateway: Option<&Arc<Gateway>>,
        plugin: Option<&Arc<dyn IMPlugin>>,
    ) -> HandleResult {
        self.set_busy(session_id, true).await;

        // ── Trigger active-searcher (best-effort, non-blocking) ────────
        // Look up the agent_id for role-based exclusion, then spawn a
        // background searcher that writes results to the memory_injection
        // slot for the next turn to consume.
        if let Some(agent_id) = self.session_manager.get_chat_id(session_id).await {
            self.maybe_spawn_active_searcher(session_id, &agent_id, &content, "user");
        }

        let session_id = session_id.to_string();
        let content_for_task = content;
        let sm = Arc::clone(&self.session_manager);
        let ufc = Arc::clone(&self.unified_fallback_client);
        let output_tx = Arc::clone(&self.output_tx);
        let channel = meta.channel.clone();
        // Clone the gateway/plugin into the spawn (optional). If streaming
        // is enabled for the session but no gateway/plugin is provided we
        // fall back to the non-streaming path with a warning.
        let gw_for_task = gateway.map(Arc::clone);
        let plugin_for_task = plugin.map(Arc::clone);
        // Clone the shutdown handle for busy-count tracking inside the
        // spawned task. The handle is optional (tests may not set one).
        let shutdown_handle = self.shutdown_handle.clone();

        tokio::spawn(async move {
            // Increment busy count at message dequeue (start of processing).
            if let Some(ref h) = shutdown_handle {
                h.increment_busy();
            }

            // Check if streaming is enabled for this session
            let stream_enabled = if let Some(cs) = sm.get_conversation_session(&session_id).await {
                cs.read().await.stream_enabled()
            } else {
                false
            };

            let result = if stream_enabled {
                if let (Some(gw), Some(pl)) = (gw_for_task.as_ref(), plugin_for_task.as_ref()) {
                    Self::call_llm_streaming(
                        ufc.primary(),
                        &content_for_task,
                        &meta,
                        &sm,
                        &session_id,
                        &channel,
                        gw,
                        pl,
                    )
                    .await
                } else {
                    tracing::warn!(
                        session_id = %session_id,
                        "streaming enabled but no gateway/plugin available; falling back to non-streaming"
                    );
                    Self::call_llm(&ufc, &content_for_task, &meta, &sm, &session_id)
                        .await
                        .map(Into::into)
                }
            } else {
                Self::call_llm(&ufc, &content_for_task, &meta, &sm, &session_id)
                    .await
                    .map(Into::into)
            };
            Self::finish_llm(&sm, &session_id, result, &ufc, &output_tx).await;

            // Decrement busy count after response sent + pending drained.
            if let Some(ref h) = shutdown_handle {
                h.decrement_busy();
            }
        });

        HandleResult::LlmStarted
    }

    pub(super) async fn set_busy(&self, session_id: &str, busy: bool) {
        if let Some(cs) = self
            .session_manager
            .get_conversation_session(session_id)
            .await
        {
            let cs = cs.write().await;
            cs.set_llm_busy(busy);
            if busy {
                cs.set_llm_state(LlmState::Requesting);
            }
        }
    }

    pub(super) async fn enqueue_pending(&self, session_id: &str, content: String) {
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
