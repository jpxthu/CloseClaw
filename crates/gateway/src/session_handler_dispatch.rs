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

use super::session_handler::{
    flatten_content_blocks, FallbackLlmCaller, MessageMetadata, SessionMessageHandler,
};
use super::Gateway;
use crate::session_manager::SessionManager;
use crate::HandleResult;
use closeclaw_common::im_plugin::IMPlugin;
use closeclaw_llm::session_state::LlmState;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::ChatSession;
use closeclaw_session::persistence::PendingMessage;

// ── Active-searcher trigger helpers ─────────────────────────────────

/// Shared dependencies for triggering the active-searcher background
/// task. Extracted so the trigger logic can be called from both the
/// pre-spawn path (user message) and inside the spawned task
/// (assistant message) without borrowing `&self` across `tokio::spawn`.
#[derive(Clone)]
struct SearcherTriggerDeps {
    session_manager: Arc<SessionManager>,
    unified_fallback_client: Arc<UnifiedFallbackClient>,
    memory_db_path: Option<std::path::PathBuf>,
}

type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'static>>;

/// Return type for the `get_agent_config` closure.
type AgentConfigResult = Result<(Option<String>, Option<serde_json::Value>), String>;

impl SearcherTriggerDeps {
    /// Trigger an active-searcher background task for the given message.
    ///
    /// Contains the same closure setup as
    /// `SessionMessageHandler::maybe_spawn_active_searcher` but takes
    /// dependencies explicitly so it can be called from within a
    /// `tokio::spawn` task.
    fn trigger(&self, session_id: &str, agent_id: &str, content: &str, message_role: &str) {
        use crate::memory::active_searcher_llm::should_trigger_role;
        use closeclaw_session::active_searcher::{ActiveSearcherRunner, SessionMessageSnapshot};

        if !should_trigger_role(agent_id) {
            return;
        }

        let sm = Arc::clone(&self.session_manager);
        let ufc = Arc::clone(&self.unified_fallback_client);

        // Closure: load agent config, serialize memory to JSON for
        // cross-crate decoupling.
        let get_agent_config = {
            let sm = Arc::clone(&sm);
            move |aid: String| -> BoxFuture<AgentConfigResult> {
                let sm = Arc::clone(&sm);
                Box::pin(async move {
                    match sm.get_agent_config(&aid).await {
                        Some(cfg) => {
                            let mem_json = cfg
                                .memory
                                .as_ref()
                                .and_then(|m| serde_json::to_value(m).ok());
                            Ok((cfg.model, mem_json))
                        }
                        None => Err("agent not found".to_string()),
                    }
                })
            }
        };

        // Closure: gather context messages as snapshots.
        let get_context_messages = {
            let sm = Arc::clone(&sm);
            let agent_id_for_ctx = agent_id.to_string();
            move |sid: String| -> BoxFuture<(Vec<SessionMessageSnapshot>, usize)> {
                let sm = Arc::clone(&sm);
                let aid = agent_id_for_ctx.clone();
                Box::pin(async move {
                    // Extract context_turns from memory config.
                    let ctx_turns = if let Some(cfg) = sm.get_agent_config(&aid).await {
                        let mem_json = cfg
                            .memory
                            .as_ref()
                            .and_then(|m| serde_json::to_value(m).ok());
                        closeclaw_session::active_searcher::extract_context_turns(&mem_json)
                    } else {
                        10
                    };

                    if let Some(cs) = sm.get_conversation_session(&sid).await {
                        let cs_read = cs.read().await;
                        let msgs = ChatSession::messages(&*cs_read);
                        let start = if msgs.len() > ctx_turns {
                            msgs.len() - ctx_turns
                        } else {
                            0
                        };
                        let snapshots: Vec<SessionMessageSnapshot> = msgs[start..]
                            .iter()
                            .map(|m| SessionMessageSnapshot {
                                role: m.role.clone(),
                                content: flatten_content_blocks(&m.content_blocks),
                            })
                            .collect();
                        (snapshots, ctx_turns)
                    } else {
                        (Vec::new(), ctx_turns)
                    }
                })
            }
        };

        // Closure: get already-injected event IDs for dedup.
        let get_injected_event_ids = {
            let sm = Arc::clone(&sm);
            move |sid: String| -> BoxFuture<std::collections::HashSet<i64>> {
                let sm = Arc::clone(&sm);
                Box::pin(async move {
                    if let Some(cs) = sm.get_conversation_session(&sid).await {
                        let cs_read = cs.read().await;
                        let slot = cs_read
                            .memory_injection_arc()
                            .lock()
                            .expect("memory_injection lock poisoned");
                        slot.as_ref()
                            .map(|inj| inj.injected_event_ids.clone())
                            .unwrap_or_default()
                    } else {
                        std::collections::HashSet::new()
                    }
                })
            }
        };

        // Closure: write memory injection into the session slot.
        let set_memory_injection = {
            let sm = Arc::clone(&sm);
            move |sid: String,
                  content: String,
                  position: String,
                  event_ids: Vec<i64>|
                  -> BoxFuture<()> {
                let sm = Arc::clone(&sm);
                Box::pin(async move {
                    if let Some(cs) = sm.get_conversation_session(&sid).await {
                        let pos_mode = match position.as_str() {
                            "before_next" => closeclaw_llm::session::InjectionPosition::BeforeNext,
                            _ => closeclaw_llm::session::InjectionPosition::AfterCurrent,
                        };
                        let injection = closeclaw_llm::session::MemoryInjection {
                            content,
                            position_mode: pos_mode,
                            injected_event_ids: event_ids.into_iter().collect(),
                        };
                        cs.read().await.set_memory_injection(injection);
                    }
                })
            }
        };

        // Closure: execute the active-searcher pipeline.
        let run_searcher = {
            let ufc = Arc::clone(&ufc);
            move |db_path: String,
                  agent_id: String,
                  role: String,
                  content: String,
                  model: String,
                  context_messages: Vec<SessionMessageSnapshot>,
                  injected_ids: std::collections::HashSet<i64>,
                  memory_config: serde_json::Value|
                  -> BoxFuture<Option<(String, String, Vec<i64>)>> {
                let ufc = Arc::clone(&ufc);
                Box::pin(async move {
                    use crate::memory::active_searcher::{ActiveSearcher, ActiveSearcherConfig};

                    let llm_messages: Vec<closeclaw_llm::session::SessionMessage> =
                        context_messages
                            .into_iter()
                            .map(|m| closeclaw_llm::session::SessionMessage {
                                role: m.role,
                                content_blocks: vec![closeclaw_llm::types::ContentBlock::Text(
                                    m.content,
                                )],
                                timestamp: chrono::Utc::now(),
                            })
                            .collect();

                    let mem_cfg: Option<closeclaw_common::agent_config::MemoryConfig> =
                        serde_json::from_value(memory_config).ok();

                    let config =
                        ActiveSearcherConfig::from_agent_config(Some(&model), mem_cfg.as_ref());
                    let searcher =
                        ActiveSearcher::new(std::path::PathBuf::from(&db_path), config.clone());
                    let caller = FallbackLlmCaller {
                        client: ufc,
                        model: config.model.clone(),
                    };

                    if let Some(injection) = searcher
                        .run(
                            &agent_id,
                            &role,
                            &content,
                            &llm_messages,
                            &injected_ids,
                            &caller,
                        )
                        .await
                    {
                        let pos_str = match injection.position_mode {
                            closeclaw_llm::session::InjectionPosition::BeforeNext => {
                                "before_next".to_string()
                            }
                            closeclaw_llm::session::InjectionPosition::AfterCurrent => {
                                "after_current".to_string()
                            }
                        };
                        let event_ids: Vec<i64> =
                            injection.injected_event_ids.iter().copied().collect();
                        Some((injection.content, pos_str, event_ids))
                    } else {
                        None
                    }
                })
            }
        };

        let deps = closeclaw_session::active_searcher::SearcherDependencies {
            get_agent_config: Box::new(get_agent_config),
            get_context_messages: Box::new(get_context_messages),
            get_injected_event_ids: Box::new(get_injected_event_ids),
            set_memory_injection: Box::new(set_memory_injection),
            run_searcher: Box::new(run_searcher),
        };

        ActiveSearcherRunner::trigger(
            session_id,
            agent_id,
            content,
            message_role,
            &self.memory_db_path,
            deps,
        );
    }
}

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

        // ── Trigger active-searcher (best-effort, non-blocking) ────
        // Look up the agent_id for role-based exclusion, then spawn a
        // background searcher that writes results to the
        // memory_injection slot for the next turn to consume.
        let searcher_deps = SearcherTriggerDeps {
            session_manager: Arc::clone(&self.session_manager),
            unified_fallback_client: Arc::clone(&self.unified_fallback_client),
            memory_db_path: self.memory_db_path.clone(),
        };
        if let Some(agent_id) = self.session_manager.get_chat_id(session_id).await {
            searcher_deps.trigger(session_id, &agent_id, &content, "user");
        }

        let session_id = session_id.to_string();
        let content_for_task = content;
        let sm = Arc::clone(&self.session_manager);
        let ufc = Arc::clone(&self.unified_fallback_client);
        let output_tx = Arc::clone(&self.output_tx);
        let channel = meta.channel.clone();
        // Clone the gateway/plugin into the spawn (optional). If
        // streaming is enabled for the session but no gateway/plugin
        // is provided we fall back to the non-streaming path.
        let gw_for_task = gateway.map(Arc::clone);
        let plugin_for_task = plugin.map(Arc::clone);
        // Clone the shutdown handle for busy-count tracking inside
        // the spawned task. The handle is optional (tests may not
        // set one).
        let shutdown_handle = self.shutdown_handle.clone();
        let searcher_deps = searcher_deps;

        tokio::spawn(async move {
            // Increment busy count at message dequeue.
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
                        "streaming enabled but no gateway/plugin; \
                         falling back to non-streaming"
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

            // Extract assistant response text for active-searcher
            // trigger (BeforeNext mode for assistant messages).
            let assistant_text = match &result {
                Ok(sr) => sr
                    .content_blocks
                    .iter()
                    .filter_map(|b| match b {
                        closeclaw_llm::types::ContentBlock::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
                Err(_) => String::new(),
            };

            Self::finish_llm(&sm, &session_id, result, &ufc, &output_tx).await;

            // ── Trigger active-searcher for assistant message ───
            // After the assistant response is stored in the session,
            // trigger active-searcher with role "assistant" so it
            // writes to the BeforeNext slot for the next user turn.
            if let Some(agent_id) = sm.get_chat_id(&session_id).await {
                searcher_deps.trigger(&session_id, &agent_id, &assistant_text, "assistant");
            }

            // Decrement busy count after response sent + pending
            // drained.
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
            tracing::warn!(
                session_id,
                error = %e,
                "failed to enqueue pending message"
            );
        }
    }
}
